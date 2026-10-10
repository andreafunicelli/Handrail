//! Adapters from each harness's JSONL stdout to [`aht_core::agent::AgentEvent`].
//!
//! Every parser is line-oriented and tolerant: blank lines, non-JSON noise and unknown event types
//! are ignored (logged at debug level), never fatal.

mod claude;
mod codex;
mod opencode;

pub use claude::ClaudeStreamParser;
pub use codex::CodexJsonParser;
pub use opencode::OpenCodeJsonParser;

use aht_core::agent::AgentEvent;
use aht_core::model::HarnessKind;
use serde_json::Value;

pub trait StreamParser: Send {
    /// Feed one stdout line (without trailing newline); returns the events it produced.
    fn feed_line(&mut self, line: &str) -> Vec<AgentEvent>;
}

/// Parser for a harness kind.
pub fn parser_for(kind: HarnessKind) -> Box<dyn StreamParser> {
    match kind {
        HarnessKind::ClaudeCode => Box::new(ClaudeStreamParser::default()),
        HarnessKind::Codex => Box::new(CodexJsonParser::default()),
        HarnessKind::OpenCode => Box::new(OpenCodeJsonParser::default()),
    }
}

/// Strip MCP namespacing so the UI sees the bare tool name (`mcp__handrail__ssh_exec` and
/// OpenCode's `handrail_ssh_exec` both become `ssh_exec`).
pub fn normalize_tool_name(name: &str) -> String {
    let n = name.strip_prefix("mcp__handrail__").unwrap_or(name);
    let n = n.strip_prefix("handrail_").unwrap_or(n);
    n.to_string()
}

pub(crate) fn parse_json(line: &str) -> Option<Value> {
    let t = line.trim();
    if !t.starts_with('{') {
        if !t.is_empty() {
            tracing::debug!(line = t, "non-JSON line from harness ignored");
        }
        return None;
    }
    match serde_json::from_str(t) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::debug!(error = %e, "unparseable JSON line from harness ignored");
            None
        }
    }
}

/// Collapse MCP/tool result content (`"text"` or `[{type:"text",text:"..."}]`) into a string when
/// it is purely text; otherwise return it unchanged.
pub(crate) fn flatten_content(v: &Value) -> Value {
    match v {
        Value::Array(items)
            if !items.is_empty()
                && items
                    .iter()
                    .all(|i| i.get("type").and_then(Value::as_str) == Some("text")) =>
        {
            Value::String(
                items
                    .iter()
                    .filter_map(|i| i.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        other => other.clone(),
    }
}

pub(crate) fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names() {
        assert_eq!(normalize_tool_name("mcp__handrail__ssh_exec"), "ssh_exec");
        assert_eq!(normalize_tool_name("handrail_ssh_exec"), "ssh_exec");
        assert_eq!(normalize_tool_name("Bash"), "Bash");
        assert_eq!(normalize_tool_name("mcp__other__x"), "mcp__other__x");
    }

    #[test]
    fn flatten() {
        let v = serde_json::json!([{"type":"text","text":"a"},{"type":"text","text":"b"}]);
        assert_eq!(flatten_content(&v), Value::String("a\nb".into()));
        let img = serde_json::json!([{"type":"image"}]);
        assert_eq!(flatten_content(&img), img);
    }
}
