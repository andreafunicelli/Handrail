//! End-to-end tests: real `McpServer` on 127.0.0.1, driven by rmcp's streamable-HTTP client
//! (the same transport Claude Code / Codex / OpenCode use) and by raw HTTP for auth checks.

mod common;

use aht_core::terminal::{tool_specs, Actor};
use aht_mcp::{McpOptions, McpServer, McpToken, SERVER_NAME};
use common::*;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ClientConfig, ClientRequest, JsonObject,
        ProtocolVersion, Request,
    },
    service::{PeerRequestOptions, RoleClient, RunningService},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ClientConfig>;

fn actor(id: &str) -> Actor {
    Actor {
        id: id.into(),
        label: format!("Agent {id}"),
        session_scope: None,
    }
}

async fn start(opts: McpOptions) -> (McpServer, Arc<MockTools>) {
    let mock = Arc::new(MockTools::default());
    let server = McpServer::start(mock.clone(), opts).await.expect("start");
    (server, mock)
}

async fn connect_with(url: &str, token: &McpToken, version: ProtocolVersion) -> Client {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url.to_string()).auth_header(token.as_str()),
    );
    ClientConfig::default()
        .with_protocol_version(version)
        .serve(transport)
        .await
        .expect("client connect")
}

async fn connect(url: &str, token: &McpToken) -> Client {
    connect_with(url, token, ProtocolVersion::LATEST_WITH_INITIALIZE).await
}

async fn call(client: &Client, name: &'static str, args: Value) -> CallToolResult {
    let mut p = CallToolRequestParams::new(name);
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    client.call_tool(p).await.expect("call_tool")
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses the JSON part of a successful result (after an optional summary line).
fn json_of(r: &CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "unexpected error: {}", text(r));
    let t = text(r);
    let start = t.find(['{', '[']).expect("json in result");
    serde_json::from_str(&t[start..]).expect("valid json")
}

#[tokio::test]
async fn url_and_addr() {
    let (server, _) = start(McpOptions::default()).await;
    assert!(server.addr().ip().is_loopback());
    assert_eq!(
        server.url(),
        format!("http://127.0.0.1:{}/mcp", server.addr().port())
    );
    server.shutdown().await;
}

#[tokio::test]
async fn info_and_tools_match_core_specs_across_protocol_versions() {
    let (server, _) = start(McpOptions::default()).await;
    let token = server.issue_token(actor("a"));
    for v in [
        ProtocolVersion::V_2025_03_26,
        ProtocolVersion::V_2025_06_18,
        ProtocolVersion::LATEST_WITH_INITIALIZE,
        ProtocolVersion::LATEST,
    ] {
        let client = connect_with(&server.url(), &token, v.clone()).await;
        if v.has_initialize() {
            let info = client.peer_info().expect("server info");
            assert_eq!(info.server_info.as_ref().unwrap().name, SERVER_NAME);
            let instr = info.instructions.clone().unwrap_or_default();
            assert!(instr.contains("needs_user"), "{instr}");
            assert!(instr.contains("approval"), "{instr}");
        }
        let tools = client.list_all_tools().await.expect("list tools");
        let specs = tool_specs();
        assert_eq!(tools.len(), specs.len(), "version {v:?}");
        for (t, s) in tools.iter().zip(specs.iter()) {
            assert_eq!(t.name, s.name);
            assert_eq!(t.description.as_deref(), Some(s.description));
            assert_eq!(Value::Object((*t.input_schema).clone()), s.input_schema);
        }
        client.cancel().await.ok();
    }
    server.shutdown().await;
}

