//! ChatGPT subscription client via the Codex OAuth backend (zero API charge).
//!
//! Instead of a metered `x-api-key`, this client uses the ChatGPT **OAuth**
//! session that `codex login` (or Hermes) already established — the same
//! subscription-backed path, no per-token billing. Mechanism (verified live
//! against the backend):
//!
//! - **Auth**: read `~/.codex/auth.json` (`CODEX_AUTH_FILE` overrides) —
//!   `tokens.{access_token, refresh_token, account_id}`. On a `401`, refresh via
//!   `https://auth.openai.com/oauth/token` (`grant_type=refresh_token`, the codex
//!   OAuth `client_id`) and persist the rotated tokens back atomically, so codex
//!   and daimon stay in sync.
//! - **Endpoint**: `POST https://chatgpt.com/backend-api/codex/responses`
//!   (OpenAI Responses API) with headers `Authorization: Bearer <access_token>`,
//!   `chatgpt-account-id`, `OpenAI-Beta: responses=experimental`,
//!   `originator: codex_cli_rs`.
//! - **Model**: `gpt-5.5` by default (`DAIMON_CHATGPT_MODEL`) — a ChatGPT account
//!   rejects raw `gpt-5*` API model ids.
//!
//! The Responses SSE event stream is mapped to daimon's provider-agnostic
//! `ContentDelta`, so the chat turn loop is unchanged.

use std::path::PathBuf;
use std::pin::Pin;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::LlmClient;
use crate::error::{Error, Result};
use crate::types::{
    AssistantContent, CompletionRequest, CompletionResponse, ContentDelta, Role, StopReason, Usage,
};

const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// The codex CLI's public OAuth client id (subscription sign-in).
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEFAULT_MODEL: &str = "gpt-5.5";

pub struct ChatGptOAuthClient {
    http: Client,
    auth_path: PathBuf,
    default_model: String,
    effort: String,
}

#[derive(Deserialize)]
struct AuthFile {
    tokens: AuthTokens,
}

#[derive(Deserialize, Clone)]
struct AuthTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    account_id: String,
}

#[derive(Deserialize)]
struct RefreshResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

fn default_auth_path() -> PathBuf {
    // ~/.codex/auth.json (HOME on unix; USERPROFILE elsewhere as a fallback).
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".codex").join("auth.json")
}

