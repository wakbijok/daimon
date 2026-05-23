//! Ollama-compatible local LLM client.
//!
//! Talks to an Ollama server (or any service that mirrors its `/api/chat`
//! endpoint). Tool-use is NOT guaranteed — many local models lack
//! function-calling discipline. Callers should keep tool-using flows on
//! Anthropic/OpenAI until local model quality is measured.
//!
//! Endpoint: `$DAIMON_LLM_LOCAL_URL` (default `http://localhost:11434`).

use std::pin::Pin;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::error::{Error, Result};
use crate::types::{
    AssistantContent, CompletionRequest, CompletionResponse, ContentDelta, Role, StopReason, Usage,
};
use crate::LlmClient;

const DEFAULT_BASE: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "llama3.2";

pub struct LocalClient {
    http: Client,
    base_url: String,
    default_model: String,
}

impl LocalClient {
    pub fn from_env() -> Result<Self> {
        let base_url =
            std::env::var("DAIMON_LLM_LOCAL_URL").unwrap_or_else(|_| DEFAULT_BASE.to_string());
        Self::new(base_url, DEFAULT_MODEL.to_string())
    }

    pub fn new(base_url: String, default_model: String) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            http,
            base_url,
            default_model,
        })
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl LlmClient for LocalClient {
    fn provider(&self) -> &'static str {
        "local"
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
            .post(self.chat_url())
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
        let raw: OllamaChatResponse = resp
            .json()
            .await
            .map_err(|e| Error::Decode(format!("ollama body: {e}")))?;
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
            .post(self.chat_url())
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

        // Ollama streams newline-delimited JSON, not SSE. Build a line-buffered
        // mapper.
        let byte_stream = resp.bytes_stream();
        let mapped = byte_stream.map(|chunk| -> Result<Vec<ContentDelta>> {
            let bytes = chunk.map_err(|e| Error::Stream(format!("ollama bytes: {e}")))?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| Error::Decode(format!("ollama bytes utf8: {e}")))?;
            let mut deltas = Vec::new();
            for line in text.split('\n').filter(|l| !l.trim().is_empty()) {
                let chunk: OllamaStreamChunk = serde_json::from_str(line)
                    .map_err(|e| Error::Decode(format!("ollama line: {e}: line=`{line}`")))?;
                if let Some(d) = chunk.into_delta() {
                    deltas.push(d);
                }
            }
            Ok(deltas)
        });
        let flat = mapped.flat_map(|r| match r {
            Ok(deltas) => futures::stream::iter(deltas.into_iter().map(Ok)).boxed(),
            Err(e) => futures::stream::iter(std::iter::once(Err(e))).boxed(),
        });
        Ok(Box::pin(flat))
    }
}

fn build_request_body(req: &CompletionRequest, stream: bool) -> Json {
    let mut messages: Vec<Json> = Vec::new();
    if let Some(sys) = &req.system {
        messages.push(json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        messages.push(json!({"role": role, "content": m.content}));
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": stream,
        "options": { "num_predict": req.max_tokens }
    });
    if let Some(t) = req.temperature {
        body["options"]["temperature"] = json!(t);
    }
    body
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: OllamaMessage,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
}

impl OllamaChatResponse {
    fn into_completion_response(self) -> CompletionResponse {
        CompletionResponse {
            id: format!("ollama-{}", uuid::Uuid::new_v4()),
            model: self.model,
            content: vec![AssistantContent::Text {
                text: self.message.content,
            }],
            stop_reason: parse_stop_reason(self.done_reason.as_deref()),
            usage: Usage {
                input_tokens: self.prompt_eval_count,
                output_tokens: self.eval_count,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaMessage>,
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

impl OllamaStreamChunk {
    fn into_delta(self) -> Option<ContentDelta> {
        if self.done {
            return Some(ContentDelta::MessageStop {
                stop_reason: parse_stop_reason(self.done_reason.as_deref()),
                usage: Usage {
                    input_tokens: self.prompt_eval_count,
                    output_tokens: self.eval_count,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
            });
        }
        let text = self.message?.content;
        if text.is_empty() {
            return None;
        }
        Some(ContentDelta::TextDelta { text })
    }
}

fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("stop") => StopReason::EndTurn,
        Some("length") => StopReason::MaxTokens,
        _ => StopReason::Error,
    }
}
