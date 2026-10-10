//! aht-mcp — local MCP server (streamable HTTP) that lets agent CLIs drive the user's terminals.
//! See docs/superpowers/specs/2026-10-10-aihubterm-design.md §8.
//!
//! Security model: bound to 127.0.0.1 only; every request needs `Authorization: Bearer <token>`
//! issued by [`McpServer::issue_token`] (one per harness launch, revoked when it exits); `Host`
//! must be `127.0.0.1:PORT`/`localhost:PORT` and any `Origin` must be loopback. The actor bound to
//! the token is used for every tool call, and an MCP session stays pinned to the token that opened
//! it.

mod auth;
mod format;
mod handler;

pub use auth::McpToken;
/// The server name advertised to clients; tools appear as `mcp__aihubterm__<tool>`.
pub const SERVER_NAME: &str = handler::SERVER_NAME;

use aht_core::terminal::{Actor, TerminalTools};
use aht_core::Result;
use auth::{GuardState, TokenStore};
use handler::{AhtHandler, Shared};
use rmcp::transport::streamable_http_server::{
    session::local::{LocalSessionManager, SessionConfig},
    StreamableHttpServerConfig, StreamableHttpService,
};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct McpOptions {
    /// Port on 127.0.0.1; `None` = pick a free one.
    pub port: Option<u16>,
    /// Server-side safety cap for one tool call (including time spent waiting for approval).
    /// Must exceed the longest `run_command` wait you want to support. Default 20 min.
    pub tool_timeout: Duration,
    /// An MCP session with no traffic for this long is dropped. Must exceed `tool_timeout`, since
    /// a long-running call produces no session events. Default 30 min.
    pub session_idle_timeout: Duration,
    /// SSE keep-alive ping interval; keeps idle HTTP connections open during long calls.
    pub sse_keep_alive: Duration,
}

impl Default for McpOptions {
    fn default() -> Self {
        Self {
            port: None,
            tool_timeout: Duration::from_secs(20 * 60),
            session_idle_timeout: Duration::from_secs(30 * 60),
            sse_keep_alive: Duration::from_secs(15),
        }
    }
}

pub struct McpServer {
    addr: SocketAddr,
    tokens: Arc<TokenStore>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl McpServer {
    /// Binds `127.0.0.1:<port or random>` and serves streamable HTTP at `/mcp`.
    pub async fn start(tools: Arc<dyn TerminalTools>, opts: McpOptions) -> Result<McpServer> {
        let listener =
            tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, opts.port.unwrap_or(0))).await?;
        let addr = listener.local_addr()?;
        let tokens = Arc::new(TokenStore::default());
        let cancel = CancellationToken::new();

        let shared = Arc::new(Shared::new(tools, opts.tool_timeout));
        let mut sessions = LocalSessionManager::default();
        let mut session_config = SessionConfig::default();
        session_config.keep_alive = Some(opts.session_idle_timeout);
        sessions.session_config = session_config;
        let config = StreamableHttpServerConfig::default()
            .with_sse_keep_alive(Some(opts.sse_keep_alive))
            .with_cancellation_token(cancel.child_token())
            .with_allowed_hosts([
                format!("127.0.0.1:{}", addr.port()),
                format!("localhost:{}", addr.port()),
            ]);
        let service = StreamableHttpService::new(
            move || Ok(AhtHandler::new(shared.clone())),
            Arc::new(sessions),
            config,
        );

        let guard = GuardState {
            tokens: tokens.clone(),
            port: addr.port(),
        };
        let app = axum::Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn_with_state(guard, auth::guard));

        let shutdown = cancel.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
                .await
            {
                tracing::error!(error = %e, "mcp server stopped with an error");
            }
        });
        tracing::info!(%addr, "mcp server listening");
        Ok(McpServer {
            addr,
            tokens,
            cancel,
            task,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:PORT/mcp`
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// Issues a fresh 256-bit token bound to `actor` (one per harness launch).
    pub fn issue_token(&self, actor: Actor) -> McpToken {
        tracing::debug!(actor = %actor.id, "mcp token issued");
        self.tokens.issue(actor)
    }

    /// Revokes a token; further requests carrying it get 401.
    pub fn revoke(&self, token: &McpToken) {
        self.tokens.revoke(token);
    }

    /// Stops accepting requests, ends all sessions and waits for the server task.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
        tracing::info!(addr = %self.addr, "mcp server stopped");
    }
}
