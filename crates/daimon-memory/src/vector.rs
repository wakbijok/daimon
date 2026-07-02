//! Vector tier — Qdrant-backed embedding store.
//!
//! Thin wrapper around `qdrant_client::Qdrant` exposing the subset of operations
//! daimon needs: ensure a collection exists, upsert points, search nearest neighbours.
//! Single-org: there is one fixed long-term memory collection ([`COLLECTION`]);
//! callers pass point ids + payloads, no tenant scoping.

use std::collections::HashMap;

/// The single fixed Qdrant collection for daimon's long-term memory.
/// (Single-org: no per-tenant collection naming.)
pub const COLLECTION: &str = "daimon_memory_long_term";

use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, Fusion, NamedVectors, PointStruct, PrefetchQueryBuilder,
    Query, QueryPointsBuilder, ScoredPoint, SearchPointsBuilder, SparseIndexConfigBuilder,
    SparseVectorConfig, SparseVectorParams, UpsertPointsBuilder, Vector, VectorInput,
    VectorParamsBuilder, VectorsConfig, vectors_config,
};
use serde_json::Value as Json;

use crate::error::Result;

/// A point to upsert into a collection.
#[derive(Debug, Clone)]
pub struct Point {
    pub id: u64,
    pub vector: Vec<f32>,
    pub payload: Json,
}

/// A hybrid point — dense + sparse vectors keyed by named-vector slots
/// (`dense` and `sparse`).
#[derive(Debug, Clone)]
pub struct HybridPoint {
    pub id: u64,
    pub dense: Vec<f32>,
    pub sparse_indices: Vec<u32>,
    pub sparse_values: Vec<f32>,
    pub payload: Json,
}

/// A scored hit returned by [`VectorStore::search`].
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: u64,
    pub score: f32,
    pub payload: Json,
}

/// Thin Qdrant client wrapper scoped to daimon's vector tier needs.
pub struct VectorStore {
    client: Qdrant,
}

impl VectorStore {
    /// Connect to a Qdrant instance at the given URL (e.g. `"http://localhost:6334"` for gRPC).
    pub fn connect(url: &str) -> Result<Self> {
        let client = Qdrant::from_url(url).build()?;
        Ok(Self { client })
    }

    /// Ensure a collection exists with the given name and dimension. Idempotent.
    /// Uses cosine distance by default.
    pub async fn ensure_collection(&self, name: &str, dim: u64) -> Result<()> {
        if self.client.collection_exists(name).await? {
            return Ok(());
        }
        let req = CreateCollectionBuilder::new(name)
            .vectors_config(VectorParamsBuilder::new(dim, Distance::Cosine));
        self.client.create_collection(req).await?;
        Ok(())
    }

    /// Upsert a batch of points into the collection. Caller's responsibility to choose
    /// stable IDs.
    pub async fn upsert(&self, collection: &str, points: Vec<Point>) -> Result<()> {
        let qdrant_points: Vec<PointStruct> = points
            .into_iter()
            .map(|p| {
                let payload = json_to_payload(p.payload);
                PointStruct::new(p.id, p.vector, payload)
            })
            .collect();
        let req = UpsertPointsBuilder::new(collection, qdrant_points).wait(true);
        self.client.upsert_points(req).await?;
        Ok(())
    }

    /// Search the collection for the `top_k` nearest vectors to `query`.
    pub async fn search(&self, collection: &str, query: Vec<f32>, top_k: u64) -> Result<Vec<Hit>> {
        let req = SearchPointsBuilder::new(collection, query, top_k).with_payload(true);
        let resp = self.client.search_points(req).await?;
        let hits = resp
            .result
            .into_iter()
            .map(scored_point_to_hit)
            .collect();
        Ok(hits)
    }

    /// Drop a collection. Used by tests for cleanup.
    pub async fn drop_collection(&self, name: &str) -> Result<()> {
        self.client.delete_collection(name).await?;
        Ok(())
    }

    /// Ensure a hybrid collection exists with the given name and dense
    /// dimension. Hybrid = named vectors `dense` (cosine) + `sparse`. If the
    /// collection already exists, no-op.
    pub async fn ensure_hybrid_collection(&self, name: &str, dense_dim: u64) -> Result<()> {
        if self.client.collection_exists(name).await? {
            return Ok(());
        }
        let mut named_dense = HashMap::new();
        named_dense.insert(
            "dense".to_string(),
            VectorParamsBuilder::new(dense_dim, Distance::Cosine).build(),
        );
        let vectors_config = VectorsConfig {
            config: Some(vectors_config::Config::ParamsMap(
                qdrant_client::qdrant::VectorParamsMap { map: named_dense },
            )),
        };

        let mut sparse_map = HashMap::new();
        sparse_map.insert(
            "sparse".to_string(),
            SparseVectorParams {
                index: Some(SparseIndexConfigBuilder::default().build()),
                modifier: None,
            },
        );
        let sparse_config = SparseVectorConfig { map: sparse_map };

        let mut req = CreateCollectionBuilder::new(name).sparse_vectors_config(sparse_config);
        req = req.vectors_config(vectors_config);

        self.client.create_collection(req).await?;
        Ok(())
    }

