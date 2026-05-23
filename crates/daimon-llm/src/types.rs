//! Provider-agnostic types. The Anthropic, OpenAI, and Local impls translate
//! their wire formats to/from these.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    /// Text content. For multi-modal or tool-result messages, callers should
    /// use the higher-level orchestration types — this is the simple-text
    /// case.
    pub content: String,
    /// For `Role::Tool`: which tool_use this is responding to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// For `Role::Assistant` messages that included tool_use blocks: the
    /// tool calls the assistant issued.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_use_id: None,
            tool_calls: Vec::new(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_use_id: None,
            tool_calls: Vec::new(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_use_id: None,
            tool_calls: Vec::new(),
        }
    }
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_use_id: Some(tool_use_id.into()),
            tool_calls: Vec::new(),
        }
    }
}

/// Tool definition the LLM may invoke.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema describing the parameters object.
    pub input_schema: Json,
}

/// A tool invocation emitted by the LLM in an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-issued id; pass back as `tool_use_id` in the tool_result.
    pub id: String,
    pub name: String,
    pub arguments: Json,
}

/// Result of executing a tool call, fed back to the LLM as a Role::Tool message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Model id. If empty, use the client's default.
    #[serde(default)]
    pub model: String,
    pub messages: Vec<ChatMessage>,
    /// Optional system prompt (separate from the messages list per Anthropic
    /// convention; OpenAI maps to a Role::System message at the front).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// Optional caller-supplied request id (for correlating across audit
    /// events + telemetry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<AssistantContent>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

/// Content block of an assistant message — either text or tool-use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantContent {
    Text { text: String },
    ToolUse { id: String, name: String, input: Json },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    Error,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub cache_creation_input_tokens: u32,
}

/// Streaming delta. Multiple variants because Anthropic's stream emits
/// distinct event types for text deltas vs tool-use input deltas vs the
/// terminal message-stop with usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    TextDelta {
        text: String,
    },
    ToolUseStart {
        id: String,
        name: String,
    },
    ToolUseInputDelta {
        partial_json: String,
    },
    ToolUseStop {
        id: String,
    },
    MessageStop {
        stop_reason: StopReason,
        usage: Usage,
    },
}
