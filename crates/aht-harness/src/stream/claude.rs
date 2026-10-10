//! Claude Code `--output-format stream-json --verbose [--include-partial-messages]` adapter.
//!
//! Lines handled (docs: https://code.claude.com/docs/en/headless, /agent-sdk/streaming-output):
//! * `stream_event` -> `content_block_delta` (`text_delta`, `thinking_delta`) => deltas
//! * `assistant` messages: `text`/`thinking` blocks (skipped if already streamed as deltas) and
//!   `tool_use` blocks => `ToolCall`
//! * `user` messages with `tool_result` blocks => `ToolResult`
//! * `result` => `Usage` (+ `Error` when `is_error`) + `Done`
//! * `system` (init, api_retry, ...) and everything else => ignored.

use super::{flatten_content, normalize_tool_name, parse_json, str_field, StreamParser};
use aht_core::agent::AgentEvent;
use serde_json::Value;

#[derive(Default)]
pub struct ClaudeStreamParser {
    /// Text/thinking of the current API message already delivered via partial-message deltas.
    streamed_text: bool,
    streamed_thinking: bool,
}

impl StreamParser for ClaudeStreamParser {
    fn feed_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Some(v) = parse_json(line) else {
            return vec![];
        };
        match str_field(&v, "type") {
            Some("stream_event") => self.stream_event(&v),
            Some("assistant") => self.assistant(&v),
            Some("user") => user(&v),
            Some("result") => result(&v),
            other => {
                tracing::debug!(?other, "claude event ignored");
                vec![]
            }
        }
    }
}

impl ClaudeStreamParser {
    fn stream_event(&mut self, v: &Value) -> Vec<AgentEvent> {
        let Some(ev) = v.get("event") else {
            return vec![];
        };
        match str_field(ev, "type") {
            Some("message_start") => {
                self.streamed_text = false;
                self.streamed_thinking = false;
                vec![]
            }
            Some("content_block_delta") => {
                let Some(d) = ev.get("delta") else {
                    return vec![];
                };
                match str_field(d, "type") {
                    Some("text_delta") => {
                        self.streamed_text = true;
                        str_field(d, "text")
                            .map(|t| AgentEvent::TextDelta { text: t.into() })
                            .into_iter()
                            .collect()
                    }
                    Some("thinking_delta") => {
                        self.streamed_thinking = true;
                        str_field(d, "thinking")
                            .map(|t| AgentEvent::ThinkingDelta { text: t.into() })
                            .into_iter()
                            .collect()
                    }
                    // input_json_delta, signature_delta: tool_use comes from `assistant`.
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }

    fn assistant(&mut self, v: &Value) -> Vec<AgentEvent> {
        let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
            return vec![];
        };
        let mut out = vec![];
        for b in blocks {
            match str_field(b, "type") {
                Some("text") if !self.streamed_text => {
                    if let Some(t) = str_field(b, "text").filter(|t| !t.is_empty()) {
                        out.push(AgentEvent::TextDelta { text: t.into() });
                    }
                }
                Some("thinking") if !self.streamed_thinking => {
                    if let Some(t) = str_field(b, "thinking").filter(|t| !t.is_empty()) {
                        out.push(AgentEvent::ThinkingDelta { text: t.into() });
                    }
                }
                Some("tool_use") => out.push(AgentEvent::ToolCall {
                    call_id: str_field(b, "id").unwrap_or_default().into(),
                    name: normalize_tool_name(str_field(b, "name").unwrap_or("unknown")),
                    input: b.get("input").cloned().unwrap_or(Value::Null),
                }),
                _ => {}
            }
        }
        out
    }
}

fn user(v: &Value) -> Vec<AgentEvent> {
    let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
        return vec![]; // plain user text (replayed prompt)
    };
    blocks
        .iter()
        .filter(|b| str_field(b, "type") == Some("tool_result"))
        .map(|b| AgentEvent::ToolResult {
            call_id: str_field(b, "tool_use_id").unwrap_or_default().into(),
            output: b.get("content").map(flatten_content).unwrap_or(Value::Null),
            is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

fn result(v: &Value) -> Vec<AgentEvent> {
    let mut out = vec![];
    if let Some(u) = v.get("usage") {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        out.push(AgentEvent::Usage {
            // Cached prompt tokens are still input the model processed; count them.
            input_tokens: n("input_tokens")
                + n("cache_creation_input_tokens")
                + n("cache_read_input_tokens"),
            output_tokens: n("output_tokens"),
            cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
        });
    }
    if v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
        || str_field(v, "subtype").is_some_and(|s| s.starts_with("error"))
    {
        let message = str_field(v, "result")
            .filter(|s| !s.is_empty())
            .or_else(|| str_field(v, "subtype"))
            .unwrap_or("Claude Code reported an error")
            .to_string();
        out.push(AgentEvent::Error {
            message,
            retryable: false,
        });
    }
    out.push(AgentEvent::Done);
    out
}
