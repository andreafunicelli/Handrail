//! OpenAI Chat Completions adapter (SSE). Used for OpenAI and every OpenAI-compatible preset
//! (DeepSeek, OpenRouter, Ollama, LM Studio, OpenCode non-Claude models, custom).

use crate::error::{Error, Result};
use crate::http::{parse_tool_args, pump, Decoder, Http};
use crate::sse::SseEvent;
use crate::types::*;
use crate::ChatProvider;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct OpenAiChatProvider {
    http: Http,
    base_url: String,
    api_key: Option<String>,
    /// OpenAI proper wants `max_completion_tokens`; compatible servers want `max_tokens`.
    max_completion_tokens: bool,
    /// Echo assistant `reasoning_content` back (DeepSeek thinking mode + tool calls).
    echo_reasoning: bool,
    /// Sent as `x-opencode-session` (OpenCode Zen/Go gateways).
    opencode_session: Option<String>,
}

impl OpenAiChatProvider {
    /// `base_url` is the API root, e.g. `https://api.openai.com/v1` or `https://api.deepseek.com`.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            http: Http::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            max_completion_tokens: false,
            echo_reasoning: false,
            opencode_session: None,
        }
    }

    /// Send `session` as the `x-opencode-session` header on every request.
    pub fn with_opencode_session(mut self, session: impl Into<String>) -> Self {
        self.opencode_session = Some(session.into());
        self
    }

    pub fn with_max_completion_tokens(mut self, yes: bool) -> Self {
        self.max_completion_tokens = yes;
        self
    }

    pub fn with_echo_reasoning(mut self, yes: bool) -> Self {
        self.echo_reasoning = yes;
        self
    }

    pub fn with_retry_backoff(mut self, d: Duration) -> Self {
        self.http.retry_backoff = d;
        self
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = match &self.opencode_session {
            Some(s) => rb.header(crate::http::OPENCODE_SESSION_HEADER, s),
            None => rb,
        };
        match &self.api_key {
            Some(k) if !k.is_empty() => rb.bearer_auth(k),
            _ => rb,
        }
    }
}

pub(crate) fn build_body(
    req: &ChatRequest,
    max_completion_tokens: bool,
    echo_reasoning: bool,
) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(s) = req.system.as_ref().filter(|s| !s.is_empty()) {
        messages.push(json!({"role":"system","content":s}));
    }
    for m in &req.messages {
        match m.role {
            Role::User => {
                let mut text = String::new();
                for p in &m.content {
                    match p {
                        Part::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => messages.push(
                            json!({"role":"tool","tool_call_id":tool_use_id,"content":content}),
                        ),
                        Part::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({"role":"user","content":text}));
                }
            }
            Role::Assistant => {
                let mut text = String::new();
                let mut reasoning = String::new();
                let mut calls = Vec::new();
                for p in &m.content {
                    match p {
                        Part::Text { text: t } => text.push_str(t),
                        Part::Reasoning { text: t } => reasoning.push_str(t),
                        Part::ToolUse { id, name, input } => calls.push(json!({
                            "id": id, "type": "function",
                            "function": {"name": name, "arguments": input.to_string()}
                        })),
                        _ => {}
                    }
                }
                if text.is_empty() && calls.is_empty() {
                    continue;
                }
                let mut msg = json!({"role":"assistant","content": if text.is_empty() { Value::Null } else { json!(text) }});
                if !calls.is_empty() {
                    msg["tool_calls"] = json!(calls);
                    if echo_reasoning && !reasoning.is_empty() {
                        msg["reasoning_content"] = json!(reasoning);
                    }
                }
                messages.push(msg);
            }
        }
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    body[if max_completion_tokens {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }] = json!(req.max_tokens);
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| {
                json!({"type":"function","function":{
                "name":t.name,"description":t.description,"parameters":t.input_schema}})
            })
            .collect();
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

#[derive(Default)]
struct ToolAcc {
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
pub(crate) struct OpenAiDecoder {
    tools: BTreeMap<u64, ToolAcc>,
    finish: Option<String>,
    usage: Option<(u64, u64)>,
    had_tools: bool,
    sent_stop: bool,
}

impl OpenAiDecoder {
    fn flush(&mut self, out: &mut Vec<ProviderEvent>) -> Result<()> {
        for (idx, t) in std::mem::take(&mut self.tools) {
            let input = match parse_tool_args(&t.args) {
                Ok(v) => v,
                // The reply hit its length limit mid-arguments: report the cut, not bad JSON.
                Err(_) if self.finish.as_deref() == Some("length") => {
                    out.push(ProviderEvent::ToolUseCut { name: t.name });
                    continue;
                }
                Err(e) => return Err(e),
            };
            out.push(ProviderEvent::ToolUse {
                id: if t.id.is_empty() {
                    format!("call_{idx}")
                } else {
                    t.id
                },
                name: t.name,
                input,
            });
        }
        Ok(())
    }

