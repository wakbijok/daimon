//! Anthropic Messages API client.
//!
//! Endpoint: `https://api.anthropic.com/v1/messages`
//! Auth: `x-api-key` header.
//! Version header: `anthropic-version: 2023-06-01`.
//! Streaming: SSE events (`event: content_block_delta` etc.).

use std::pin::Pin;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::error::{Error, Result};
use crate::types::{
    AssistantContent, ChatMessage, CompletionRequest, CompletionResponse, ContentDelta, Role,
    StopReason, Usage,
};
use crate::LlmClient;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

pub struct AnthropicClient {
    http: Client,
    api_key: String,
    default_model: String,
}

impl AnthropicClient {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| Error::MissingApiKey("ANTHROPIC_API_KEY"))?;
        Self::new(api_key, DEFAULT_MODEL.to_string())
    }

    pub fn new(api_key: String, default_model: String) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            http,
            api_key,
            default_model,
        })
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    fn provider(&self) -> &'static str {
        "anthropic"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse> {
        if req.model.is_empty() {
            req.model = self.default_model.clone();
        }
        let body = build_request_body(&req, false);
        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::ApiError {
                status: status.as_u16(),
                body,
            });
        }
        let raw: RawMessageResponse = resp
            .json()
            .await
            .map_err(|e| Error::Decode(format!("messages body: {e}")))?;
        Ok(raw.into_completion_response())
    }

    async fn complete_stream(
        &self,
        mut req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ContentDelta>> + Send>>> {
        if req.model.is_empty() {
            req.model = self.default_model.clone();
        }
        let body = build_request_body(&req, true);
        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::ApiError {
                status: status.as_u16(),
                body,
            });
        }

        let event_stream = resp.bytes_stream().eventsource();
        let mapped = event_stream.map(|item| -> Result<Option<ContentDelta>> {
            let event = item.map_err(|e| Error::Stream(format!("sse: {e}")))?;
            // Anthropic uses `event:` to mark the type, `data:` JSON body.
            match event.event.as_str() {
                "content_block_start" => {
                    let parsed: ContentBlockStartEvent = serde_json::from_str(&event.data)
                        .map_err(|e| Error::Decode(format!("content_block_start: {e}")))?;
                    Ok(parsed.into_delta())
                }
                "content_block_delta" => {
                    let parsed: ContentBlockDeltaEvent = serde_json::from_str(&event.data)
                        .map_err(|e| Error::Decode(format!("content_block_delta: {e}")))?;
                    Ok(parsed.into_delta())
                }
                "content_block_stop" => {
                    let parsed: ContentBlockStopEvent = serde_json::from_str(&event.data)
                        .map_err(|e| Error::Decode(format!("content_block_stop: {e}")))?;
                    Ok(parsed.into_delta())
                }
                "message_delta" => {
                    let parsed: MessageDeltaEvent = serde_json::from_str(&event.data)
                        .map_err(|e| Error::Decode(format!("message_delta: {e}")))?;
                    Ok(parsed.into_delta())
                }
                "message_stop" | "ping" | "message_start" => Ok(None),
                other => {
                    tracing::debug!(event = other, "ignoring unknown anthropic event");
                    Ok(None)
                }
            }
        });
        // Filter out the Nones — only forward real deltas.
        let stream = mapped.filter_map(|r| async move {
            match r {
                Ok(Some(d)) => Some(Ok(d)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        });
        Ok(Box::pin(stream))
    }
}

// ---- request body shape ------------------------------------------------------

fn build_request_body(req: &CompletionRequest, stream: bool) -> Json {
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": map_messages(&req.messages),
        "stream": stream,
    });
    if let Some(sys) = &req.system {
        body["system"] = json!(sys);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(
            req.tools
                .iter()
                .map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                }))
                .collect::<Vec<_>>()
        );
    }
    body
}

