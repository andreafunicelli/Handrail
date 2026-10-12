use aht_agent::{Agent, AgentConfig};
use aht_core::agent::AgentEvent;
use aht_core::model::{ApprovalMode, Environment};
use aht_core::terminal::*;
use aht_core::{ConnectionId, Error, OutputId, Result, SessionId};
use aht_providers::{
    ChatProvider, ChatRequest, Message, ModelInfo, Part, ProviderEvent, Role, StopReason,
};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// ------------------------------------------------------------ ScriptedProvider

enum Turn {
    Events(Vec<ProviderEvent>),
    /// Emit these events, then hang forever (until cancelled).
    Hang(Vec<ProviderEvent>),
    /// `stream()` itself fails.
    StartError(aht_providers::Error),
}

struct ScriptedProvider {
    turns: Mutex<VecDeque<Turn>>,
    requests: Mutex<Vec<ChatRequest>>,
    /// Replayed forever once the script is exhausted (for max_steps tests).
    repeat: Mutex<Option<Vec<ProviderEvent>>>,
}

impl ScriptedProvider {
    fn new(turns: Vec<Turn>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into()),
            requests: Mutex::default(),
            repeat: Mutex::default(),
        })
    }
    fn repeating(events: Vec<ProviderEvent>) -> Arc<Self> {
        let p = Self::new(vec![]);
        *p.repeat.lock().unwrap() = Some(events);
        p
    }
    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    async fn stream(
        &self,
        req: ChatRequest,
        _cancel: CancellationToken,
    ) -> aht_providers::Result<BoxStream<'static, aht_providers::Result<ProviderEvent>>> {
        self.requests.lock().unwrap().push(req);
        let turn = self.turns.lock().unwrap().pop_front();
        let turn = match turn {
            Some(t) => t,
            None => Turn::Events(
                self.repeat
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("script exhausted"),
            ),
        };
        match turn {
            Turn::Events(evs) => Ok(stream::iter(evs.into_iter().map(Ok)).boxed()),
            Turn::Hang(evs) => Ok(stream::iter(evs.into_iter().map(Ok))
                .chain(stream::pending())
                .boxed()),
            Turn::StartError(e) => Err(e),
        }
    }

    async fn list_models(&self) -> aht_providers::Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
}

fn text(t: &str) -> ProviderEvent {
    ProviderEvent::TextDelta(t.into())
}
fn tool(id: &str, name: &str, input: Value) -> ProviderEvent {
    ProviderEvent::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }
}
fn stop(r: StopReason) -> ProviderEvent {
    ProviderEvent::Stop { reason: r }
}
fn answer(t: &str) -> Turn {
    Turn::Events(vec![text(t), stop(StopReason::EndTurn)])
}
fn calls(evs: Vec<ProviderEvent>) -> Turn {
    let mut evs = evs;
    evs.push(stop(StopReason::ToolUse));
    Turn::Events(evs)
}

// ------------------------------------------------------------------- MockTools

#[derive(Default)]
struct MockTools {
    run_results: Mutex<VecDeque<Result<CommandResult>>>,
    commands: Mutex<Vec<String>>,
    hang_on_run: bool,
}

fn session() -> SessionInfo {
    SessionInfo {
        session_id: SessionId::new(),
        connection_id: None,
        title: "prod-web-1".into(),
        env: Environment::Prod,
        os: Some("Ubuntu 24.04".into()),
        shell: Some("bash".into()),
        cwd: Some("/srv/app".into()),
        user: Some("deploy".into()),
        host: Some("web1.example.com".into()),
        integration: IntegrationMode::Full,
        state: SessionState::Idle,
        approval_mode: ApprovalMode::ConfirmAll,
        ended: None,
    }
}

fn cmd_result(output: &str) -> CommandResult {
    CommandResult {
        state: CommandState::Completed,
        exit_code: Some(0),
        output: output.into(),
        truncated: false,
        output_id: Some(OutputId::new()),
        duration_ms: 12,
        command: "x".into(),
        note: None,
    }
}