    fn stop(&mut self, out: &mut Vec<ProviderEvent>) {
        let had_tools = self.had_tools;
        if let Some((input, output)) = self.usage.take() {
            out.push(ProviderEvent::Usage { input, output });
        }
        let reason = match self.finish.as_deref() {
            Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            // Some servers report "stop" even when tool calls were made.
            Some("stop") | None if had_tools => StopReason::ToolUse,
            Some("stop") | None => StopReason::EndTurn,
            Some(_) => StopReason::Other,
        };
        self.sent_stop = true;
        out.push(ProviderEvent::Stop { reason });
    }
}

impl Decoder for OpenAiDecoder {
    fn feed(&mut self, ev: &SseEvent) -> Result<Vec<ProviderEvent>> {
        let data = ev.data.trim();
        let mut out = Vec::new();
        if data.is_empty() {
            return Ok(out);
        }
        if data == "[DONE]" {
            self.flush(&mut out)?;
            self.stop(&mut out);
            return Ok(out);
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| Error::Protocol(format!("bad SSE json: {e}")))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let m = err["message"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| err.to_string());
            return Err(Error::Server {
                status: 500,
                message: m,
            });
        }
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some((
                u["prompt_tokens"].as_u64().unwrap_or(0),
                u["completion_tokens"].as_u64().unwrap_or(0),
            ));
        }
        let Some(choice) = v["choices"].get(0) else {
            return Ok(out);
        };
        let delta = &choice["delta"];
        for key in ["reasoning_content", "reasoning"] {
            if let Some(t) = delta[key].as_str().filter(|t| !t.is_empty()) {
                out.push(ProviderEvent::ThinkingDelta(t.into()));
                break;
            }
        }
        if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            out.push(ProviderEvent::TextDelta(t.into()));
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            self.had_tools |= !calls.is_empty();
            for (pos, c) in calls.iter().enumerate() {
                let idx = c["index"].as_u64().unwrap_or(pos as u64);
                let acc = self.tools.entry(idx).or_default();
                if let Some(id) = c["id"].as_str().filter(|s| !s.is_empty()) {
                    acc.id = id.into();
                }
                if let Some(n) = c["function"]["name"].as_str() {
                    acc.name.push_str(n);
                }
                match &c["function"]["arguments"] {
                    Value::String(s) => acc.args.push_str(s),
                    Value::Object(_) => acc.args = c["function"]["arguments"].to_string(),
                    _ => {}
                }
            }
        }
        if let Some(fr) = choice["finish_reason"].as_str() {
            self.finish = Some(fr.to_string());
            // Tool calls are complete once the finish reason arrives.
            self.flush(&mut out)?;
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<ProviderEvent>> {
        if self.sent_stop {
            return Ok(vec![]);
        }
        // Stream closed without [DONE]: accept it only if the model signalled completion.
        if self.finish.is_none() {
            return Err(Error::Protocol("stream ended before finish_reason".into()));
        }
        let mut out = Vec::new();
        self.flush(&mut out)?;
        self.stop(&mut out);
        Ok(out)
    }
}

#[async_trait]
impl ChatProvider for OpenAiChatProvider {
    async fn stream(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ProviderEvent>>> {
        let body = build_body(&req, self.max_completion_tokens, self.echo_reasoning);
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .http
            .send(
                || {
                    self.auth(self.http.client().post(&url))
                        .header("accept", "text/event-stream")
                        .json(&body)
                },
                &cancel,
            )
            .await?;
        Ok(pump(resp, OpenAiDecoder::default(), cancel))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/models", self.base_url);
        self.http
            .list_models(|| self.auth(self.http.client().get(&url)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_maps_tool_roundtrip() {
        let req = ChatRequest {
            model: "m".into(),
            system: Some("sys".into()),
            messages: vec![
                Message::user_text("hi"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        Part::Reasoning {
                            text: "think".into(),
                        },
                        Part::ToolUse {
                            id: "c1".into(),
                            name: "run".into(),
                            input: json!({"cmd":"ls"}),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![Part::ToolResult {
                        tool_use_id: "c1".into(),
                        content: "a b".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![ToolDef {
                name: "run".into(),
                description: "d".into(),
                input_schema: json!({"type":"object"}),
            }],
            max_tokens: 50,
            temperature: Some(0.2),
        };
        let b = build_body(&req, false, true);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(
            b["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{\"cmd\":\"ls\"}"
        );
        assert!(b["messages"][2]["content"].is_null());
        assert_eq!(b["messages"][2]["reasoning_content"], "think");
        assert_eq!(b["messages"][3]["role"], "tool");
        assert_eq!(b["messages"][3]["tool_call_id"], "c1");
        assert_eq!(b["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert_eq!(b["max_tokens"], 50);
        let b2 = build_body(&req, true, false);
        assert_eq!(b2["max_completion_tokens"], 50);
        assert!(b2["messages"][2].get("reasoning_content").is_none());
    }
}
