//! OpenAI-compatible Chat Completions client.
//!
//! Default endpoint: `https://api.openai.com/v1/chat/completions`, overridable
//! via `OPENAI_BASE_URL` so daimon can point at ANY OpenAI-compatible server —
//! a local runtime (Ollama `/v1`, llama.cpp, vLLM, LM Studio), a gateway
//! (LiteLLM), or a subscription-fronting proxy — without per-token API charges.
//! Auth: `Authorization: Bearer <key>` (`OPENAI_API_KEY`; a proxy may accept any
//! placeholder). Tool-use uses OpenAI's `tools`/`tool_choice`/`tool_calls` shape,
//! mapped to/from daimon's provider-agnostic `ToolDefinition` / `ToolCall`.

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

const DEFAULT_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o";

pub struct OpenAiClient {
    http: Client,
    api_key: String,
    /// Full chat-completions endpoint, derived from the base URL.
    url: String,
    default_model: String,
}

impl OpenAiClient {
    /// Construct from env: `OPENAI_BASE_URL` (default `https://api.openai.com/v1`),
    /// `OPENAI_API_KEY`, `OPENAI_MODEL` (default `gpt-4o`). Pointing
    /// `OPENAI_BASE_URL` at a local runtime or a subscription-fronting proxy is
    /// the zero-API-charge path.
    pub fn from_env() -> Result<Self> {
        let base =
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.to_string());
        // A proxy / local server often accepts any token; still require the var
        // so auth is explicit, but allow a placeholder.
        let api_key =
            std::env::var("OPENAI_API_KEY").map_err(|_| Error::MissingApiKey("OPENAI_API_KEY"))?;
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        Self::with_base(api_key, base, model)
    }

    /// Construct against the default OpenAI endpoint.
    pub fn new(api_key: String, default_model: String) -> Result<Self> {
        Self::with_base(api_key, DEFAULT_BASE.to_string(), default_model)
    }

    /// Construct against an explicit OpenAI-compatible base URL (e.g.
    /// `http://localhost:11434/v1` for Ollama). The chat-completions path is
    /// appended.
    pub fn with_base(api_key: String, base_url: String, default_model: String) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        Ok(Self {
            http,
            api_key,
            url,
            default_model,
        })
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    fn provider(&self) -> &'static str {
        "openai"
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
            .post(&self.url)
            .bearer_auth(&self.api_key)
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
        let raw: ChatCompletion = resp
            .json()
            .await
            .map_err(|e| Error::Decode(format!("chat body: {e}")))?;
        raw.into_completion_response()
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
            .post(&self.url)
            .bearer_auth(&self.api_key)
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
            if event.data == "[DONE]" {
                return Ok(None);
            }
            let chunk: StreamChunk = serde_json::from_str(&event.data)
                .map_err(|e| Error::Decode(format!("chat stream chunk: {e}")))?;
            Ok(chunk.into_delta())
        });
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

// ---- request body -----------------------------------------------------------

fn build_request_body(req: &CompletionRequest, stream: bool) -> Json {
    let mut messages: Vec<Json> = Vec::new();
    if let Some(sys) = &req.system {
        messages.push(json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        messages.push(map_message(m));
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": messages,
        "stream": stream,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(
            req.tools
                .iter()
                .map(|t| json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                }))
                .collect::<Vec<_>>()
        );
    }
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    body
}

fn map_message(m: &ChatMessage) -> Json {
    match m.role {
        Role::System => json!({"role": "system", "content": m.content}),
        Role::User => json!({"role": "user", "content": m.content}),
        Role::Assistant => {
            if m.tool_calls.is_empty() {
                json!({"role": "assistant", "content": m.content})
            } else {
                let tool_calls: Vec<Json> = m
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                let mut o = json!({"role": "assistant", "tool_calls": tool_calls});
                if !m.content.is_empty() {
                    o["content"] = json!(m.content);
                }
                o
            }
        }
        Role::Tool => {
            let id = m.tool_use_id.clone().unwrap_or_default();
            json!({"role": "tool", "tool_call_id": id, "content": m.content})
        }
    }
}

// ---- response ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChatCompletion {
    id: String,
    model: String,
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChoiceToolCall>,
}

#[derive(Debug, Deserialize)]
struct ChoiceToolCall {
    id: String,
    function: ChoiceFunctionCall,
}

#[derive(Debug, Deserialize)]
struct ChoiceFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize, Default)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

impl ChatCompletion {
    fn into_completion_response(self) -> Result<CompletionResponse> {
        let choice = self
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Decode("openai: no choices in response".into()))?;
        let mut content = Vec::new();
        if let Some(text) = choice.message.content {
            if !text.is_empty() {
                content.push(AssistantContent::Text { text });
            }
        }
        for tc in choice.message.tool_calls {
            let parsed: Json = serde_json::from_str(&tc.function.arguments)
                .map_err(|e| Error::Decode(format!("openai tool args: {e}")))?;
            content.push(AssistantContent::ToolUse {
                id: tc.id,
                name: tc.function.name,
                input: parsed,
            });
        }
        let usage = self.usage.unwrap_or_default();
        Ok(CompletionResponse {
            id: self.id,
            model: self.model,
            content,
            stop_reason: parse_stop_reason(choice.finish_reason.as_deref()),
            usage: Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
        })
    }
}

fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("stop") => StopReason::EndTurn,
        Some("length") => StopReason::MaxTokens,
        Some("tool_calls") => StopReason::ToolUse,
        _ => StopReason::Error,
    }
}

// ---- stream ------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

impl StreamChunk {
    fn into_delta(self) -> Option<ContentDelta> {
        let Some(c) = self.choices.into_iter().next() else {
            return self.usage.map(|u| ContentDelta::MessageStop {
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: u.prompt_tokens,
                    output_tokens: u.completion_tokens,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
            });
        };
        if let Some(text) = c.delta.content.filter(|t| !t.is_empty()) {
            return Some(ContentDelta::TextDelta { text });
        }
        if let Some(reason) = c.finish_reason {
            return Some(ContentDelta::MessageStop {
                stop_reason: parse_stop_reason(Some(&reason)),
                usage: self
                    .usage
                    .map(|u| Usage {
                        input_tokens: u.prompt_tokens,
                        output_tokens: u.completion_tokens,
                        cache_read_input_tokens: 0,
                        cache_creation_input_tokens: 0,
                    })
                    .unwrap_or_default(),
            });
        }
        None
    }
}
