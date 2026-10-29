//! The built-in agent: a tool loop that drives the user's live terminals through
//! [`aht_core::terminal::TerminalTools`], speaking to any [`aht_providers::ChatProvider`].

mod prompt;
mod truncate;

pub use truncate::cap_tool_content;

use aht_core::agent::AgentEvent;
use aht_core::terminal::{dispatch_tool, tool_specs, Actor, SessionInfo, TerminalTools};
use aht_providers::{
    estimate_cost, ChatProvider, ChatRequest, Message, Part, ProviderEvent, Role, StopReason,
    ToolDef,
};
use futures::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Default cap, in bytes, on a single tool result fed back to the model.
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 12 * 1024;

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub model: String,
    /// Maximum model calls per `run` (each call may issue several tool calls).
    pub max_steps: usize,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    /// Appended to the built-in system prompt (user/profile instructions).
    pub system_extra: Option<String>,
    pub max_tool_result_bytes: usize,
}

/// Model calls per message unless the user sets another limit.
pub const DEFAULT_MAX_STEPS: usize = 50;

/// Room for one reply, by model. Reasoning models spend part of it thinking, so they need far
/// more than chat models; asking for more than a model allows is an error, so older models with
/// small limits get theirs.
pub fn default_max_tokens(model: &str) -> u32 {
    let m = model.to_ascii_lowercase();
    let reasoning = m.contains("reasoner")
        || m.contains("-r1")
        || m.starts_with("r1")
        || m.contains("thinking")
        || ["o1", "o3", "o4", "gpt-5"]
            .iter()
            .any(|p| m.starts_with(p) || m.contains(&format!("/{p}")));
    if reasoning {
        32_768
    } else if m.contains("claude-3-haiku")
        || m.contains("claude-3-opus")
        || m.contains("claude-3-sonnet")
    {
        4_096
    } else if m.contains("claude-3-5") || m.contains("claude-3.5") {
        8_192
    } else if m.contains("claude") || m.starts_with("gpt-4.1") || m.starts_with("gpt-4o") {
        16_384
    } else {
        // DeepSeek chat, local and other OpenAI-compatible models.
        8_192
    }
}

/// The most a reply may be given when the model used its whole budget thinking.
fn max_tokens_cap(model: &str) -> u32 {
    if default_max_tokens(model) >= 32_768 {
        65_536
    } else {
        default_max_tokens(model)
    }
}

impl AgentConfig {
    pub fn new(model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            max_tokens: default_max_tokens(&model),
            model,
            max_steps: DEFAULT_MAX_STEPS,
            temperature: None,
            system_extra: None,
            max_tool_result_bytes: DEFAULT_MAX_TOOL_RESULT_BYTES,
        }
    }
}

/// Failure of a run, already classified for the UI.
struct Failure {
    error: aht_core::Error,
    retryable: bool,
}

impl From<aht_providers::Error> for Failure {
    fn from(e: aht_providers::Error) -> Self {
        Failure {
            retryable: e.retryable(),
            error: e.into(),
        }
    }
}

type ToolCallTriple = (String, String, Value);