#[tokio::test]
async fn every_tool_round_trips() {
    let (server, mock) = start(McpOptions::default()).await;
    let token = server.issue_token(actor("harness:test"));
    let c = connect(&server.url(), &token).await;

    let v = json_of(&call(&c, "list_connections", json!({})).await);
    assert_eq!(v, serde_json::to_value(vec![connection()]).unwrap());

    let v = json_of(&call(&c, "list_sessions", json!({})).await);
    assert_eq!(v, serde_json::to_value(vec![session()]).unwrap());

    let v = json_of(&call(&c, "open_session", json!({"connection_id": conn_id()})).await);
    assert_eq!(v, serde_json::to_value(session()).unwrap());

    let v = json_of(&call(&c, "session_info", json!({"session_id": sess_id()})).await);
    assert_eq!(v, serde_json::to_value(session()).unwrap());

    let r = call(
        &c,
        "run_command",
        json!({"session_id": sess_id(), "command": "uptime", "reason": "check load"}),
    )
    .await;
    let t = text(&r);
    assert_eq!(
        t.lines().next().unwrap(),
        "exit 0 · 132 ms · output 2.1 KB",
        "{t}"
    );
    assert_eq!(
        json_of(&r),
        serde_json::to_value(command_result("uptime")).unwrap()
    );

    let v = json_of(
        &call(
            &c,
            "read_output",
            json!({"output_id": out_id(), "offset": 10}),
        )
        .await,
    );
    assert_eq!(v["data"], "hello");
    assert_eq!(v["offset"], 10);

    let v = json_of(
        &call(
            &c,
            "send_keys",
            json!({"session_id": sess_id(), "keys": ["ctrl_c", "q"]}),
        )
        .await,
    );
    assert_eq!(v, json!({"ok": true}));

    let calls = mock.calls();
    let names: Vec<_> = calls.iter().map(|c| c.tool).collect();
    assert_eq!(
        names,
        [
            "list_connections",
            "list_sessions",
            "open_session",
            "session_info",
            "run_command",
            "read_output",
            "send_keys"
        ]
    );
    assert!(calls.iter().all(|c| c.actor == "harness:test"));
    assert_eq!(calls[4].args["reason"], "check load");
    assert_eq!(calls[5].args["limit"], 8192, "core default limit applied");
    c.cancel().await.ok();
    server.shutdown().await;
}