impl ChatGptOAuthClient {
    /// From env: `CODEX_AUTH_FILE` (default `~/.codex/auth.json`),
    /// `DAIMON_CHATGPT_MODEL` (default `gpt-5.5`), `DAIMON_CHATGPT_EFFORT`
    /// (reasoning effort, default `low`).
    pub fn from_env() -> Result<Self> {
        let auth_path = std::env::var("CODEX_AUTH_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_auth_path());
        if !auth_path.exists() {
            return Err(Error::MissingApiKey("CODEX_AUTH_FILE (~/.codex/auth.json)"));
        }
        let default_model =
            std::env::var("DAIMON_CHATGPT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let effort = std::env::var("DAIMON_CHATGPT_EFFORT").unwrap_or_else(|_| "low".to_string());
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        Ok(Self {
            http,
            auth_path,
            default_model,
            effort,
        })
    }

    fn load_tokens(&self) -> Result<AuthTokens> {
        let raw = std::fs::read_to_string(&self.auth_path)
            .map_err(|e| Error::Other(format!("read {}: {e}", self.auth_path.display())))?;
        let af: AuthFile =
            serde_json::from_str(&raw).map_err(|e| Error::Other(format!("parse auth.json: {e}")))?;
        Ok(af.tokens)
    }

    /// Exchange the refresh token for a fresh access token and persist the
    /// rotated tokens back to `auth.json` (atomic), so codex + daimon share one
    /// live session.
    async fn refresh(&self) -> Result<AuthTokens> {
        let cur = self.load_tokens()?;
        let resp = self
            .http
            .post(TOKEN_URL)
            .json(&json!({
                "client_id": CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": cur.refresh_token,
                "scope": "openid profile email",
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::ApiError { status, body });
        }
        let r: RefreshResp = resp
            .json()
            .await
            .map_err(|e| Error::Decode(format!("oauth refresh: {e}")))?;

        // Persist: mutate the existing file (preserve unknown keys) + atomic swap.
        let raw = std::fs::read_to_string(&self.auth_path)
            .map_err(|e| Error::Other(format!("read auth.json: {e}")))?;
        let mut root: Json = serde_json::from_str(&raw)
            .map_err(|e| Error::Other(format!("parse auth.json: {e}")))?;
        root["tokens"]["access_token"] = json!(r.access_token);
        if let Some(rt) = &r.refresh_token {
            root["tokens"]["refresh_token"] = json!(rt);
        }
        if let Some(it) = &r.id_token {
            root["tokens"]["id_token"] = json!(it);
        }
        root["last_refresh"] = json!(chrono::Utc::now().to_rfc3339());
        let tmp = self.auth_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&root).unwrap_or_default())
            .map_err(|e| Error::Other(format!("write auth.json: {e}")))?;
        std::fs::rename(&tmp, &self.auth_path)
            .map_err(|e| Error::Other(format!("swap auth.json: {e}")))?;

        let mut tokens = cur;
        tokens.access_token = r.access_token;
        if let Some(rt) = r.refresh_token {
            tokens.refresh_token = rt;
        }
        Ok(tokens)
    }

    fn build_body(&self, req: &CompletionRequest, stream: bool) -> Json {
        let model = if req.model.is_empty() {
            self.default_model.clone()
        } else {
            req.model.clone()
        };
        let mut input: Vec<Json> = Vec::new();
        for m in &req.messages {
            match m.role {
                Role::User | Role::System => input.push(json!({
                    "type": "message", "role": "user",
                    "content": [{ "type": "input_text", "text": m.content }],
                })),
                Role::Assistant => {
                    if !m.content.is_empty() {
                        input.push(json!({
                            "type": "message", "role": "assistant",
                            "content": [{ "type": "output_text", "text": m.content }],
                        }));
                    }
                    for tc in &m.tool_calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.name,
                            "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into()),
                        }));
                    }
                }
                Role::Tool => input.push(json!({
                    "type": "function_call_output",
                    "call_id": m.tool_use_id.clone().unwrap_or_default(),
                    "output": m.content,
                })),
            }
        }
        let mut body = json!({
            "model": model,
            "input": input,
            "stream": stream,
            "store": false,
            "parallel_tool_calls": false,
            "reasoning": { "effort": self.effort },
        });
        if let Some(sys) = &req.system {
            body["instructions"] = json!(sys);
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|t| json!({
                        "type": "function",
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }))
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = json!("auto");
        }
        body
    }

    async fn post_responses(
        &self,
        access_token: &str,
        account_id: &str,
        body: &Json,
    ) -> Result<reqwest::Response> {
        Ok(self
            .http
            .post(RESPONSES_URL)
            .bearer_auth(access_token)
            .header("chatgpt-account-id", account_id)
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "codex_cli_rs")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(body)
            .send()
            .await?)
    }

    /// POST with a 401-driven single refresh+retry.
    async fn send_with_refresh(&self, body: &Json) -> Result<reqwest::Response> {
        let t = self.load_tokens()?;
        let resp = self.post_responses(&t.access_token, &t.account_id, body).await?;
        if resp.status().as_u16() == 401 {
            let fresh = self.refresh().await?;
            let resp = self
                .post_responses(&fresh.access_token, &fresh.account_id, body)
                .await?;
            return Ok(resp);
        }
        Ok(resp)
    }
}