fn map_messages(messages: &[ChatMessage]) -> Vec<Json> {
    messages
        .iter()
        .filter_map(|m| match m.role {
            Role::System => None, // Anthropic system prompt is separate from messages.
            Role::User => Some(json!({
                "role": "user",
                "content": m.content,
            })),
            Role::Assistant => {
                // If the assistant message issued tool_use blocks, encode as a
                // content array; otherwise it's a plain text message.
                if m.tool_calls.is_empty() {
                    Some(json!({
                        "role": "assistant",
                        "content": m.content,
                    }))
                } else {
                    let mut content = Vec::new();
                    if !m.content.is_empty() {
                        content.push(json!({"type": "text", "text": m.content}));
                    }
                    for tc in &m.tool_calls {
                        content.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        }));
                    }
                    Some(json!({
                        "role": "assistant",
                        "content": content,
                    }))
                }
            }
            Role::Tool => {
                let id = m.tool_use_id.clone().unwrap_or_default();
                Some(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": m.content,
                    }],
                }))
            }
        })
        .collect()
}

// ---- response shape ----------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawMessageResponse {
    id: String,
    model: String,
    content: Vec<RawContentBlock>,
    stop_reason: Option<String>,
    usage: RawUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RawContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: Json },
}

#[derive(Debug, Deserialize, Default)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

impl RawMessageResponse {
    fn into_completion_response(self) -> CompletionResponse {
        let content = self
            .content
            .into_iter()
            .map(|c| match c {
                RawContentBlock::Text { text } => AssistantContent::Text { text },
                RawContentBlock::ToolUse { id, name, input } => {
                    AssistantContent::ToolUse { id, name, input }
                }
            })
            .collect();
        CompletionResponse {
            id: self.id,
            model: self.model,
            content,
            stop_reason: parse_stop_reason(self.stop_reason.as_deref()),
            usage: Usage {
                input_tokens: self.usage.input_tokens,
                output_tokens: self.usage.output_tokens,
                cache_read_input_tokens: self.usage.cache_read_input_tokens,
                cache_creation_input_tokens: self.usage.cache_creation_input_tokens,
            },
        }
    }
}

fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::Error,
    }
}

// ---- stream event shapes -----------------------------------------------------

#[derive(Debug, Deserialize)]
struct ContentBlockStartEvent {
    index: u32,
    content_block: RawContentBlock,
}

impl ContentBlockStartEvent {
    fn into_delta(self) -> Option<ContentDelta> {
        let _ = self.index;
        match self.content_block {
            RawContentBlock::Text { .. } => None, // text start is followed by deltas
            RawContentBlock::ToolUse { id, name, .. } => Some(ContentDelta::ToolUseStart { id, name }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ContentBlockDeltaEvent {
    delta: RawDelta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RawDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

impl ContentBlockDeltaEvent {
    fn into_delta(self) -> Option<ContentDelta> {
        match self.delta {
            RawDelta::TextDelta { text } => Some(ContentDelta::TextDelta { text }),
            RawDelta::InputJsonDelta { partial_json } => {
                Some(ContentDelta::ToolUseInputDelta { partial_json })
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ContentBlockStopEvent {
    #[allow(dead_code)]
    index: u32,
}

impl ContentBlockStopEvent {
    fn into_delta(self) -> Option<ContentDelta> {
        // Anthropic doesn't carry id on content_block_stop; the caller tracks
        // current block by stream order. We surface a generic ToolUseStop with
        // empty id — the chat loop ignores it for text blocks.
        Some(ContentDelta::ToolUseStop { id: String::new() })
    }
}

#[derive(Debug, Deserialize)]
struct MessageDeltaEvent {
    delta: MessageDelta,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
struct MessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

impl MessageDeltaEvent {
    fn into_delta(self) -> Option<ContentDelta> {
        Some(ContentDelta::MessageStop {
            stop_reason: parse_stop_reason(self.delta.stop_reason.as_deref()),
            usage: self
                .usage
                .map(|u| Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_read_input_tokens: u.cache_read_input_tokens,
                    cache_creation_input_tokens: u.cache_creation_input_tokens,
                })
                .unwrap_or_default(),
        })
    }
}