#[async_trait]
impl TerminalTools for MockTools {
    async fn list_connections(&self, _: &Actor) -> Result<Vec<ConnectionSummary>> {
        Ok(vec![])
    }
    async fn open_session(&self, _: &Actor, _: ConnectionId) -> Result<SessionInfo> {
        Err(Error::Other("unused".into()))
    }
    async fn list_sessions(&self, _: &Actor) -> Result<Vec<SessionInfo>> {
        Ok(vec![session()])
    }
    async fn session_info(&self, _: &Actor, _: SessionId) -> Result<SessionInfo> {
        Ok(session())
    }
    async fn run_command(&self, _: &Actor, req: RunCommandRequest) -> Result<CommandResult> {
        self.commands.lock().unwrap().push(req.command.clone());
        if self.hang_on_run {
            futures::future::pending::<()>().await;
        }
        self.run_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(cmd_result("ok")))
    }
    async fn read_output(&self, _: &Actor, _: OutputId, _: usize, _: usize) -> Result<OutputChunk> {
        Err(Error::Other("unused".into()))
    }
    async fn send_keys(&self, _: &Actor, _: SessionId, _: Vec<ControlKey>) -> Result<()> {
        Ok(())
    }
}

// --------------------------------------------------------------------- helpers

fn agent(p: Arc<ScriptedProvider>, t: Arc<MockTools>, cfg: AgentConfig) -> Agent {
    Agent::new(
        p,
        t,
        Actor {
            id: "builtin:test".into(),
            label: "Test".into(),
            session_scope: None,
        },
        cfg,
    )
}

fn cfg() -> AgentConfig {
    AgentConfig::new("test-model")
}

async fn run(a: &mut Agent, msg: &str) -> (aht_core::Result<()>, Vec<AgentEvent>) {
    run_with(a, msg, CancellationToken::new()).await
}

async fn run_with(
    a: &mut Agent,
    msg: &str,
    cancel: CancellationToken,
) -> (aht_core::Result<()>, Vec<AgentEvent>) {
    let (tx, mut rx) = mpsc::channel(1024);
    let r = a.run(msg, None, tx, cancel).await;
    let mut evs = Vec::new();
    while let Some(e) = rx.recv().await {
        evs.push(e);
    }
    (r, evs)
}

/// Every assistant tool call is answered by a tool result in the very next message.
fn assert_history_well_formed(h: &[Message]) {
    for (i, m) in h.iter().enumerate() {
        let ids: Vec<&String> = m
            .content
            .iter()
            .filter_map(|p| match p {
                Part::ToolUse { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            continue;
        }
        assert_eq!(m.role, Role::Assistant);
        let next = h
            .get(i + 1)
            .unwrap_or_else(|| panic!("tool call without result: {h:#?}"));
        assert_eq!(next.role, Role::User);
        for id in ids {
            assert!(
                next.content.iter().any(
                    |p| matches!(p, Part::ToolResult { tool_use_id, .. } if tool_use_id == id)
                ),
                "no result for {id}"
            );
        }
    }
}

fn kinds(evs: &[AgentEvent]) -> Vec<&'static str> {
    evs.iter()
        .map(|e| match e {
            AgentEvent::TextDelta { .. } => "text",
            AgentEvent::ThinkingDelta { .. } => "thinking",
            AgentEvent::ToolCall { .. } => "call",
            AgentEvent::ToolResult { .. } => "result",
            AgentEvent::ApprovalRequested(_) => "approval_req",
            AgentEvent::ApprovalResolved { .. } => "approval_res",
            AgentEvent::Usage { .. } => "usage",
            AgentEvent::Error { .. } => "error",
            AgentEvent::StepLimit { .. } => "step_limit",
            AgentEvent::Done => "done",
        })
        .collect()
}

fn tool_result_parts(m: &Message) -> Vec<(&str, &str, bool)> {
    m.content
        .iter()
        .filter_map(|p| match p {
            Part::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some((tool_use_id.as_str(), content.as_str(), *is_error)),
            _ => None,
        })
        .collect()
}

// ----------------------------------------------------------------------- tests

#[tokio::test]
async fn multi_step_loop_runs_tools_and_finishes() {
    let sid = SessionId::new().to_string();
    let provider = ScriptedProvider::new(vec![
        calls(vec![
            ProviderEvent::ThinkingDelta("hmm".into()),
            text("Checking sessions. "),
            tool("t1", "list_sessions", json!({})),
            ProviderEvent::Usage {
                input: 10,
                output: 5,
            },
        ]),
        calls(vec![tool(
            "t2",
            "run_command",
            json!({"session_id": sid, "command": "uptime", "reason": "check load"}),
        )]),
        answer("Load is fine."),
    ]);
    let tools = Arc::new(MockTools::default());
    let mut a = agent(provider.clone(), tools.clone(), cfg());
    let (r, evs) = run(&mut a, "how is the server?").await;
    r.unwrap();

    assert_eq!(
        kinds(&evs),
        ["thinking", "text", "usage", "call", "result", "call", "result", "text", "done"]
    );
    assert_eq!(*tools.commands.lock().unwrap(), ["uptime"]);
    match &evs[4] {
        AgentEvent::ToolResult {
            call_id,
            is_error,
            output,
        } => {
            assert_eq!(call_id, "t1");
            assert!(!is_error);
            assert!(output.is_array());
        }
        e => panic!("{e:?}"),
    }

    // Requests: system prompt, all 7 tools, growing history.
    let reqs = provider.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[0].model, "test-model");
    assert_eq!(reqs[0].tools.len(), tool_specs().len());
    let sys = reqs[0].system.as_ref().unwrap();
    assert!(
        sys.contains("LIVE terminal") && sys.contains("needs_user") && sys.contains("password")
    );
    assert_eq!(reqs[0].messages.len(), 1);
    assert_eq!(reqs[1].messages.len(), 3);
    assert_eq!(reqs[2].messages.len(), 5);
    // History keeps reasoning + text + tool use of step 1.
    assert!(matches!(
        &reqs[1].messages[1].content[0],
        Part::Reasoning { .. }
    ));
    assert_history_well_formed(a.history());
    assert_eq!(a.history().len(), 6);
}

