//! `codex exec --json` adapter.
//!
//! Event types (docs: https://learn.chatgpt.com/docs/non-interactive-mode, formerly
//! developers.openai.com/codex/noninteractive): `thread.started`, `turn.started`,
//! `item.started|updated|completed` (items: `agent_message`, `reasoning`, `command_execution`,
//! `mcp_tool_call`, `file_change`, `web_search`, ...), `turn.completed{usage}`, `turn.failed`,
//! `error`. The `mcp_tool_call` item shape (`server`, `tool`, `arguments`, `result`, `error`,
//! `status`) follows the open-source codex-rs `ThreadItem` definitions [U: not shown in docs].

use super::{flatten_content, normalize_tool_name, parse_json, str_field, StreamParser};
use aht_core::agent::AgentEvent;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Default)]
pub struct CodexJsonParser {
    /// Items for which a `ToolCall` was already emitted.
    called: HashSet<String>,
    last_error: Option<String>,
}

impl StreamParser for CodexJsonParser {
    fn feed_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Some(v) = parse_json(line) else {
            return vec![];
        };
        match str_field(&v, "type") {
            Some("item.started") => self.item(&v, false),
            Some("item.completed") => self.item(&v, true),
            Some("turn.completed") => {
                let mut out = vec![];
                if let Some(u) = v.get("usage") {
                    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                    out.push(AgentEvent::Usage {
                        input_tokens: n("input_tokens"),
                        output_tokens: n("output_tokens") + n("reasoning_output_tokens"),
                        cost_usd: None,
                    });
                }
                out.push(AgentEvent::Done);
                out
            }
            Some("turn.failed") => {
                let msg = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("turn failed");
                let mut out = self.error(msg);
                out.push(AgentEvent::Done);
                out
            }
            Some("error") => {
                let msg = str_field(&v, "message").unwrap_or("error");
                self.error(msg)
            }
            other => {
                tracing::debug!(?other, "codex event ignored");
                vec![]
            }
        }
    }
}

impl CodexJsonParser {
    /// `error` is followed by `turn.failed` with the same text; report it once.
    fn error(&mut self, msg: &str) -> Vec<AgentEvent> {
        if self.last_error.as_deref() == Some(msg) {
            return vec![];
        }
        self.last_error = Some(msg.to_string());
        vec![AgentEvent::Error {
            message: msg.to_string(),
            retryable: false,
        }]
    }

    fn item(&mut self, v: &Value, completed: bool) -> Vec<AgentEvent> {
        let Some(item) = v.get("item") else {
            return vec![];
        };
        let id = str_field(item, "id").unwrap_or_default().to_string();
        let kind = str_field(item, "type").unwrap_or_default();
        let status_failed = str_field(item, "status") == Some("failed");
        match kind {
            "agent_message" if completed => str_field(item, "text")
                .filter(|t| !t.is_empty())
                .map(|t| AgentEvent::TextDelta { text: t.into() })
                .into_iter()
                .collect(),
            "reasoning" if completed => str_field(item, "text")
                .filter(|t| !t.is_empty())
                .map(|t| AgentEvent::ThinkingDelta { text: t.into() })
                .into_iter()
                .collect(),
            "mcp_tool_call" => {
                let call = AgentEvent::ToolCall {
                    call_id: id.clone(),
                    name: normalize_tool_name(str_field(item, "tool").unwrap_or("unknown")),
                    input: item.get("arguments").cloned().unwrap_or(Value::Null),
                };
                let err = item.get("error").filter(|e| !e.is_null());
                let is_error = status_failed || err.is_some();
                let output = if let Some(e) = err {
                    e.get("message").cloned().unwrap_or_else(|| e.clone())
                } else {
                    let r = item.get("result").unwrap_or(&Value::Null);
                    match r.get("structured_content").filter(|s| !s.is_null()) {
                        Some(s) => s.clone(),
                        None => r.get("content").map(flatten_content).unwrap_or(Value::Null),
                    }
                };
                self.tool(id, completed, call, output, is_error)
            }
            "command_execution" => {
                let call = AgentEvent::ToolCall {
                    call_id: id.clone(),
                    name: "shell".into(),
                    input: json!({"command": item.get("command").cloned().unwrap_or(Value::Null)}),
                };
                let code = item.get("exit_code").and_then(Value::as_i64);
                let is_error = status_failed || code.is_some_and(|c| c != 0);
                let output = json!({
                    "output": item.get("aggregated_output").cloned().unwrap_or(Value::Null),
                    "exit_code": code,
                });
                self.tool(id, completed, call, output, is_error)
            }
            "file_change" | "web_search" => {
                let call = AgentEvent::ToolCall {
                    call_id: id.clone(),
                    name: kind.into(),
                    input: item.clone(),
                };
                self.tool(id, completed, call, Value::Null, status_failed)
            }
            other => {
                tracing::debug!(other, "codex item ignored");
                vec![]
            }
        }
    }

    fn tool(
        &mut self,
        id: String,
        completed: bool,
        call: AgentEvent,
        output: Value,
        is_error: bool,
    ) -> Vec<AgentEvent> {
        let mut out = vec![];
        if self.called.insert(id.clone()) {
            out.push(call);
        }
        if completed {
            out.push(AgentEvent::ToolResult {
                call_id: id,
                output,
                is_error,
            });
        }
        out
    }
}
