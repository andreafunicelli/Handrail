//! The MCP `ServerHandler`: tools come from `aht_core::terminal::tool_specs()` at runtime and are
//! dispatched through `aht_core::terminal::dispatch_tool` with the actor bound to the token.

use crate::auth::AuthContext;
use crate::format;
use aht_core::terminal::{dispatch_tool, tool_specs, Actor, TerminalTools};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        InitializeRequestParams, InitializeResult, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub(crate) const SERVER_NAME: &str = "handrail";

pub(crate) const INSTRUCTIONS: &str = "\
You are operating the user's LIVE terminal sessions in Handrail (SSH servers and local shells). \
Every command you run is typed into a real terminal the user is watching, on real machines.
- Start with list_sessions (or list_connections + open_session) and session_info to learn the OS, shell and cwd.
- run_command may require the user's approval first; the call waits until the user decides. \
If the result state is denied, do not retry the same command; ask the user.
- If a result state is needs_user (password prompt, full-screen program, user typing), STOP and ask the user. \
Never try to answer password prompts or type secrets.
- Prefer non-interactive commands and flags (no vim/top/less; use --no-pager, -y only when the user asked).
- Long output is truncated (head+tail); page through it with read_output and the output_id.
- A timed_out command is still running: poll with read_output or interrupt with send_keys [\"ctrl_c\"].
- To change a file on an SSH server, cat it first, then call propose_file_edit with the complete new contents; \
the user reviews the diff and saves it. Do not edit files with sed, echo or tee.";

/// Built once; every session handler shares it.
pub(crate) struct Shared {
    pub tools: Arc<dyn TerminalTools>,
    pub tool_list: Vec<Tool>,
    pub tool_timeout: Duration,
}

impl Shared {
    pub fn new(tools: Arc<dyn TerminalTools>, tool_timeout: Duration) -> Self {
        Self {
            tools,
            tool_list: mcp_tools(),
            tool_timeout,
        }
    }
}

/// Converts the core tool specs into MCP tool descriptors (schemas are not duplicated here).
pub(crate) fn mcp_tools() -> Vec<Tool> {
    tool_specs()
        .into_iter()
        .map(|s| {
            let schema: JsonObject = match s.input_schema {
                Value::Object(m) => m,
                _ => JsonObject::new(),
            };
            Tool::new(s.name, s.description, Arc::new(schema))
        })
        .collect()
}

/// One instance per MCP session (or per request in stateless mode).
pub(crate) struct AhtHandler {
    shared: Arc<Shared>,
    /// The token that opened this session; later requests must present the same one.
    bound_token: OnceLock<u64>,
}

impl AhtHandler {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            bound_token: OnceLock::new(),
        }
    }

    fn actor(&self, ctx: &RequestContext<RoleServer>) -> Result<Arc<Actor>, ErrorData> {
        let auth = ctx
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|p| p.extensions.get::<AuthContext>())
            .ok_or_else(|| ErrorData::invalid_request("unauthenticated request", None))?;
        let bound = *self.bound_token.get_or_init(|| auth.token_id);
        if bound != auth.token_id {
            tracing::warn!("mcp: request presented a different token than the session's");
            return Err(ErrorData::invalid_request(
                "this MCP session belongs to a different token",
                None,
            ));
        }
        Ok(auth.actor.clone())
    }
}

fn error_result(text: String) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(text)]).into()
}

impl ServerHandler for AhtHandler {
    fn get_info(&self) -> ServerConfig {
        let mut implementation = Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION"));
        implementation.title = Some("Handrail terminals".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(implementation)
            .with_instructions(INSTRUCTIONS)
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        // Pin this session to the token that opened it.
        self.actor(&ctx)?;
        ctx.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.actor(&ctx)?;
        Ok(ListToolsResult::with_all_items(
            self.shared.tool_list.clone(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.shared
            .tool_list
            .iter()
            .find(|t| t.name == name)
            .cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let actor = self.actor(&ctx)?;
        let name = request.name.as_ref();
        if !self.shared.tool_list.iter().any(|t| t.name == name) {
            let known: Vec<&str> = self
                .shared
                .tool_list
                .iter()
                .map(|t| t.name.as_ref())
                .collect();
            return Ok(error_result(format!(
                "Unknown tool `{name}`. Available tools: {}",
                known.join(", ")
            )));
        }
        let args = Value::Object(request.arguments.clone().unwrap_or_default());
        tracing::debug!(tool = name, actor = %actor.id, "mcp tool call");

        let call = dispatch_tool(self.shared.tools.as_ref(), &actor, name, args);
        let outcome = tokio::select! {
            r = tokio::time::timeout(self.shared.tool_timeout, call) => r,
            _ = ctx.ct.cancelled() => {
                tracing::debug!(tool = name, actor = %actor.id, "mcp tool call cancelled by client");
                return Ok(error_result(format!("{name} was cancelled by the client.")));
            }
        };
        Ok(match outcome {
            Ok(Ok(value)) => {
                tracing::debug!(tool = name, actor = %actor.id, "mcp tool call ok");
                CallToolResult::success(vec![ContentBlock::text(format::success_text(
                    name, &value,
                ))])
                .into()
            }
            Ok(Err(e)) => {
                tracing::debug!(tool = name, actor = %actor.id, error = %e, "mcp tool call failed");
                error_result(format::error_text(name, &e))
            }
            Err(_) => error_result(format!(
                "{name} did not finish within {} s and was abandoned by the MCP server. \
Check the session with session_info / read_output.",
                self.shared.tool_timeout.as_secs()
            )),
        })
    }
}