#[tokio::test]
async fn tool_errors_are_results_not_protocol_errors() {
    let (server, _) = start(McpOptions::default()).await;
    let token = server.issue_token(actor("a"));
    let c = connect(&server.url(), &token).await;

    // Core error (NotFound) -> is_error with a hint.
    let other = uuid::Uuid::from_u128(99);
    let r = call(&c, "session_info", json!({"session_id": other})).await;
    assert_eq!(r.is_error, Some(true));
    let t = text(&r);
    assert!(
        t.contains("not found") && t.contains("list_sessions"),
        "{t}"
    );

    // Invalid arguments.
    let r = call(&c, "run_command", json!({"session_id": "not-a-uuid"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("invalid input"), "{}", text(&r));

    // Busy.
    let r = call(
        &c,
        "run_command",
        json!({"session_id": sess_id(), "command": "busy"}),
    )
    .await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("busy"));

    // Unknown tool.
    let r = call(&c, "rm_rf", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("Available tools: list_connections"));

    // needs_user is a normal result with a clear instruction.
    let r = call(
        &c,
        "run_command",
        json!({"session_id": sess_id(), "command": "sudo -i"}),
    )
    .await;
    assert_ne!(r.is_error, Some(true));
    let first = text(&r).lines().next().unwrap().to_string();
    assert!(
        first.starts_with("needs user") && first.contains("ask the user"),
        "{first}"
    );

    // The session still works afterwards.
    assert_ne!(
        call(&c, "list_sessions", json!({})).await.is_error,
        Some(true)
    );
    c.cancel().await.ok();
    server.shutdown().await;
}

#[tokio::test]
async fn actor_is_bound_to_token() {
    let (server, mock) = start(McpOptions::default()).await;
    let ta = server.issue_token(actor("harness:claude_code:p1"));
    let tb = server.issue_token(actor("harness:codex:p2"));
    let ca = connect(&server.url(), &ta).await;
    let cb = connect_with(&server.url(), &tb, ProtocolVersion::LATEST).await;
    call(&ca, "list_sessions", json!({})).await;
    call(&cb, "list_connections", json!({})).await;
    call(&ca, "session_info", json!({"session_id": sess_id()})).await;
    let got: Vec<_> = mock
        .calls()
        .into_iter()
        .map(|c| (c.actor, c.tool))
        .collect();
    assert_eq!(
        got,
        [
            ("harness:claude_code:p1".to_string(), "list_sessions"),
            ("harness:codex:p2".to_string(), "list_connections"),
            ("harness:claude_code:p1".to_string(), "session_info"),
        ]
    );
    ca.cancel().await.ok();
    cb.cancel().await.ok();
    server.shutdown().await;
}

#[tokio::test]
async fn slow_run_command_completes() {
    // Keep-alive shorter than the call so SSE pings flow while waiting.
    let (server, _) = start(McpOptions {
        sse_keep_alive: Duration::from_millis(500),
        ..Default::default()
    })
    .await;
    let token = server.issue_token(actor("a"));
    for v in [
        ProtocolVersion::LATEST_WITH_INITIALIZE,
        ProtocolVersion::LATEST,
    ] {
        let c = connect_with(&server.url(), &token, v).await;
        let t0 = Instant::now();
        let r = call(
            &c,
            "run_command",
            json!({"session_id": sess_id(), "command": "slow"}),
        )
        .await;
        assert!(t0.elapsed() >= Duration::from_secs(3));
        assert_eq!(json_of(&r)["command"], "slow");
        c.cancel().await.ok();
    }
    server.shutdown().await;
}

#[tokio::test]
async fn client_cancellation_stops_waiting() {
    let (server, mock) = start(McpOptions::default()).await;
    let token = server.issue_token(actor("a"));
    let c = connect(&server.url(), &token).await;
    let mut args = JsonObject::new();
    args.insert("session_id".into(), json!(sess_id()));
    args.insert("command".into(), json!("hang"));
    let handle = c
        .peer()
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(
                CallToolRequestParams::new("run_command").with_arguments(args),
            )),
            PeerRequestOptions::with_timeout(Duration::from_millis(500)),
        )
        .await
        .expect("send");
    // Client-side timeout sends notifications/cancelled to the server.
    assert!(handle.await_response().await.is_err());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !mock.hang_dropped.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "server kept waiting after cancel"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Session still usable.
    assert_ne!(
        call(&c, "list_sessions", json!({})).await.is_error,
        Some(true)
    );
    c.cancel().await.ok();
    server.shutdown().await;
}

#[tokio::test]
async fn server_side_tool_timeout_is_reported() {
    let (server, mock) = start(McpOptions {
        tool_timeout: Duration::from_millis(300),
        ..Default::default()
    })
    .await;
    let token = server.issue_token(actor("a"));
    let c = connect(&server.url(), &token).await;
    let r = call(
        &c,
        "run_command",
        json!({"session_id": sess_id(), "command": "hang"}),
    )
    .await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("did not finish"), "{}", text(&r));
    assert!(mock.hang_dropped.load(Ordering::SeqCst));
    c.cancel().await.ok();
    server.shutdown().await;
}

// ---------- raw HTTP: auth, Host, Origin, session pinning ----------

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"raw","version":"1"}}}"#;

fn post(url: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(INIT)
}

