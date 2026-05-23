//! Greedy-MMR context packer — fits ranked hits into a token budget.
//!
//! Phase 3 D6c. Given a list of retrieved chunks ordered by relevance score,
//! pick a subset that:
//!   * stays within `max_tokens` budget (sum of `token_estimate` ≤ budget)
//!   * favours diversity via Maximal Marginal Relevance: balance original
//!     score against pairwise similarity to already-picked items
//!
//! Similarity here is a cheap Jaccard over whitespace-token sets. For higher
//! fidelity, swap to embedding cosine via the dense vector — but Jaccard is
//! fast enough for typical top-25 → pick-5-7 packing.

use std::collections::HashSet;

use crate::retrieve::RetrievedChunk;

/// Packer configuration.
#[derive(Debug, Clone, Copy)]
pub struct PackConfig {
    /// Token budget — sum of `token_estimate` across picks ≤ this.
    pub max_tokens: i32,
    /// MMR diversity weight in [0.0, 1.0]. 0.0 = pure relevance, 1.0 = pure
    /// diversity. Default 0.3 favours relevance with mild diversity.
    pub diversity_lambda: f32,
}

impl Default for PackConfig {
    fn default() -> Self {
        Self {
            max_tokens: 3000,
            diversity_lambda: 0.3,
        }
    }
}

/// A packed context item — same shape as the input chunk plus the pick order.
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub chunk: RetrievedChunk,
    /// 0-based index in the packed output.
    pub pick_order: usize,
}

/// Pack ranked `hits` into a token-budgeted context.
pub fn pack_context(hits: &[RetrievedChunk], cfg: &PackConfig) -> Vec<ContextItem> {
    if hits.is_empty() || cfg.max_tokens <= 0 {
        return Vec::new();
    }

    // Pre-tokenise once for Jaccard.
    let token_sets: Vec<HashSet<String>> = hits.iter().map(|h| tokenize(&h.content)).collect();

    let mut picked: Vec<usize> = Vec::new();
    let mut remaining: Vec<usize> = (0..hits.len()).collect();
    let mut tokens_used: i32 = 0;

    while !remaining.is_empty() {
        let mut best: Option<(usize, f32)> = None; // (idx-in-remaining, mmr-score)
        for (ri, &cand) in remaining.iter().enumerate() {
            // Skip if it would blow the budget.
            if tokens_used + hits[cand].token_estimate > cfg.max_tokens {
                continue;
            }
            // Diversity penalty = max Jaccard against picked set.
            let max_sim = picked
                .iter()
                .map(|&p| jaccard(&token_sets[cand], &token_sets[p]))
                .fold(0.0f32, f32::max);
            let mmr = (1.0 - cfg.diversity_lambda) * hits[cand].score
                - cfg.diversity_lambda * max_sim;
            if best.map_or(true, |(_, bs)| mmr > bs) {
                best = Some((ri, mmr));
            }
        }
        let Some((ri, _)) = best else { break };
        let cand = remaining.remove(ri);
        tokens_used += hits[cand].token_estimate;
        picked.push(cand);
    }

    picked
        .into_iter()
        .enumerate()
        .map(|(pick_order, idx)| ContextItem {
            chunk: hits[idx].clone(),
            pick_order,
        })
        .collect()
}

fn tokenize(s: &str) -> HashSet<String> {
    s.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersect = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 { 0.0 } else { intersect / union }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn chunk(id: u64, score: f32, content: &str, tokens: i32) -> RetrievedChunk {
        RetrievedChunk {
            chunk_id: id,
            document_id: Uuid::nil(),
            source_id: format!("doc-{id}"),
            source_kind: "test".into(),
            chunk_index: 0,
            content: content.into(),
            token_estimate: tokens,
            score,
        }
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(pack_context(&[], &PackConfig::default()).is_empty());
    }

    #[test]
    fn fits_under_budget() {
        let hits = vec![
            chunk(1, 0.9, "alpha beta gamma", 100),
            chunk(2, 0.8, "delta epsilon zeta", 100),
            chunk(3, 0.7, "eta theta iota", 100),
        ];
        let cfg = PackConfig {
            max_tokens: 250,
            diversity_lambda: 0.0,
        };
        let packed = pack_context(&hits, &cfg);
        assert_eq!(packed.len(), 2);
        let total: i32 = packed.iter().map(|i| i.chunk.token_estimate).sum();
        assert!(total <= cfg.max_tokens, "total {} > budget {}", total, cfg.max_tokens);
    }

    #[test]
    fn diversity_breaks_ties_in_favor_of_novel_content() {
        // Two near-duplicates score the same; the diverse third should be picked.
        let hits = vec![
            chunk(1, 0.9, "the cat sat on the mat", 100),
            chunk(2, 0.9, "the cat sat on the mat again", 100),
            chunk(3, 0.85, "quantum mechanics describes subatomic particles", 100),
        ];
        let cfg = PackConfig {
            max_tokens: 200,
            diversity_lambda: 0.5,
        };
        let packed = pack_context(&hits, &cfg);
        assert_eq!(packed.len(), 2);
        let ids: Vec<u64> = packed.iter().map(|i| i.chunk.chunk_id).collect();
        // First should be the highest-scoring (id=1 or 2). Second should be the
        // diverse one (id=3), not the duplicate.
        assert!(ids.contains(&3), "expected diverse chunk id=3 in pack, got {ids:?}");
    }

    #[test]
    fn zero_budget_returns_empty() {
        let hits = vec![chunk(1, 0.9, "foo", 100)];
        let cfg = PackConfig {
            max_tokens: 0,
            diversity_lambda: 0.0,
        };
        assert!(pack_context(&hits, &cfg).is_empty());
    }
}