#[async_trait]
impl LlmClient for ChatGptOAuthClient {
    fn provider(&self) -> &'static str {
        "chatgpt"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse> {
        // Aggregate the stream into a single response (the endpoint always
        // streams; chat.rs uses complete_stream — this is for other callers).
        let model = if req.model.is_empty() {
            self.default_model.clone()
        } else {
            req.model.clone()
        };
        let mut stream = self.complete_stream(req).await?;
        let mut text = String::new();
        let mut tool: Option<(String, String, String)> = None; // (id, name, args)
        let mut tools: Vec<AssistantContent> = Vec::new();
        let mut stop = StopReason::EndTurn;
        let mut usage = Usage::default();
        while let Some(item) = stream.next().await {
            match item? {
                ContentDelta::TextDelta { text: t } => text.push_str(&t),
                ContentDelta::ToolUseStart { id, name } => tool = Some((id, name, String::new())),
                ContentDelta::ToolUseInputDelta { partial_json } => {
                    if let Some(t) = tool.as_mut() {
                        t.2.push_str(&partial_json);
                    }
                }
                ContentDelta::ToolUseStop { .. } => {
                    if let Some((id, name, args)) = tool.take() {
                        let input = serde_json::from_str(&args).unwrap_or_else(|_| json!({}));
                        tools.push(AssistantContent::ToolUse { id, name, input });
                    }
                }
                ContentDelta::MessageStop {
                    stop_reason,
                    usage: u,
                } => {
                    stop = stop_reason;
                    usage = u;
                }
            }
        }
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(AssistantContent::Text { text });
        }
        content.extend(tools);
        Ok(CompletionResponse {
            id: String::new(),
            model,
            content,
            stop_reason: stop,
            usage,
        })
    }

    async fn complete_stream(
        &self,
        req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ContentDelta>> + Send>>> {
        let body = self.build_body(&req, true);
        let resp = self.send_with_refresh(&body).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::ApiError {
                status: status.as_u16(),
                body,
            });
        }
        let events = resp.bytes_stream().eventsource();
        let mapped = events.flat_map(|item| {
            let deltas: Vec<Result<ContentDelta>> = match item {
                Ok(ev) => parse_event(&ev.data),
                Err(e) => vec![Err(Error::Stream(format!("sse: {e}")))],
            };
            futures::stream::iter(deltas)
        });
        Ok(Box::pin(mapped))
    }
}

/// Map one Responses SSE event to zero or more daimon `ContentDelta`s.
fn parse_event(data: &str) -> Vec<Result<ContentDelta>> {
    if data.trim().is_empty() || data == "[DONE]" {
        return vec![];
    }
    let v: Json = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return vec![Err(Error::Decode(format!("responses event: {e}")))],
    };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "response.output_text.delta" => {
            let text = v.get("delta").and_then(|d| d.as_str()).unwrap_or("").to_string();
            vec![Ok(ContentDelta::TextDelta { text })]
        }
        "response.output_item.added" => {
            let item = &v["item"];
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                let id = call_id(item);
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                vec![Ok(ContentDelta::ToolUseStart { id, name })]
            } else {
                vec![]
            }
        }
        "response.function_call_arguments.delta" => {
            let partial_json = v.get("delta").and_then(|d| d.as_str()).unwrap_or("").to_string();
            vec![Ok(ContentDelta::ToolUseInputDelta { partial_json })]
        }
        "response.output_item.done" => {
            let item = &v["item"];
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                vec![Ok(ContentDelta::ToolUseStop { id: call_id(item) })]
            } else {
                vec![]
            }
        }
        "response.completed" => {
            let response = &v["response"];
            // ToolUse iff any output item is a function_call.
            let saw_tool = response
                .get("output")
                .and_then(|o| o.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|it| it.get("type").and_then(|t| t.as_str()) == Some("function_call"))
                })
                .unwrap_or(false);
            let stop_reason = if saw_tool {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            };
            let u = &response["usage"];
            let usage = Usage {
                input_tokens: u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                ..Default::default()
            };
            vec![Ok(ContentDelta::MessageStop { stop_reason, usage })]
        }
        "response.failed" | "response.incomplete" | "error" => {
            let msg = v["response"]["error"]["message"]
                .as_str()
                .or_else(|| v["error"]["message"].as_str())
                .or_else(|| v["message"].as_str())
                .unwrap_or("responses stream failed")
                .to_string();
            vec![Err(Error::Stream(msg))]
        }
        _ => vec![],
    }
}

