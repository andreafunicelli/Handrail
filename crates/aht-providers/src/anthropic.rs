//! Anthropic Messages API adapter (SSE streaming, tool use, extended thinking deltas).

use crate::error::{Error, Result};
use crate::http::{parse_tool_args, pump, Decoder, Http};
use crate::sse::SseEvent;
use crate::types::*;
use crate::ChatProvider;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct AnthropicProvider {
    http: Http,
    base_url: String,
    api_key: Option<String>,
    /// Also send `Authorization: Bearer` (gateways such as OpenCode Zen accept it).
    bearer_too: bool,
}

impl AnthropicProvider {
    /// `base_url` may end in `/v1` (e.g. `https://api.anthropic.com/v1`) or not
    /// (e.g. DeepSeek's `https://api.deepseek.com/anthropic`); `/v1` is added when missing.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            http: Http::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            bearer_too: false,
        }
    }

    pub fn with_bearer(mut self, yes: bool) -> Self {
        self.bearer_too = yes;
        self
    }

    pub fn with_retry_backoff(mut self, d: Duration) -> Self {
        self.http.retry_backoff = d;
        self
    }

    fn url(&self, path: &str) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/{path}", self.base_url)
        } else {
            format!("{}/v1/{path}", self.base_url)
        }
    }

    fn headers(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut rb = rb.header("anthropic-version", "2023-06-01");
        if let Some(k) = &self.api_key {
            rb = rb.header("x-api-key", k);
            if self.bearer_too {
                rb = rb.bearer_auth(k);
            }
        }
        rb
    }
}

pub(crate) fn build_body(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .filter_map(|m| {
            let mut blocks: Vec<Value> = Vec::new();
            // tool_result blocks must come first in a user turn.
            for p in &m.content {
                if let Part::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = p
                {
                    let mut b =
                        json!({"type":"tool_result","tool_use_id":tool_use_id,"content":content});
                    if *is_error {
                        b["is_error"] = json!(true);
                    }
                    blocks.push(b);
                }
            }
            for p in &m.content {
                match p {
                    Part::Text { text } if !text.is_empty() => {
                        blocks.push(json!({"type":"text","text":text}))
                    }
                    Part::ToolUse { id, name, input } => {
                        blocks.push(json!({"type":"tool_use","id":id,"name":name,"input":input}))
                    }
                    _ => {}
                }
            }
            if blocks.is_empty() {
                return None;
            }
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            Some(json!({"role":role,"content":blocks}))
        })
        .collect();
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "stream": true,
        "messages": messages,
    });
    if let Some(s) = req.system.as_ref().filter(|s| !s.is_empty()) {
        body["system"] = json!(s);
    }
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.input_schema}))
            .collect();
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

enum Block {
    Text,
    Thinking,
    Tool {
        id: String,
        name: String,
        json: String,
    },
}

#[derive(Default)]
pub(crate) struct AnthropicDecoder {
    blocks: HashMap<u64, Block>,
    input_tokens: u64,
    output_tokens: u64,
    stop: Option<StopReason>,
    done: bool,
}

fn map_stop(r: &str) -> StopReason {
    match r {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        _ => StopReason::Other,
    }
}

