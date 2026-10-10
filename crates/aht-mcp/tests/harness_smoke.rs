//! Manual smoke checks against locally installed agent CLIs. They only read config / probe our
//! local server (no model calls, no subscription usage). Run with:
//! `cargo test -p aht-mcp --test harness_smoke -- --ignored --nocapture`

mod common;

use aht_core::terminal::Actor;
use aht_mcp::{McpOptions, McpServer};
use common::MockTools;
use std::process::Command;
use std::sync::Arc;

const TOKEN_ENV: &str = "AIHUB_MCP_TOKEN";

async fn server() -> (McpServer, String) {
    let server = McpServer::start(Arc::new(MockTools::default()), McpOptions::default())
        .await
        .unwrap();
    let token = server.issue_token(Actor::user());
    (server, token.as_str().to_string())
}

fn run(mut cmd: Command) -> String {
    let out = cmd.output().expect("spawn CLI (is it installed?)");
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("{s}");
    s
}

/// Codex: `-c mcp_servers.handrail.*` overrides are accepted and parsed as an HTTP server.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the codex CLI installed"]
async fn codex_accepts_cli_mcp_config() {
    let (server, token) = server().await;
    let program = if cfg!(windows) { "codex.cmd" } else { "codex" };
    let mut cmd = Command::new(program);
    cmd.env(TOKEN_ENV, token).args([
        "-c",
        &format!("mcp_servers.handrail.url=\"{}\"", server.url()),
        "-c",
        &format!("mcp_servers.handrail.bearer_token_env_var=\"{TOKEN_ENV}\""),
        "-c",
        "mcp_servers.handrail.tool_timeout_sec=900",
        "-c",
        "mcp_servers.handrail.startup_timeout_sec=30",
        "mcp",
        "get",
        "handrail",
        "--json",
    ]);
    let out = tokio::task::spawn_blocking(move || run(cmd)).await.unwrap();
    assert!(out.contains(&server.url()), "{out}");
    assert!(out.contains(TOKEN_ENV), "{out}");
    server.shutdown().await;
}

/// OpenCode: `OPENCODE_CONFIG_CONTENT` remote server with a bearer header connects to us.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the opencode CLI installed"]
async fn opencode_connects_with_env_config() {
    let (server, token) = server().await;
    let config = serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": { "handrail": {
            "type": "remote",
            "url": server.url(),
            "enabled": true,
            "oauth": false,
            "timeout": 30000,
            "headers": { "Authorization": format!("Bearer {{env:{TOKEN_ENV}}}") }
        }}
    });
    let mut cmd = Command::new("opencode");
    cmd.env(TOKEN_ENV, token)
        .env("OPENCODE_CONFIG_CONTENT", config.to_string())
        .args(["mcp", "list"]);
    let out = tokio::task::spawn_blocking(move || run(cmd)).await.unwrap();
    let line = out
        .lines()
        .find(|l| l.contains("handrail"))
        .unwrap_or_else(|| panic!("handrail not listed: {out}"));
    assert!(line.contains("connected"), "{line}");
    server.shutdown().await;
}

/// Claude Code (not installed on the dev box when this was written). Starts the server, writes
/// the `--mcp-config` file and keeps serving for `AHT_MCP_MANUAL_SECS` (default 120) seconds so you
/// can run the printed command and check `/mcp` shows `handrail` connected with 7 tools.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual: needs the claude CLI and a human"]
async fn claude_code_manual_check() {
    let (server, token) = server().await;
    let config = serde_json::json!({ "mcpServers": { "handrail": {
        "type": "http",
        "url": server.url(),
        "headers": { "Authorization": format!("Bearer ${{{TOKEN_ENV}}}") }
    }}});
    let path = std::env::temp_dir().join("handrail-mcp-smoke.json");
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    println!(
        "{TOKEN_ENV}={token} claude --mcp-config {} --strict-mcp-config --allowedTools \"mcp__handrail__*\"",
        path.display()
    );
    let secs = std::env::var("AHT_MCP_MANUAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
    server.shutdown().await;
}