/// Prefer `call_id` (the id the tool result must echo); fall back to `id`.
fn call_id(item: &Json) -> String {
    item.get("call_id")
        .and_then(|c| c.as_str())
        .or_else(|| item.get("id").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, ToolCall, ToolDefinition};

    #[test]
    fn body_maps_history_and_tools() {
        let client = ChatGptOAuthClient {
            http: Client::new(),
            auth_path: PathBuf::from("/nonexistent"),
            default_model: "gpt-5.5".into(),
            effort: "low".into(),
        };
        let req = CompletionRequest {
            model: String::new(),
            system: Some("be terse".into()),
            messages: vec![
                ChatMessage::user("list rules"),
                ChatMessage {
                    role: Role::Assistant,
                    content: String::new(),
                    tool_use_id: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "fw.list".into(),
                        arguments: json!({"target":"edge"}),
                    }],
                },
                ChatMessage::tool_result("call_1", "6 rules"),
            ],
            max_tokens: 1024,
            temperature: None,
            tools: vec![ToolDefinition {
                name: "fw.list".into(),
                description: "list".into(),
                input_schema: json!({"type":"object"}),
            }],
            request_id: None,
        };
        let body = client.build_body(&req, true);
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["instructions"], "be terse");
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(body["tools"][0]["name"], "fw.list");
    }

    #[test]
    fn parse_text_delta() {
        let d = parse_event(r#"{"type":"response.output_text.delta","delta":"Hel"}"#);
        assert!(matches!(&d[0], Ok(ContentDelta::TextDelta { text }) if text == "Hel"));
    }

    #[test]
    fn parse_tool_call_flow() {
        let start = parse_event(
            r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_9","name":"fw.list"}}"#,
        );
        assert!(matches!(&start[0], Ok(ContentDelta::ToolUseStart { id, name }) if id=="call_9" && name=="fw.list"));
        let arg = parse_event(
            r#"{"type":"response.function_call_arguments.delta","delta":"{\"t\":1}"}"#,
        );
        assert!(matches!(&arg[0], Ok(ContentDelta::ToolUseInputDelta { partial_json }) if partial_json == r#"{"t":1}"#));
        let done = parse_event(
            r#"{"type":"response.output_item.done","item":{"type":"function_call","call_id":"call_9"}}"#,
        );
        assert!(matches!(&done[0], Ok(ContentDelta::ToolUseStop { id }) if id == "call_9"));
    }

    /// Live smoke against the real ChatGPT backend (needs ~/.codex/auth.json +
    /// network). Run: `cargo test -p daimon-llm live_smoke -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_smoke() {
        let c = ChatGptOAuthClient::from_env().expect("codex auth");
        let req = CompletionRequest {
            model: String::new(),
            system: Some("Be terse.".into()),
            messages: vec![ChatMessage::user("Reply with exactly: PONG")],
            max_tokens: 32,
            temperature: None,
            tools: vec![],
            request_id: None,
        };
        let mut stream = c.complete_stream(req).await.expect("stream");
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            match item.expect("delta") {
                ContentDelta::TextDelta { text: t } => text.push_str(&t),
                ContentDelta::MessageStop { .. } => break,
                _ => {}
            }
        }
        println!("LIVE CHATGPT REPLY: {text:?}");
        assert!(text.to_uppercase().contains("PONG"), "got: {text:?}");
    }

    #[test]
    fn parse_completed_stop_reason() {
        let end = parse_event(
            r#"{"type":"response.completed","response":{"output":[{"type":"message"}],"usage":{"input_tokens":10,"output_tokens":3}}}"#,
        );
        assert!(matches!(&end[0], Ok(ContentDelta::MessageStop { stop_reason, usage }) if *stop_reason==StopReason::EndTurn && usage.input_tokens==10));
        let tool = parse_event(
            r#"{"type":"response.completed","response":{"output":[{"type":"function_call"}],"usage":{}}}"#,
        );
        assert!(matches!(&tool[0], Ok(ContentDelta::MessageStop { stop_reason, .. }) if *stop_reason==StopReason::ToolUse));
    }
}