pub struct Agent {
    provider: Arc<dyn ChatProvider>,
    tools: Arc<dyn TerminalTools>,
    actor: Actor,
    cfg: AgentConfig,
    history: Vec<Message>,
    session_hint: Option<SessionInfo>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn ChatProvider>,
        tools: Arc<dyn TerminalTools>,
        actor: Actor,
        cfg: AgentConfig,
    ) -> Self {
        Self {
            provider,
            tools,
            actor,
            cfg,
            history: Vec::new(),
            session_hint: None,
        }
    }

    /// Conversation so far (always well-formed: every tool call has a matching result).
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Model calls allowed per message, from the user's settings (at least 1).
    pub fn set_max_steps(&mut self, steps: usize) {
        self.cfg.max_steps = steps.max(1);
    }

    /// Run one user turn to completion: stream the model, execute tool calls, feed results back,
    /// repeat until the model answers without tools, `max_steps` is hit, or `cancel` fires.
    ///
    /// Always ends by sending `AgentEvent::Done`. On failure an `AgentEvent::Error` is sent
    /// first and the error is also returned. Dropping the event receiver stops the run.
    pub async fn run(
        &mut self,
        user_message: &str,
        session_hint: Option<SessionInfo>,
        events: mpsc::Sender<AgentEvent>,
        cancel: CancellationToken,
    ) -> aht_core::Result<()> {
        if session_hint.is_some() {
            self.session_hint = session_hint;
        }
        let result = self.run_inner(user_message, &events, &cancel).await;
        let out = match result {
            Ok(()) => Ok(()),
            Err(f) => {
                let _ = events
                    .send(AgentEvent::Error {
                        message: f.error.to_string(),
                        retryable: f.retryable,
                    })
                    .await;
                Err(f.error)
            }
        };
        let _ = events.send(AgentEvent::Done).await;
        out
    }

    async fn run_inner(
        &mut self,
        user_message: &str,
        events: &mpsc::Sender<AgentEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let start_len = self.history.len();
        self.history.push(Message::user_text(user_message));
        let tool_defs: Vec<ToolDef> = tool_specs()
            .into_iter()
            .map(|t| ToolDef {
                name: t.name.into(),
                description: t.description.into(),
                input_schema: t.input_schema,
            })
            .collect();
        let system = prompt::build(self.session_hint.as_ref(), self.cfg.system_extra.as_deref());

        for step in 0..self.cfg.max_steps {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let req = ChatRequest {
                model: self.cfg.model.clone(),
                system: Some(system.clone()),
                messages: self.history.clone(),
                tools: tool_defs.clone(),
                max_tokens: self.cfg.max_tokens,
                temperature: self.cfg.temperature,
            };
            let turn = match self.model_turn(req, events, cancel).await {
                Ok(t) => t,
                Err(f) => {
                    if step == 0 {
                        // Nothing happened yet: let the caller resend the same message cleanly.
                        self.history.truncate(start_len);
                    }
                    return Err(f);
                }
            };
            let aborted = turn.cancelled || turn.receiver_gone;
            // A tool call the length limit cut off cannot run. Ask again with more room (kept for
            // later turns) when the model allows it; the cut reply is not kept, the step starts
            // over. Else say what happened instead of a JSON error the user cannot act on.
            if let Some(name) = turn.cut_tool.as_deref().filter(|_| !aborted) {
                let cap = max_tokens_cap(&self.cfg.model);
                if self.cfg.max_tokens < cap {
                    self.cfg.max_tokens = self.cfg.max_tokens.saturating_mul(2).min(cap);
                    continue;
                }
                if step == 0 {
                    self.history.truncate(start_len);
                }
                return Err(Failure {
                    error: aht_core::Error::Provider(format!(
                        "the model's reply reached its length limit ({} tokens) while writing the \
                         `{name}` call, so the call was cut off and not run. It is too large for \
                         one reply of this model: ask for a smaller step (one file or one part at \
                         a time), or use a model with a larger output limit",
                        self.cfg.max_tokens
                    )),
                    retryable: false,
                });
            }
            let answered = !turn.text.is_empty();
            // A cancelled stream may hold tool calls we cannot trust; drop them.
            let calls = if aborted { Vec::new() } else { turn.tool_uses };
            if !turn.text.is_empty() || !calls.is_empty() {
                let mut parts = Vec::new();
                if !turn.reasoning.is_empty() {
                    parts.push(Part::Reasoning {
                        text: turn.reasoning,
                    });
                }
                if !turn.text.is_empty() {
                    parts.push(Part::Text { text: turn.text });
                }
                for (id, name, input) in &calls {
                    parts.push(Part::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
                self.history.push(Message {
                    role: Role::Assistant,
                    content: parts,
                });
            }
            if aborted {
                return Ok(());
            }
            if calls.is_empty() {
                if turn.stop == StopReason::MaxTokens {
                    if !answered {
                        // All of it went on thinking: ask again with more room (kept for later
                        // turns), up to what the model allows.
                        let cap = max_tokens_cap(&self.cfg.model);
                        if self.cfg.max_tokens < cap {
                            self.cfg.max_tokens = self.cfg.max_tokens.saturating_mul(2).min(cap);
                            continue;
                        }
                        return Err(Failure {
                            error: aht_core::Error::Provider(format!(
                                "the model spent its whole reply budget ({} tokens) thinking \
                                 without answering; try again, or ask for a smaller step",
                                self.cfg.max_tokens
                            )),
                            retryable: true,
                        });
                    }
                    // A long answer: keep what arrived and say how to get the rest.
                    let note =
                        "\n\n_(The reply hit the length limit. Say \"continue\" for the rest.)_";
                    let _ = events
                        .send(AgentEvent::TextDelta { text: note.into() })
                        .await;
                }
                return Ok(());
            }
            if !self.run_tools(calls, events, cancel).await {
                return Ok(());
            }
        }

        // Step budget exhausted while the model still wanted tools.
        let note = format!(
            "I stopped after {} steps without finishing. Continue to let me go on.",
            self.cfg.max_steps
        );
        self.history.push(Message::assistant_text(note));
        let _ = events
            .send(AgentEvent::StepLimit {
                steps: self.cfg.max_steps as u32,
            })
            .await;
        Ok(())
    }

    /// Stream one model response, forwarding deltas as events.
    async fn model_turn(
        &self,
        req: ChatRequest,
        events: &mpsc::Sender<AgentEvent>,
        cancel: &CancellationToken,
    ) -> Result<Turn, Failure> {
        let mut turn = Turn::default();
        let mut stream = tokio::select! {
            _ = cancel.cancelled() => { turn.cancelled = true; return Ok(turn); }
            s = self.provider.stream(req, cancel.clone()) => s?,
        };
        loop {
            let item = tokio::select! {
                _ = cancel.cancelled() => { turn.cancelled = true; break; }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            let ev = match item {
                Ok(ev) => ev,
                Err(e) => {
                    if cancel.is_cancelled() {
                        turn.cancelled = true;
                        break;
                    }
                    return Err(e.into());
                }
            };
            let out = match ev {
                ProviderEvent::TextDelta(t) => {
                    turn.text.push_str(&t);
                    Some(AgentEvent::TextDelta { text: t })
                }
                ProviderEvent::ThinkingDelta(t) => {
                    turn.reasoning.push_str(&t);
                    Some(AgentEvent::ThinkingDelta { text: t })
                }
                ProviderEvent::ToolUse { id, name, input } => {
                    turn.tool_uses.push((id, name, input));
                    None
                }
                ProviderEvent::ToolUseCut { name } => {
                    turn.cut_tool = Some(name);
                    None
                }
                ProviderEvent::Usage { input, output } => Some(AgentEvent::Usage {
                    input_tokens: input,
                    output_tokens: output,
                    cost_usd: estimate_cost(&self.cfg.model, input, output),
                }),
                ProviderEvent::Stop { reason } => {
                    turn.stop = reason;
                    None
                }
            };
            if let Some(out) = out {
                if events.send(out).await.is_err() {
                    turn.receiver_gone = true;
                    break;
                }
            }
        }
        Ok(turn)
    }

    /// Execute tool calls sequentially and append one user message holding all results.
    /// Returns false when the run must stop (cancelled or receiver gone).
    async fn run_tools(
        &mut self,
        calls: Vec<ToolCallTriple>,
        events: &mpsc::Sender<AgentEvent>,
        cancel: &CancellationToken,
    ) -> bool {
        let mut results: Vec<Part> = Vec::new();
        let mut keep_going = true;
        for (id, name, input) in calls {
            if !keep_going {
                // Aborted earlier in this batch: still answer every call to keep history valid.
                results.push(Part::ToolResult {
                    tool_use_id: id,
                    content: "Cancelled by the user before this tool ran.".into(),
                    is_error: true,
                });
                continue;
            }
            let call = AgentEvent::ToolCall {
                call_id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            };
            if events.send(call).await.is_err() {
                keep_going = false;
                results.push(Part::ToolResult {
                    tool_use_id: id,
                    content: "Cancelled before this tool ran.".into(),
                    is_error: true,
                });
                continue;
            }
            let outcome = tokio::select! {
                _ = cancel.cancelled() => None,
                r = dispatch_tool(self.tools.as_ref(), &self.actor, &name, input) => Some(r),
            };
            let (output, is_error, content) = match outcome {
                None => {
                    keep_going = false;
                    let msg = "Cancelled by the user while this tool was running; the command \
may still be running in the terminal.";
                    (json!({"error": msg}), true, msg.to_string())
                }
                Some(Ok(v)) => {
                    let content = cap_tool_content(&v, self.cfg.max_tool_result_bytes);
                    (v, false, content)
                }
                Some(Err(e)) => {
                    let msg = format!("Error: {e}");
                    (json!({"error": e.to_string()}), true, msg)
                }
            };
            let sent = events
                .send(AgentEvent::ToolResult {
                    call_id: id.clone(),
                    output,
                    is_error,
                })
                .await;
            if sent.is_err() {
                keep_going = false;
            }
            results.push(Part::ToolResult {
                tool_use_id: id,
                content,
                is_error,
            });
        }
        self.history.push(Message {
            role: Role::User,
            content: results,
        });
        keep_going
    }
}

struct Turn {
    text: String,
    reasoning: String,
    tool_uses: Vec<ToolCallTriple>,
    /// A tool call the length limit cut off (its name): it cannot run.
    cut_tool: Option<String>,
    stop: StopReason,
    receiver_gone: bool,
    cancelled: bool,
}

impl Default for Turn {
    fn default() -> Self {
        Self {
            text: String::new(),
            reasoning: String::new(),
            tool_uses: Vec::new(),
            cut_tool: None,
            stop: StopReason::EndTurn,
            receiver_gone: false,
            cancelled: false,
        }
    }
}