#[tokio::test]
async fn usage_event_carries_estimated_cost() {
    let provider = ScriptedProvider::new(vec![Turn::Events(vec![
        text("hi"),
        ProviderEvent::Usage {
            input: 1_000_000,
            output: 0,
        },
        stop(StopReason::EndTurn),
    ])]);
    let mut c = cfg();
    c.model = "claude-sonnet-5-5".into();
    let mut a = agent(provider, Arc::new(MockTools::default()), c);
    let (_, evs) = run(&mut a, "x").await;
    let cost = evs.iter().find_map(|e| match e {
        AgentEvent::Usage {
            cost_usd,
            input_tokens,
            ..
        } => {
            assert_eq!(*input_tokens, 1_000_000);
            Some(*cost_usd)
        }
        _ => None,
    });
    assert_eq!(cost, Some(Some(3.0)));
}

#[tokio::test]
async fn tool_errors_become_error_results_and_agent_recovers() {
    let sid = SessionId::new().to_string();
    let provider = ScriptedProvider::new(vec![
        // bad arguments -> Error::Invalid; unknown tool -> NotFound; runtime failure -> Denied
        calls(vec![
            tool(
                "a",
                "run_command",
                json!({"session_id": "not-a-uuid", "command": "ls"}),
            ),
            tool("b", "no_such_tool", json!({})),
            tool(
                "c",
                "run_command",
                json!({"session_id": sid, "command": "rm x"}),
            ),
        ]),
        calls(vec![tool(
            "d",
            "run_command",
            json!({"session_id": sid, "command": "ls"}),
        )]),
        answer("recovered"),
    ]);
    let tools = Arc::new(MockTools::default());
    tools
        .run_results
        .lock()
        .unwrap()
        .push_back(Err(Error::Denied("policy".into())));
    let mut a = agent(provider.clone(), tools.clone(), cfg());
    let (r, evs) = run(&mut a, "go").await;
    r.unwrap();

    let results: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult {
                call_id, is_error, ..
            } => Some((call_id.as_str(), *is_error)),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        [("a", true), ("b", true), ("c", true), ("d", false)]
    );

    let reqs = provider.requests();
    let fed = tool_result_parts(&reqs[1].messages[2]);
    assert_eq!(fed.len(), 3);
    assert!(fed.iter().all(|(_, _, err)| *err));
    assert!(fed[0].1.starts_with("Error: invalid input"));
    assert!(fed[1].1.contains("not found"));
    assert!(fed[2].1.contains("permission denied: policy"));
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "recovered")));
    assert_history_well_formed(a.history());
}

