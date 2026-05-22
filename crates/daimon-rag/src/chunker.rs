//! Token-approximate text chunking for embedding ingest.
//!
//! Splits on whitespace boundaries with a sliding-window approach. Uses a word→token
//! approximation (1 token ≈ 0.75 words for English). Sentence-boundary awareness is
//! a Phase 3 D6 improvement; this lite version is intentionally simple to unblock the
//! end-to-end retrieval test.

/// Approximate words-per-token ratio for English. Used to convert token-budget into
/// word-budget for the simple splitter below.
const WORDS_PER_TOKEN: f32 = 0.75;

#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub chunk_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_tokens: 512,
            overlap_tokens: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 0-based index in source order.
    pub index: usize,
    /// Inclusive start word offset in the source content.
    pub word_start: usize,
    /// Exclusive end word offset.
    pub word_end: usize,
    /// The chunk text (reconstructed from words, single-space joined).
    pub text: String,
}

/// Chunk `content` into overlapping windows according to `cfg`. Returns at least one chunk
/// even for short inputs.
pub fn chunk(content: &str, cfg: &ChunkConfig) -> Vec<Chunk> {
    let words: Vec<&str> = content.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }
    let chunk_words = ((cfg.chunk_tokens as f32) / WORDS_PER_TOKEN).round() as usize;
    let overlap_words = ((cfg.overlap_tokens as f32) / WORDS_PER_TOKEN).round() as usize;
    let stride = chunk_words.saturating_sub(overlap_words).max(1);

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut idx = 0usize;
    while start < words.len() {
        let end = (start + chunk_words).min(words.len());
        let text = words[start..end].join(" ");
        out.push(Chunk {
            index: idx,
            word_start: start,
            word_end: end,
            text,
        });
        if end == words.len() {
            break;
        }
        start += stride;
        idx += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(chunk("", &ChunkConfig::default()).is_empty());
        assert!(chunk("   \n\t  ", &ChunkConfig::default()).is_empty());
    }

    #[test]
    fn short_input_yields_one_chunk() {
        let cs = chunk("hello world", &ChunkConfig::default());
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "hello world");
        assert_eq!(cs[0].word_start, 0);
        assert_eq!(cs[0].word_end, 2);
    }

    #[test]
    fn long_input_overlaps_correctly() {
        let words: Vec<String> = (0..1000).map(|i| format!("w{}", i)).collect();
        let content = words.join(" ");
        let cfg = ChunkConfig {
            chunk_tokens: 100,
            overlap_tokens: 10,
        };
        let cs = chunk(&content, &cfg);
        assert!(cs.len() > 1, "should produce multiple chunks");
        for win in cs.windows(2) {
            // overlap = previous chunk's last `overlap_words` should equal next chunk's first `overlap_words`
            assert!(win[1].word_start < win[0].word_end, "chunks should overlap");
        }
        assert_eq!(cs.last().unwrap().word_end, 1000, "last chunk should reach end");
    }
}