    /// Upsert hybrid points (each carries dense + sparse vectors).
    pub async fn upsert_hybrid(&self, collection: &str, points: Vec<HybridPoint>) -> Result<()> {
        let qdrant_points: Vec<PointStruct> = points
            .into_iter()
            .map(|p| {
                let mut vectors = HashMap::new();
                vectors.insert(
                    "dense".to_string(),
                    Vector {
                        data: p.dense,
                        indices: None,
                        vector: None,
                        vectors_count: None,
                    },
                );
                vectors.insert(
                    "sparse".to_string(),
                    Vector {
                        data: p.sparse_values,
                        indices: Some(qdrant_client::qdrant::SparseIndices {
                            data: p.sparse_indices,
                        }),
                        vector: None,
                        vectors_count: None,
                    },
                );
                let named = NamedVectors { vectors };
                let payload = json_to_payload(p.payload);
                PointStruct::new(p.id, named, payload)
            })
            .collect();
        let req = UpsertPointsBuilder::new(collection, qdrant_points).wait(true);
        self.client.upsert_points(req).await?;
        Ok(())
    }

    /// Hybrid query — prefetch dense + sparse, fuse with RRF. Returns the
    /// fused top_k.
    pub async fn query_hybrid(
        &self,
        collection: &str,
        dense_q: Vec<f32>,
        sparse_indices: Vec<u32>,
        sparse_values: Vec<f32>,
        top_k: u64,
    ) -> Result<Vec<Hit>> {
        let prefetch_limit = (top_k * 4).max(25);
        let dense_prefetch = PrefetchQueryBuilder::default()
            .query(Query::new_nearest(VectorInput::from(dense_q)))
            .using("dense")
            .limit(prefetch_limit)
            .build();
        let sparse_prefetch = PrefetchQueryBuilder::default()
            .query(Query::new_nearest(VectorInput::new_sparse(
                sparse_indices,
                sparse_values,
            )))
            .using("sparse")
            .limit(prefetch_limit)
            .build();

        let req = QueryPointsBuilder::new(collection.to_string())
            .add_prefetch(dense_prefetch)
            .add_prefetch(sparse_prefetch)
            .query(Query::new_fusion(Fusion::Rrf))
            .limit(top_k)
            .with_payload(true)
            .build();

        let resp = self.client.query(req).await?;
        Ok(resp.result.into_iter().map(scored_point_to_hit).collect())
    }

    /// Delete a set of points by their numeric ids. Returns Ok even if some
    /// ids didn't exist — Qdrant's behaviour is best-effort delete-by-id.
    pub async fn delete_points(&self, collection: &str, ids: &[u64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        use qdrant_client::qdrant::{DeletePointsBuilder, PointsIdsList, PointId};
        let points = PointsIdsList {
            ids: ids
                .iter()
                .map(|&id| PointId {
                    point_id_options: Some(
                        qdrant_client::qdrant::point_id::PointIdOptions::Num(id),
                    ),
                })
                .collect(),
        };
        self.client
            .delete_points(DeletePointsBuilder::new(collection).points(points).wait(true))
            .await?;
        Ok(())
    }
}

fn json_to_payload(value: Json) -> qdrant_client::Payload {
    qdrant_client::Payload::try_from(value).unwrap_or_default()
}

fn scored_point_to_hit(p: ScoredPoint) -> Hit {
    let id = match p.id {
        Some(point_id) => match point_id.point_id_options {
            Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(n)) => n,
            _ => 0,
        },
        None => 0,
    };
    let payload_json = serde_json::to_value(&p.payload).unwrap_or(Json::Null);
    Hit {
        id,
        score: p.score,
        payload: payload_json,
    }
}

#[cfg(test)]
mod tests {
    // Real Qdrant required for these tests — see tests/vector_store.rs for the integration test.
    // Unit tests here cover only payload conversion helpers.

    use super::*;

    #[test]
    fn payload_roundtrip_object() {
        let value = serde_json::json!({ "text": "hello", "n": 42 });
        let payload = json_to_payload(value.clone());
        let back = serde_json::to_value(&payload).unwrap();
        assert_eq!(back, value);
    }
}