#[tokio::test]
async fn needs_user_result_is_passed_through_not_treated_as_error() {
    let sid = SessionId::new().to_string();
    let provider = ScriptedProvider::new(vec![
        calls(vec![tool(
            "a",
            "run_command",
            json!({"session_id": sid, "command": "sudo ls"}),
        )]),
        answer("Please type your sudo password in the terminal."),
    ]);
    let tools = Arc::new(MockTools::default());
    let mut r = cmd_result("[sudo] password for deploy:");
    r.state = CommandState::NeedsUser;
    r.exit_code = None;
    tools.run_results.lock().unwrap().push_back(Ok(r));
    let mut a = agent(provider.clone(), tools, cfg());
    run(&mut a, "x").await.0.unwrap();
    let fed = tool_result_parts(&provider.requests()[1].messages[2])[0]
        .1
        .to_string();
    assert!(fed.contains("needs_user"));
    assert!(!tool_result_parts(&provider.requests()[1].messages[2])[0].2);
}

#[tokio::test]
async fn max_steps_stops_with_message_and_valid_history() {
    let provider = ScriptedProvider::repeating(vec![
        tool("loop", "list_sessions", json!({})),
        stop(StopReason::ToolUse),
    ]);
    let mut c = cfg();
    c.max_steps = 3;
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), c);
    let (r, evs) = run(&mut a, "go").await;
    r.unwrap();
    assert_eq!(provider.requests().len(), 3);
    let k = kinds(&evs);
    assert_eq!(k.iter().filter(|k| **k == "call").count(), 3);
    assert_eq!(k[k.len() - 2..], ["step_limit", "done"]);
    assert_eq!(evs[evs.len() - 2], AgentEvent::StepLimit { steps: 3 });
    assert_history_well_formed(a.history());
    assert_eq!(a.history().last().unwrap().role, Role::Assistant);
}

#[tokio::test]
async fn cancel_while_streaming_stops_and_keeps_history_valid() {
    let provider = ScriptedProvider::new(vec![
        Turn::Hang(vec![text("partial ")]),
        answer("second run ok"),
    ]);
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), cfg());
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let (tx, mut rx) = mpsc::channel(64);
    let waiter = tokio::spawn(async move {
        let mut evs = vec![];
        while let Some(e) = rx.recv().await {
            if matches!(e, AgentEvent::TextDelta { .. }) {
                c2.cancel();
            }
            evs.push(e);
        }
        evs
    });
    a.run("go", None, tx, cancel).await.unwrap();
    let evs = waiter.await.unwrap();
    assert_eq!(kinds(&evs), ["text", "done"]);
    assert_history_well_formed(a.history());

    // The agent is reusable afterwards.
    let (r, evs) = run(&mut a, "again").await;
    r.unwrap();
    assert!(kinds(&evs).contains(&"text"));
}

#[tokio::test]
async fn cancel_while_tool_is_waiting_answers_every_pending_call() {
    let sid = SessionId::new().to_string();
    let provider = ScriptedProvider::new(vec![calls(vec![
        tool(
            "a",
            "run_command",
            json!({"session_id": sid, "command": "sleep 999"}),
        ),
        tool("b", "list_sessions", json!({})),
    ])]);
    let tools = Arc::new(MockTools {
        hang_on_run: true,
        ..Default::default()
    });
    let mut a = agent(provider.clone(), tools, cfg());
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let (tx, mut rx) = mpsc::channel(64);
    let waiter = tokio::spawn(async move {
        let mut evs = vec![];
        while let Some(e) = rx.recv().await {
            if matches!(e, AgentEvent::ToolCall { .. }) {
                c2.cancel();
            }
            evs.push(e);
        }
        evs
    });
    a.run("go", None, tx, cancel).await.unwrap();
    let evs = waiter.await.unwrap();
    assert_eq!(kinds(&evs), ["call", "result", "done"]);
    assert!(matches!(
        &evs[1],
        AgentEvent::ToolResult { is_error: true, .. }
    ));
    assert_eq!(
        provider.requests().len(),
        1,
        "no further model call after cancel"
    );
    assert_history_well_formed(a.history());
    let last = tool_result_parts(a.history().last().unwrap());
    assert_eq!(last.len(), 2);
    assert!(last.iter().all(|(_, _, e)| *e));
}

