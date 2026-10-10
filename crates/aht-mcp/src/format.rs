//! Turning tool outcomes into agent-readable text.

use aht_core::terminal::{CommandResult, CommandState};
use aht_core::Error;
use serde_json::Value;

/// Pretty JSON for a successful tool result; `run_command` gets a one-line summary first.
pub(crate) fn success_text(tool: &str, value: &Value) -> String {
    let json = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    if tool == "run_command" {
        if let Ok(r) = serde_json::from_value::<CommandResult>(value.clone()) {
            return format!("{}\n\n{json}", command_summary(&r));
        }
    }
    json
}

/// e.g. `exit 0 · 132 ms · output 2.1 KB`.
pub(crate) fn command_summary(r: &CommandResult) -> String {
    let exit = r
        .exit_code
        .map_or_else(|| "unknown".to_string(), |c| c.to_string());
    let mut s = match r.state {
        CommandState::Completed => format!("exit {exit}"),
        CommandState::TimedOut => "timed out (still running)".into(),
        CommandState::Interrupted => "interrupted".into(),
        CommandState::NeedsUser => "needs user".into(),
        CommandState::Denied => "denied by user".into(),
    };
    s.push_str(&format!(
        " · {} ms · output {}",
        r.duration_ms,
        human_size(r.output.len())
    ));
    if r.truncated {
        s.push_str(" (truncated; use read_output with output_id for the rest)");
    }
    let hint = match r.state {
        CommandState::Completed => None,
        CommandState::TimedOut => Some(
            "The command is still running: poll with read_output, or interrupt with send_keys [\"ctrl_c\"].",
        ),
        CommandState::Interrupted => {
            Some("The connection dropped or the user took over. Check session_info before continuing.")
        }
        CommandState::NeedsUser => Some(
            "The program is waiting for the user (password prompt or full-screen app). Stop and ask the user; do not try to answer it yourself.",
        ),
        CommandState::Denied => Some(
            "The user denied this command. Do not retry it unchanged; ask the user how to proceed.",
        ),
    };
    if let Some(h) = hint {
        s.push_str(" — ");
        s.push_str(h);
    }
    s
}

fn human_size(n: usize) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if f < KB * KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{:.1} MB", f / (KB * KB))
    }
}

/// Error text the agent can act on.
pub(crate) fn error_text(tool: &str, err: &Error) -> String {
    let hint = match err {
        Error::NotFound(_) => {
            "Check the id: use list_sessions / list_connections to get valid ids."
        }
        Error::Invalid(_) => "Fix the arguments to match the tool's input schema and retry.",
        Error::Denied(_) => {
            "This action is not permitted for you. Do not retry; ask the user if it is needed."
        }
        Error::Busy(_) => {
            "The session is busy. Wait for the running command (read_output) or ask the user."
        }
        Error::VaultLocked => "The user's vault is locked. Ask the user to unlock Handrail.",
        Error::Auth(_) | Error::HostKeyMismatch(_) => {
            "Connecting needs the user's attention. Stop and ask the user."
        }
        Error::Network(_) | Error::Io(_) => {
            "A transient failure may have occurred; you may retry once."
        }
        Error::Provider(_) | Error::Other(_) => "Unexpected error; report it to the user.",
    };
    format!("{tool} failed: {err}\n{hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(
        state: CommandState,
        exit: Option<i32>,
        out: usize,
        truncated: bool,
    ) -> CommandResult {
        CommandResult {
            state,
            exit_code: exit,
            output: "x".repeat(out),
            truncated,
            output_id: None,
            duration_ms: 132,
            command: "ls".into(),
            note: None,
        }
    }

    #[test]
    fn summary_completed() {
        assert_eq!(
            command_summary(&result(CommandState::Completed, Some(0), 2150, false)),
            "exit 0 · 132 ms · output 2.1 KB"
        );
        assert_eq!(
            command_summary(&result(CommandState::Completed, None, 10, false)),
            "exit unknown · 132 ms · output 10 B"
        );
    }

    #[test]
    fn summary_other_states_have_hints() {
        let s = command_summary(&result(CommandState::NeedsUser, None, 0, false));
        assert!(s.starts_with("needs user · 132 ms · output 0 B — "), "{s}");
        assert!(s.contains("ask the user"));
        let s = command_summary(&result(CommandState::TimedOut, None, 5, true));
        assert!(s.contains("truncated") && s.contains("ctrl_c"), "{s}");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn errors_have_hints() {
        let t = error_text("session_info", &Error::NotFound("session x".into()));
        assert!(t.starts_with("session_info failed: not found: session x\n"));
        assert!(t.contains("list_sessions"));
    }
}
