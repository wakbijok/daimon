//! LLM tier for daimon (Phase 4 D1).
//!
//! `LlmClient` is the provider-agnostic trait every model implementation
//! satisfies. Three concrete impls ship in Phase 4:
//!
//! - **AnthropicClient** — Claude Messages API (Opus 4.7, Sonnet 4.6, Haiku 4.5).
//!   Native tool-use support; streaming via Anthropic's SSE format.
//! - **OpenAiClient** — GPT-4o family via the Chat Completions API. Same
//!   tool-use surface as Anthropic (mapped to OpenAI function-calling).
//! - **LocalClient** — Ollama-compatible HTTP API for self-hosted models
//!   (Llama, Qwen, etc.). No tool-use guarantee — the orchestrator should
//!   keep tool-using flows on Anthropic/OpenAI until local model quality
//!   is measured.
//!
//! Every completion (streaming or batch) reports usage tokens. Callers
//! pipe those to the memory long-term tier (Phase 3) for cost dashboards
//! and to the audit log (Phase 2c) for compliance traceability.

pub mod anthropic;
pub mod error;
pub mod local;
pub mod openai;
pub mod types;

pub use anthropic::AnthropicClient;
pub use error::{Error, Result};
pub use local::LocalClient;
pub use openai::OpenAiClient;
pub use types::{
    AssistantContent, ChatMessage, CompletionRequest, CompletionResponse, ContentDelta, Role,
    StopReason, ToolCall, ToolDefinition, ToolResult, Usage,
};

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

/// Provider-agnostic LLM client.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Provider identifier ("anthropic", "openai", "local").
    fn provider(&self) -> &'static str;

    /// Default model id for this client (e.g. "claude-sonnet-4-6").
    fn default_model(&self) -> &str;

    /// Batch completion. Single request, single response.
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>;

    /// Streaming completion. Yields ContentDelta items as they arrive,
    /// terminated by a final delta carrying `stop_reason` + `usage`.
    async fn complete_stream(
        &self,
        req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ContentDelta>> + Send>>>;
}
