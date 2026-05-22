//! Vector tier — Qdrant-backed embedding store.
//!
//! Thin wrapper around `qdrant_client::Qdrant` exposing the subset of operations
//! daimon needs: ensure a collection exists, upsert points, search nearest neighbours.
//! Multi-tenant isolation is enforced at the caller level by collection naming
//! (`tenant_<id>_<purpose>`) and payload filtering.

use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, ScoredPoint, SearchPointsBuilder,
    UpsertPointsBuilder, VectorParamsBuilder,
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
    /// stable IDs and to encode tenant_id in payload + collection name.
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
