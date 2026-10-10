use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// Assistant reasoning text. Only echoed back to providers that require it (DeepSeek
    /// thinking mode with tool calls); dropped for everyone else.
    Reasoning {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Part>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![Part::Text { text: text.into() }],
        }
    }
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![Part::Text { text: text.into() }],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// Emitted once the tool call's arguments are fully received and parsed.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    Usage {
        input: u64,
        output: u64,
    },
    Stop {
        reason: StopReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
}