#[tokio::test]
async fn missing_invalid_and_revoked_tokens_get_401() {
    let (server, mock) = start(McpOptions::default()).await;
    let url = server.url();
    let token = server.issue_token(actor("a"));

    let r = post(&url).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(r.headers()["www-authenticate"], "Bearer");
    let r = post(&url).bearer_auth("wrong").send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = post(&url)
        .header("Authorization", format!("Basic {}", token.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    // Non-/mcp paths are guarded too.
    let r = reqwest::get(format!("http://{}/", server.addr()))
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    let r = post(&url).bearer_auth(token.as_str()).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.status());

    // An rmcp client with a bad token cannot connect.
    let bad = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url.clone()).auth_header("nope"),
    );
    assert!(ClientConfig::default().serve(bad).await.is_err());

    // Revocation applies to an already-connected client.
    let c = connect(&url, &token).await;
    call(&c, "list_sessions", json!({})).await;
    server.revoke(&token);
    let mut p = CallToolRequestParams::new("list_connections");
    p = p.with_arguments(JsonObject::new());
    assert!(c.call_tool(p).await.is_err());
    let r = post(&url).bearer_auth(token.as_str()).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(
        mock.calls()
            .iter()
            .filter(|c| c.tool == "list_connections")
            .count(),
        0
    );
    c.cancel().await.ok();
    server.shutdown().await;
}

#[tokio::test]
async fn bad_host_or_origin_rejected() {
    let (server, _) = start(McpOptions::default()).await;
    let url = server.url();
    let port = server.addr().port();
    let token = server.issue_token(actor("a"));

    for host in [
        format!("evil.example:{port}"),
        "127.0.0.1".to_string(),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
    ] {
        let r = post(&url)
            .bearer_auth(token.as_str())
            .header("Host", host.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "host {host}");
    }
    let r = post(&url)
        .bearer_auth(token.as_str())
        .header("Host", format!("localhost:{port}"))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());

    for origin in [
        "http://evil.example",
        "null",
        "http://localhost.evil.example",
    ] {
        let r = post(&url)
            .bearer_auth(token.as_str())
            .header("Origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "origin {origin}");
    }
    let r = post(&url)
        .bearer_auth(token.as_str())
        .header("Origin", "http://localhost:1420")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    server.shutdown().await;
}

#[tokio::test]
async fn session_is_pinned_to_its_token() {
    let (server, mock) = start(McpOptions::default()).await;
    let url = server.url();
    let ta = server.issue_token(actor("a"));
    let tb = server.issue_token(actor("b"));

    let r = post(&url).bearer_auth(ta.as_str()).send().await.unwrap();
    assert!(r.status().is_success());
    let sid = r.headers()["mcp-session-id"].to_str().unwrap().to_string();
    let _ = r.text().await;

    let send = |token: &McpToken, body: &'static str| {
        reqwest::Client::new()
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", sid.clone())
            .header("MCP-Protocol-Version", "2025-06-18")
            .bearer_auth(token.as_str())
            .body(body)
            .send()
    };
    let r = send(
        &ta,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await
    .unwrap();
    assert!(r.status().is_success(), "{}", r.status());

    let call_body = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#;
    let r = send(&tb, call_body).await.unwrap();
    let body = tokio::time::timeout(Duration::from_secs(5), r.text())
        .await
        .unwrap()
        .unwrap();
    assert!(body.contains("different token"), "{body}");
    assert!(mock.calls().is_empty(), "token b must not reach the tools");

    let r = send(&ta, call_body).await.unwrap();
    let body = tokio::time::timeout(Duration::from_secs(5), r.text())
        .await
        .unwrap()
        .unwrap();
    assert!(body.contains("web-1"), "{body}");
    assert_eq!(mock.calls()[0].actor, "a");
    server.shutdown().await;
}

#[tokio::test]
async fn shutdown_with_connected_client_returns() {
    let (server, _) = start(McpOptions::default()).await;
    let token = server.issue_token(actor("a"));
    let c = connect(&server.url(), &token).await;
    call(&c, "list_sessions", json!({})).await;
    let url = server.url();
    tokio::time::timeout(Duration::from_secs(10), server.shutdown())
        .await
        .expect("shutdown hung");
    assert!(reqwest::get(url).await.is_err());
    drop(c);
}