impl Decoder for AnthropicDecoder {
    fn feed(&mut self, ev: &SseEvent) -> Result<Vec<ProviderEvent>> {
        let kind = ev.event.as_deref().unwrap_or("");
        if kind == "ping" || ev.data.trim().is_empty() {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(&ev.data)
            .map_err(|e| Error::Protocol(format!("bad SSE json: {e}")))?;
        let kind = if kind.is_empty() {
            v["type"].as_str().unwrap_or("")
        } else {
            kind
        };
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                let u = &v["message"]["usage"];
                self.input_tokens = u["input_tokens"].as_u64().unwrap_or(0)
                    + u["cache_creation_input_tokens"].as_u64().unwrap_or(0)
                    + u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                self.output_tokens = u["output_tokens"].as_u64().unwrap_or(0);
            }
            "content_block_start" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                let cb = &v["content_block"];
                let block = match cb["type"].as_str() {
                    Some("tool_use") => Block::Tool {
                        id: cb["id"].as_str().unwrap_or_default().into(),
                        name: cb["name"].as_str().unwrap_or_default().into(),
                        json: String::new(),
                    },
                    Some("thinking") => {
                        if let Some(t) = cb["thinking"].as_str().filter(|t| !t.is_empty()) {
                            out.push(ProviderEvent::ThinkingDelta(t.into()));
                        }
                        Block::Thinking
                    }
                    _ => {
                        if let Some(t) = cb["text"].as_str().filter(|t| !t.is_empty()) {
                            out.push(ProviderEvent::TextDelta(t.into()));
                        }
                        Block::Text
                    }
                };
                self.blocks.insert(idx, block);
            }
            "content_block_delta" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                let d = &v["delta"];
                match d["type"].as_str() {
                    Some("text_delta") => {
                        if let Some(t) = d["text"].as_str() {
                            out.push(ProviderEvent::TextDelta(t.into()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(t) = d["thinking"].as_str() {
                            out.push(ProviderEvent::ThinkingDelta(t.into()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(Block::Tool { json, .. }) = self.blocks.get_mut(&idx) {
                            json.push_str(d["partial_json"].as_str().unwrap_or_default());
                        }
                    }
                    _ => {} // signature_delta, citations...
                }
            }
            "content_block_stop" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                if let Some(Block::Tool { id, name, json }) = self.blocks.remove(&idx) {
                    out.push(ProviderEvent::ToolUse {
                        id,
                        name,
                        input: parse_tool_args(&json)?,
                    });
                }
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.stop = Some(map_stop(r));
                }
                let u = &v["usage"];
                if let Some(o) = u["output_tokens"].as_u64() {
                    self.output_tokens = o;
                }
                if let Some(i) = u["input_tokens"].as_u64().filter(|i| *i > 0) {
                    self.input_tokens = i;
                }
            }
            "message_stop" => {
                self.done = true;
                out.push(ProviderEvent::Usage {
                    input: self.input_tokens,
                    output: self.output_tokens,
                });
                out.push(ProviderEvent::Stop {
                    reason: self.stop.unwrap_or(StopReason::EndTurn),
                });
            }
            "error" => {
                let t = v["error"]["type"].as_str().unwrap_or("error");
                let m = v["error"]["message"].as_str().unwrap_or("unknown error");
                return Err(match t {
                    "overloaded_error" | "api_error" => Error::Server {
                        status: 529,
                        message: m.into(),
                    },
                    "rate_limit_error" => Error::RateLimited {
                        message: m.into(),
                        retry_after: None,
                    },
                    "authentication_error" | "permission_error" => Error::Auth(m.into()),
                    _ => Error::BadRequest {
                        status: 400,
                        message: format!("{t}: {m}"),
                    },
                });
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<ProviderEvent>> {
        if self.done {
            Ok(vec![])
        } else {
            Err(Error::Protocol(
                "stream ended before message_stop".to_string(),
            ))
        }
    }
}

#[async_trait]
impl ChatProvider for AnthropicProvider {
    async fn stream(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>> {
        let body = build_body(&req);
        let url = self.url("messages");
        let resp = self
            .http
            .send(
                || {
                    self.headers(self.http.client().post(&url))
                        .header("accept", "text/event-stream")
                        .json(&body)
                },
                &cancel,
            )
            .await?;
        Ok(pump(resp, AnthropicDecoder::default(), cancel))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}?limit=1000", self.url("models"));
        self.http
            .list_models(|| self.headers(self.http.client().get(&url)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_orders_tool_results_first_and_skips_reasoning() {
        let req = ChatRequest {
            model: "m".into(),
            system: Some("sys".into()),
            messages: vec![
                Message::user_text("hi"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        Part::Reasoning { text: "hmm".into() },
                        Part::ToolUse {
                            id: "t1".into(),
                            name: "x".into(),
                            input: json!({"a":1}),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![
                        Part::Text {
                            text: "note".into(),
                        },
                        Part::ToolResult {
                            tool_use_id: "t1".into(),
                            content: "out".into(),
                            is_error: true,
                        },
                    ],
                },
            ],
            tools: vec![],
            max_tokens: 100,
            temperature: None,
        };
        let b = build_body(&req);
        assert_eq!(b["system"], "sys");
        assert_eq!(b["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(b["messages"][1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(b["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(b["messages"][2]["content"][0]["is_error"], true);
        assert_eq!(b["messages"][2]["content"][1]["type"], "text");
        assert!(b.get("tools").is_none());
    }

    #[test]
    fn url_forms() {
        let a = AnthropicProvider::new("https://api.anthropic.com/v1/", None);
        assert_eq!(a.url("messages"), "https://api.anthropic.com/v1/messages");
        let d = AnthropicProvider::new("https://api.deepseek.com/anthropic", None);
        assert_eq!(
            d.url("messages"),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
    }
}
