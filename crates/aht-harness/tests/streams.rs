//! Stream adapter tests against fixture JSONL files.
//!
//! Fixture formats are based on the documented output of each CLI:
//! * Claude Code stream-json: https://code.claude.com/docs/en/headless and
//!   https://code.claude.com/docs/en/agent-sdk/streaming-output
//! * Codex `exec --json`: https://learn.chatgpt.com/docs/non-interactive-mode
//! * OpenCode `run --format json`: https://opencode.ai/docs/cli/ (event/part shapes from the
//!   opencode source; marked unverified in the adapter docs)

use aht_core::agent::AgentEvent;
use aht_harness::{ClaudeStreamParser, CodexJsonParser, OpenCodeJsonParser, StreamParser};
use serde_json::{json, Value};

fn replay(p: &mut dyn StreamParser, fixture: &str) -> Vec<AgentEvent> {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().flat_map(|l| p.feed_line(l)).collect()
}

fn text(t: &str) -> AgentEvent {
    AgentEvent::TextDelta { text: t.into() }
}

fn call(id: &str, name: &str, input: Value) -> AgentEvent {
    AgentEvent::ToolCall {
        call_id: id.into(),
        name: name.into(),
        input,
    }
}

fn result(id: &str, output: Value, is_error: bool) -> AgentEvent {
    AgentEvent::ToolResult {
        call_id: id.into(),
        output,
        is_error,
    }
}

#[test]
fn claude_partial_messages_do_not_duplicate_text() {
    let ev = replay(&mut ClaudeStreamParser::default(), "claude_partial.jsonl");
    assert_eq!(
        ev,
        vec![
            text("Checking "),
            text("disk usage."),
            call("toolu_1", "ssh_exec", json!({"command": "df -h"})),
            result(
                "toolu_1",
                json!("Filesystem Size Used\n/dev/sda1 50G 20G"),
                false
            ),
            text("Disk is 40% used."),
            AgentEvent::Usage {
                input_tokens: 150,
                output_tokens: 42,
                cost_usd: Some(0.0123)
            },
            AgentEvent::Done,
        ]
    );
}

#[test]
fn claude_plain_messages_errors_and_unknown_lines() {
    let ev = replay(&mut ClaudeStreamParser::default(), "claude_plain.jsonl");
    assert_eq!(
        ev,
        vec![
            AgentEvent::ThinkingDelta {
                text: "Need to list files.".into()
            },
            text("Listing."),
            call("toolu_9", "Bash", json!({"command": "ls"})),
            result("toolu_9", json!("permission denied"), true),
            AgentEvent::Usage {
                input_tokens: 5,
                output_tokens: 2,
                cost_usd: None
            },
            AgentEvent::Error {
                message: "error_max_turns".into(),
                retryable: false
            },
            AgentEvent::Done,
        ]
    );
}

#[test]
fn codex_exec_items() {
    let ev = replay(&mut CodexJsonParser::default(), "codex_exec.jsonl");
    assert_eq!(
        ev,
        vec![
            AgentEvent::ThinkingDelta {
                text: "Plan: run df.".into()
            },
            call("item_1", "ssh_exec", json!({"command": "df -h"})),
            result("item_1", json!("/dev/sda1 50G 20G"), false),
            call("item_2", "ssh_exec", json!({"command": "rm -rf /"})),
            result("item_2", json!("denied by policy"), true),
            call("item_3", "shell", json!({"command": "ls"})),
            result("item_3", json!({"output": "a\nb\n", "exit_code": 2}), true),
            text("Disk is 40% used."),
            AgentEvent::Usage {
                input_tokens: 24763,
                output_tokens: 152,
                cost_usd: None
            },
            AgentEvent::Done,
        ]
    );
}

#[test]
fn codex_completed_without_started_still_emits_call() {
    let mut p = CodexJsonParser::default();
    let ev = p.feed_line(
        r#"{"type":"item.completed","item":{"id":"i","type":"mcp_tool_call","tool":"t","arguments":{},"result":{"content":[],"structured_content":{"ok":true}},"status":"completed"}}"#,
    );
    assert_eq!(
        ev,
        vec![
            call("i", "t", json!({})),
            result("i", json!({"ok": true}), false)
        ]
    );
}

#[test]
fn codex_error_is_reported_once() {
    let ev = replay(&mut CodexJsonParser::default(), "codex_failed.jsonl");
    assert_eq!(
        ev,
        vec![
            AgentEvent::Error {
                message: "stream disconnected before completion".into(),
                retryable: false
            },
            AgentEvent::Done
        ]
    );
}

#[test]
fn opencode_run_accumulates_usage_over_steps() {
    let ev = replay(&mut OpenCodeJsonParser::default(), "opencode_run.jsonl");
    assert_eq!(ev.len(), 9, "{ev:?}");
    assert_eq!(
        ev[..7],
        [
            text("Let me check the disk."),
            call("call_1", "ssh_exec", json!({"command": "df -h"})),
            result("call_1", json!("/dev/sda1 50G 20G"), false),
            call("call_2", "bash", json!({"command": "ls"})),
            result("call_2", json!("tool disabled"), true),
            AgentEvent::ThinkingDelta {
                text: "Compute percentage.".into()
            },
            text("Disk is 40% used."),
        ]
    );
    match &ev[ev.len() - 2] {
        AgentEvent::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
        } => {
            assert_eq!(*input_tokens, 100 + 50 + 120 + 10);
            assert_eq!(*output_tokens, 10 + 20 + 5);
            assert!((cost_usd.unwrap() - 0.003).abs() < 1e-9);
        }
        other => panic!("expected usage, got {other:?}"),
    }
    assert_eq!(ev.last(), Some(&AgentEvent::Done));
}

#[test]
fn opencode_error() {
    let ev = replay(&mut OpenCodeJsonParser::default(), "opencode_error.jsonl");
    assert_eq!(
        ev,
        vec![AgentEvent::Error {
            message: "Rate limit exceeded".into(),
            retryable: false
        }]
    );
}

#[test]
fn garbage_input_is_ignored() {
    for mut p in [
        Box::new(ClaudeStreamParser::default()) as Box<dyn StreamParser>,
        Box::new(CodexJsonParser::default()),
        Box::new(OpenCodeJsonParser::default()),
    ] {
        for l in [
            "",
            "   ",
            "plain text",
            "{broken",
            "[1,2]",
            r#"{"type":"zzz"}"#,
            "{}",
        ] {
            assert!(p.feed_line(l).is_empty(), "{l:?}");
        }
    }
}
