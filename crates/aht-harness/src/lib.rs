//! aht-harness: detect, configure, launch and supervise the user's own agent CLIs (Claude Code,
//! Codex, OpenCode), wired to the local AIHubTerm MCP server.
//!
//! See `docs/superpowers/specs/2026-10-10-aihubterm-design.md` section 9. We never read harness
//! credential files; login state comes only from the CLIs' own status commands.

pub mod detect;
pub mod extensions;
pub mod launch;
pub mod profile;
pub mod runner;
pub mod stream;

pub use detect::{detect_all, detect_one, resolve_program, HarnessInfo};
pub use extensions::{list_extensions, resolve_plugin_dirs, Ext, Extensions};
pub use launch::{
    build_launch, login_spec, native_spec, LaunchContext, LaunchMode, LaunchSpec, API_BILLING_FLAG,
    DEFAULT_TOKEN_ENV_VAR, MCP_SERVER_NAME,
};
pub use profile::{context_markdown, prepare_profile_dir};
pub use runner::HeadlessRun;
pub use stream::{ClaudeStreamParser, CodexJsonParser, OpenCodeJsonParser, StreamParser};
