//! Normalized agent events rendered by the AI sidebar. Produced by the built-in agent and by
//! harness stream adapters (Claude Code / Codex / OpenCode), so the UI renders one format.

use crate::{ApprovalId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streaming assistant text.
    TextDelta {
        text: String,
    },
    /// Streaming reasoning/thinking text, if the model exposes it.
    ThinkingDelta {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        output: Value,
        is_error: bool,
    },
    ApprovalRequested(ApprovalRequest),
    ApprovalResolved {
        approval_id: ApprovalId,
        decision: ApprovalDecision,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
    },
    Error {
        message: String,
        retryable: bool,
    },
    /// The run used all its model calls while the model still had work to do; the user can
    /// let it continue.
    StepLimit {
        steps: u32,
    },
    /// Turn finished.
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Read,
    Write,
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub approval_id: ApprovalId,
    pub session_id: SessionId,
    pub actor_label: String,
    pub command: String,
    pub reason: Option<String>,
    pub risk: RiskClass,
    /// Why approval is required, e.g. "matches deny-list: rm -rf".
    pub policy_note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    /// User edited the command before approving.
    ApproveEdited {
        command: String,
    },
    Deny {
        message: Option<String>,
    },
}
