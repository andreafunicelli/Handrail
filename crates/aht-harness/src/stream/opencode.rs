//! `opencode run --format json` adapter.
//!
//! Each stdout line is a bus event `{type, timestamp, sessionID, ...}` (docs:
//! https://opencode.ai/docs/cli/ ; shapes follow opencode's `run` command and `MessageV2.Part`
//! schema [U: not exhaustively documented, verify against the installed version]):
//! * `text` / `reasoning` => `part.text` (emitted when the part completes)
//! * `tool_use` => `part.{callID, tool, state{status, input, output|error}}` emitted once the tool
//!   finished, so we emit `ToolCall` + `ToolResult` together
//! * `step_finish` => `part.{reason, tokens{input,output,reasoning,cache{read,write}}, cost}`;
//!   usage is accumulated across steps and flushed with `Done` when `reason != "tool-calls"`
//! * `error` => `error.{name, data.message}`

use super::{normalize_tool_name, parse_json, str_field, StreamParser};
use aht_core::agent::AgentEvent;
use serde_json::Value;

#[derive(Default)]
pub struct OpenCodeJsonParser {
    input: u64,
    output: u64,
    cost: f64,
    saw_cost: bool,
}

impl StreamParser for OpenCodeJsonParser {
    fn feed_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Some(v) = parse_json(line) else {
            return vec![];
        };
        let part = v.get("part").unwrap_or(&Value::Null);
        match str_field(&v, "type") {
            Some("text") => text(part, false),
            Some("reasoning") => text(part, true),
            Some("tool_use") => tool(part),
            Some("step_finish") => self.step_finish(part),
            Some("error") => vec![AgentEvent::Error {
                message: v
                    .pointer("/error/data/message")
                    .and_then(Value::as_str)
                    .or_else(|| v.pointer("/error/message").and_then(Value::as_str))
                    .or_else(|| v.pointer("/error/name").and_then(Value::as_str))
                    .unwrap_or("OpenCode error")
                    .to_string(),
                retryable: false,
            }],
            other => {
                tracing::debug!(?other, "opencode event ignored");
                vec![]
            }
        }
    }
}

fn text(part: &Value, thinking: bool) -> Vec<AgentEvent> {
    match str_field(part, "text").filter(|t| !t.is_empty()) {
        Some(t) if thinking => vec![AgentEvent::ThinkingDelta { text: t.into() }],
        Some(t) => vec![AgentEvent::TextDelta { text: t.into() }],
        None => vec![],
    }
}

fn tool(part: &Value) -> Vec<AgentEvent> {
    let state = part.get("state").unwrap_or(&Value::Null);
    let call_id = str_field(part, "callID")
        .or_else(|| str_field(part, "id"))
        .unwrap_or_default()
        .to_string();
    let is_error = str_field(state, "status") == Some("error");
    let output = if is_error {
        state.get("error").cloned().unwrap_or(Value::Null)
    } else {
        state.get("output").cloned().unwrap_or(Value::Null)
    };
    vec![
        AgentEvent::ToolCall {
            call_id: call_id.clone(),
            name: normalize_tool_name(str_field(part, "tool").unwrap_or("unknown")),
            input: state.get("input").cloned().unwrap_or(Value::Null),
        },
        AgentEvent::ToolResult {
            call_id,
            output,
            is_error,
        },
    ]
}

impl OpenCodeJsonParser {
    fn step_finish(&mut self, part: &Value) -> Vec<AgentEvent> {
        let n = |p: &str| part.pointer(p).and_then(Value::as_u64).unwrap_or(0);
        self.input += n("/tokens/input") + n("/tokens/cache/read") + n("/tokens/cache/write");
        self.output += n("/tokens/output") + n("/tokens/reasoning");
        if let Some(c) = part.get("cost").and_then(Value::as_f64) {
            self.cost += c;
            self.saw_cost = true;
        }
        if str_field(part, "reason") == Some("tool-calls") {
            return vec![]; // more steps follow
        }
        let usage = AgentEvent::Usage {
            input_tokens: self.input,
            output_tokens: self.output,
            cost_usd: self.saw_cost.then_some(self.cost),
        };
        *self = Self::default();
        vec![usage, AgentEvent::Done]
    }
}
