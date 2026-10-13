//! OpenChamber (<https://github.com/openchamber/openchamber>): a web UI for OpenCode.
//!
//! Handrail starts `openchamber serve` in the foreground on 127.0.0.1, password-protected, with
//! the environment of an OpenCode native launch. OpenChamber spawns `opencode serve` with that
//! environment (it passes `OPENCODE_CONFIG_CONTENT` through), so its OpenCode gets Handrail's
//! MCP tools exactly like the other OpenCode displays. The UI is then shown in a window of its
//! own (OpenChamber's `frame-ancestors` and `SameSite=Strict` cookie rule out an iframe).
//!
//! Facts about the CLI (`packages/web/bin/lib/cli-args.js`, `commands-serve.js`, OpenChamber
//! 2.3): `--foreground` keeps it attached; `--quiet` prints the bound port on stdout once it is
//! listening; `OPENCODE_BINARY` names the OpenCode executable to spawn.

use crate::detect::{command_for, find_binary, find_in_dirs, known_dirs};
use crate::launch::LaunchSpec;
use crate::runner::{cleanup, kill_tree, write_files};
use aht_core::model::HarnessKind;
use aht_core::{Error, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;

/// The executable's base name.
pub const BINARY: &str = "openchamber";
/// How long OpenChamber (and the OpenCode it starts) may take to listen.
pub const START_TIMEOUT: Duration = Duration::from_secs(90);

/// PATH first, then the directories npm/bun install global CLIs to.
pub fn find() -> Option<PathBuf> {
    which::which(BINARY)
        .ok()
        .or_else(|| find_in_dirs(BINARY, &known_dirs(HarnessKind::OpenCode)))
}

/// The arguments of `openchamber serve` for a private, foreground server.
pub fn serve_args(port: u16, password: &str) -> Vec<String> {
    [
        "serve",
        "--foreground",
        "--quiet",
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--ui-password",
        password,
    ]
    .map(String::from)
    .to_vec()
}

/// The port `--quiet` prints once the server listens: `"<port>"` or `"<port> pass:<pw>"`.
pub fn parse_ready_line(line: &str) -> Option<u16> {
    line.split_whitespace()
        .next()?
        .parse()
        .ok()
        .filter(|p| *p > 0)
}

/// A running OpenChamber server; stopped (with its OpenCode) by [`OpenChamber::stop`] or when
/// dropped.
pub struct OpenChamber {
    child: Child,
    pub port: u16,
    files: Vec<PathBuf>,
}

impl OpenChamber {
    /// Start OpenChamber with `spec`'s environment and files (an OpenCode native launch) and wait
    /// until it listens.
    pub async fn start(spec: &LaunchSpec, password: &str) -> Result<Self> {
        let program = find().ok_or_else(|| {
            Error::NotFound(
                "OpenChamber is not installed. Install it with `npm install -g @openchamber/web` \
                 (needs Node.js 24.14 or newer) and OpenCode, then try again."
                    .into(),
            )
        })?;
        let port = free_port()?;
        let files = write_files(spec)?;
        let mut cmd = command_for(&program);
        cmd.args(serve_args(port, password))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        if let Some(opencode) = find_binary(HarnessKind::OpenCode) {
            cmd.env("OPENCODE_BINARY", opencode);
        }
        #[cfg(unix)]
        cmd.process_group(0); // so `kill_tree` reaches the OpenCode it starts
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                cleanup(&files);
                return Err(Error::Other(format!("could not start OpenChamber: {e}")));
            }
        };
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::info!(target: "openchamber", "{line}");
                }
            });
        }
        let stdout = child.stdout.take();
        let ready = async {
            let mut lines = BufReader::new(stdout?).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(p) = parse_ready_line(&line) {
                    // Keep draining stdout so the server never blocks on a full pipe.
                    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
                    return Some(p);
                }
                tracing::info!(target: "openchamber", "{line}");
            }
            None
        };
        let mut server = Self { child, port, files };
        match tokio::time::timeout(START_TIMEOUT, ready).await {
            Ok(Some(p)) => {
                server.port = p;
                Ok(server)
            }
            Ok(None) => {
                let status = server.child.wait().await.ok();
                server.stop().await;
                Err(Error::Other(format!(
                    "OpenChamber exited before it was ready ({}); see the log for its output",
                    status.map_or("unknown status".into(), |s| s.to_string())
                )))
            }
            Err(_) => {
                server.stop().await;
                Err(Error::Other(format!(
                    "OpenChamber did not start within {} s",
                    START_TIMEOUT.as_secs()
                )))
            }
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Stop OpenChamber and the OpenCode server it started, and remove the launch files.
    pub async fn stop(&mut self) {
        if let Some(pid) = self.child.id() {
            kill_tree(pid).await;
        }
        let _ = self.child.kill().await;
        cleanup(&self.files);
        self.files.clear();
    }
}

/// A port nothing listens on right now (the OS picks it; OpenChamber binds it moments later).
fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| Error::Other(format!("no free local port: {e}")))?;
    Ok(l.local_addr()
        .map_err(|e| Error::Other(e.to_string()))?
        .port())
}

/// JavaScript for the OpenChamber window: signs in once with `password` (same origin, so the
/// `SameSite=Strict` session cookie is set) and reloads. Runs before the page's own scripts.
pub fn auto_login_script(origin: &str, password: &str) -> String {
    let origin = serde_json::to_string(origin).expect("string");
    let body = serde_json::json!({ "password": password, "trustDevice": false }).to_string();
    // A JSON string is a valid JS string literal.
    let body = serde_json::to_string(&body).expect("string");
    format!(
        r#"(() => {{
  if (location.origin !== {origin}) return;
  const key = "handrail-signed-in";
  try {{ if (sessionStorage.getItem(key)) return; sessionStorage.setItem(key, "1"); }} catch {{ return; }}
  fetch("/auth/session", {{
    method: "POST",
    headers: {{ "Content-Type": "application/json" }},
    credentials: "same-origin",
    body: {body},
  }}).then((r) => {{ if (r.ok) location.reload(); }}).catch(() => {{}});
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_lines() {
        assert_eq!(parse_ready_line("41234\n"), Some(41234));
        assert_eq!(parse_ready_line("41234 pass:abc"), Some(41234));
        assert_eq!(
            parse_ready_line("Starting OpenChamber on port auto (foreground)"),
            None
        );
        assert_eq!(parse_ready_line("0"), None);
        assert_eq!(parse_ready_line(""), None);
    }

    #[test]
    fn private_foreground_server() {
        let a = serve_args(4100, "s3cret");
        assert_eq!(a[0], "serve");
        for f in ["--foreground", "--quiet"] {
            assert!(a.iter().any(|x| x == f), "{f}");
        }
        let after = |flag: &str| a[a.iter().position(|x| x == flag).unwrap() + 1].clone();
        assert_eq!(after("--host"), "127.0.0.1");
        assert_eq!(after("--port"), "4100");
        assert_eq!(after("--ui-password"), "s3cret");
        assert!(!a.iter().any(|x| x == "--lan"));
    }

    #[test]
    fn login_script_targets_its_origin() {
        let s = auto_login_script("http://127.0.0.1:4100", "p\"w");
        assert!(s.contains(r#"location.origin !== "http://127.0.0.1:4100""#));
        assert!(s.contains("/auth/session"));
        // The password is JSON-encoded inside a JS string literal: quotes stay escaped.
        assert!(s.contains(r#"\"password\":\"p\\\"w\""#), "{s}");
    }
}