#[tokio::test]
async fn history_persists_across_runs() {
    let provider = ScriptedProvider::new(vec![answer("first answer"), answer("second answer")]);
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), cfg());
    run(&mut a, "one").await.0.unwrap();
    let hint = session();
    let (tx, mut rx) = mpsc::channel(64);
    a.run("two", Some(hint.clone()), tx, CancellationToken::new())
        .await
        .unwrap();
    while rx.recv().await.is_some() {}

    let reqs = provider.requests();
    assert_eq!(reqs[1].messages.len(), 3);
    assert_eq!(reqs[1].messages[0], Message::user_text("one"));
    assert_eq!(reqs[1].messages[1], Message::assistant_text("first answer"));
    assert_eq!(reqs[1].messages[2], Message::user_text("two"));
    // Session hint lands in the system prompt of that run only.
    assert!(!reqs[0].system.as_ref().unwrap().contains("prod-web-1"));
    let sys = reqs[1].system.as_ref().unwrap();
    assert!(sys.contains(&hint.session_id.to_string()) && sys.contains("/srv/app"));
    assert_eq!(a.history().len(), 4);
    a.clear_history();
    assert!(a.history().is_empty());
}

#[tokio::test]
async fn large_tool_output_is_capped_for_model_but_not_for_ui() {
    let sid = SessionId::new().to_string();
    let provider = ScriptedProvider::new(vec![
        calls(vec![tool(
            "a",
            "run_command",
            json!({"session_id": sid, "command": "cat big"}),
        )]),
        answer("done"),
    ]);
    let tools = Arc::new(MockTools::default());
    let big = format!("START{}END", "y".repeat(200_000));
    tools
        .run_results
        .lock()
        .unwrap()
        .push_back(Ok(cmd_result(&big)));
    let mut a = agent(provider.clone(), tools, cfg());
    let (_, evs) = run(&mut a, "x").await;

    let fed = tool_result_parts(&provider.requests()[1].messages[2])[0]
        .1
        .to_string();
    assert!(fed.len() <= 12 * 1024, "{}", fed.len());
    let v: Value = serde_json::from_str(&fed).unwrap();
    assert_eq!(v["exit_code"], 0);
    assert!(v["output_id"].is_string());
    let o = v["output"].as_str().unwrap();
    assert!(o.starts_with("START") && o.ends_with("END") && o.contains("read_output"));
    let ui_len = evs
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolResult { output, .. } => output["output"].as_str().map(str::len),
            _ => None,
        })
        .unwrap();
    assert!(ui_len > 200_000);
}

#[tokio::test]
async fn provider_error_emits_error_and_done_and_rolls_back_first_step() {
    let provider = ScriptedProvider::new(vec![
        Turn::StartError(aht_providers::Error::RateLimited {
            message: "slow down".into(),
            retry_after: None,
        }),
        answer("fine now"),
    ]);
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), cfg());
    let (r, evs) = run(&mut a, "hello").await;
    assert!(matches!(r, Err(Error::Provider(m)) if m.contains("slow down")));
    assert_eq!(kinds(&evs), ["error", "done"]);
    assert!(matches!(
        &evs[0],
        AgentEvent::Error {
            retryable: true,
            ..
        }
    ));
    assert!(
        a.history().is_empty(),
        "retrying the same message must not duplicate it"
    );

    run(&mut a, "hello").await.0.unwrap();
    assert_eq!(provider.requests()[1].messages.len(), 1);

    // Auth errors are not retryable and map to Error::Auth.
    let provider = ScriptedProvider::new(vec![Turn::StartError(aht_providers::Error::Auth(
        "bad key".into(),
    ))]);
    let mut a = agent(provider, Arc::new(MockTools::default()), cfg());
    let (r, evs) = run(&mut a, "x").await;
    assert!(matches!(r, Err(Error::Auth(_))));
    assert!(matches!(
        &evs[0],
        AgentEvent::Error {
            retryable: false,
            ..
        }
    ));
}

#[tokio::test]
async fn mid_stream_error_after_tool_step_keeps_tool_history() {
    let sid = SessionId::new().to_string();
    let provider = ScriptedProvider::new(vec![
        calls(vec![tool(
            "a",
            "run_command",
            json!({"session_id": sid, "command": "ls"}),
        )]),
        Turn::StartError(aht_providers::Error::Network("reset".into())),
    ]);
    let mut a = agent(provider, Arc::new(MockTools::default()), cfg());
    let (r, evs) = run(&mut a, "x").await;
    assert!(r.is_err());
    assert_eq!(kinds(&evs), ["call", "result", "error", "done"]);
    // The command really ran, so its call/result pair stays in history.
    assert_eq!(a.history().len(), 3);
    assert_history_well_formed(a.history());
}

#[tokio::test]
async fn dropping_the_event_receiver_stops_the_run() {
    let provider = ScriptedProvider::repeating(vec![text("a"), stop(StopReason::EndTurn)]);
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), cfg());
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    a.run("x", None, tx, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test]
async fn a_cut_off_answer_is_kept_with_a_note() {
    let provider = ScriptedProvider::new(vec![Turn::Events(vec![
        text("cut"),
        stop(StopReason::MaxTokens),
    ])]);
    let mut a = agent(provider, Arc::new(MockTools::default()), cfg());
    let (r, evs) = run(&mut a, "x").await;
    assert!(r.is_ok());
    assert_eq!(kinds(&evs), ["text", "text", "done"]);
    assert!(matches!(&evs[1], AgentEvent::TextDelta { text } if text.contains("continue")));
}

#[tokio::test]
async fn thinking_that_fills_the_budget_is_retried_with_more_room() {
    let provider = ScriptedProvider::new(vec![
        Turn::Events(vec![
            ProviderEvent::ThinkingDelta("hmm".into()),
            stop(StopReason::MaxTokens),
        ]),
        Turn::Events(vec![text("done"), stop(StopReason::EndTurn)]),
    ]);
    let mut c = cfg();
    c.model = "deepseek-reasoner".into();
    c.max_tokens = 32_768;
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), c);
    let (r, _) = run(&mut a, "x").await;
    assert!(r.is_ok());
    let reqs = provider.requests();
    assert_eq!(
        reqs.iter().map(|r| r.max_tokens).collect::<Vec<_>>(),
        [32_768, 65_536]
    );
}

#[tokio::test]
async fn thinking_past_the_models_limit_is_a_clear_error() {
    let provider = ScriptedProvider::repeating(vec![
        ProviderEvent::ThinkingDelta("hmm".into()),
        stop(StopReason::MaxTokens),
    ]);
    let mut c = cfg();
    c.model = "deepseek-chat".into();
    c.max_tokens = 8_192;
    let mut a = agent(provider.clone(), Arc::new(MockTools::default()), c);
    let (r, evs) = run(&mut a, "x").await;
    assert!(r.is_err());
    assert_eq!(
        provider.requests().len(),
        1,
        "no retry beyond the model's limit"
    );
    assert!(
        matches!(evs.iter().find(|e| matches!(e, AgentEvent::Error { .. })),
        Some(AgentEvent::Error { message, retryable: true }) if message.contains("thinking"))
    );
}

#[test]
fn reply_budgets_fit_the_model() {
    use aht_agent::default_max_tokens as m;
    assert_eq!(m("deepseek-reasoner"), 32_768);
    assert_eq!(m("deepseek-chat"), 8_192);
    assert_eq!(m("claude-sonnet-4-5"), 16_384);
    assert_eq!(m("claude-3-haiku-20240307"), 4_096);
    assert_eq!(m("o3-mini"), 32_768);
    assert_eq!(m("llama3.1:8b"), 8_192);
}
