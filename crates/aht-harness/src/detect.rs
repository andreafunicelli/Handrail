//! Locating the harness CLIs, their versions and login state.
//!
//! Login state comes exclusively from the CLIs' own non-interactive status commands
//! (`claude auth status`, `codex login status`, `opencode auth list`). Credential files are never
//! opened.

use aht_core::model::HarnessKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// How long `--version` / status commands may run before we give up.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessInfo {
    pub kind: HarnessKind,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// `None` when not installed or the status could not be determined.
    pub logged_in: Option<bool>,
}

/// Executable base name of a harness.
pub fn binary_name(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::ClaudeCode => "claude",
        HarnessKind::Codex => "codex",
        HarnessKind::OpenCode => "opencode",
    }
}

/// Inverse of [`binary_name`].
pub fn kind_for_binary(name: &str) -> Option<HarnessKind> {
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.split('.').next().unwrap_or(stem);
    [
        HarnessKind::ClaudeCode,
        HarnessKind::Codex,
        HarnessKind::OpenCode,
    ]
    .into_iter()
    .find(|k| binary_name(*k) == stem)
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Well-known install directories to search after PATH.
pub fn known_dirs(kind: HarnessKind) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let home = home_dir();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(appdata).join("npm")); // npm global (Windows)
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(local).join("Microsoft/WinGet/Links")); // WinGet links
    }
    if let Some(h) = &home {
        v.push(h.join(".local/bin"));
        v.push(h.join(".npm-global/bin"));
        v.push(h.join(".bun/bin"));
        match kind {
            HarnessKind::ClaudeCode => v.push(h.join(".claude/local")),
            HarnessKind::OpenCode => v.push(h.join(".opencode/bin")),
            HarnessKind::Codex => {}
        }
    }
    #[cfg(unix)]
    {
        v.push(PathBuf::from("/opt/homebrew/bin"));
        v.push(PathBuf::from("/usr/local/bin"));
    }
    v
}

/// Look for `name` (with Windows executable extensions) in `dirs`.
pub fn find_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ".ps1"]
    } else {
        &[""]
    };
    for d in dirs {
        for e in exts {
            let p = d.join(format!("{name}{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// PATH first (the `which` crate honours PATHEXT on Windows), then known install dirs.
pub fn find_binary(kind: HarnessKind) -> Option<PathBuf> {
    let name = binary_name(kind);
    which::which(name)
        .ok()
        .or_else(|| find_in_dirs(name, &known_dirs(kind)))
}

/// Resolve a `LaunchSpec::program` to something spawnable: absolute/existing paths are kept,
/// harness names are located via [`find_binary`], anything else falls back to the bare name.
pub fn resolve_program(program: &str) -> PathBuf {
    let p = Path::new(program);
    if p.is_absolute() && p.exists() {
        return p.to_path_buf();
    }
    if let Some(found) = kind_for_binary(program).and_then(find_binary) {
        return found;
    }
    which::which(program).unwrap_or_else(|_| PathBuf::from(program))
}

/// Build a command for `path`, wrapping PowerShell shims (`.ps1`) which cannot be exec'd directly.
pub(crate) fn command_for(path: &Path) -> Command {
    let is_ps1 = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ps1"));
    // Mutated only by the Windows-only `creation_flags` call below.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = if is_ps1 {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-File"]).arg(path);
        c
    } else {
        Command::new(path)
    };
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// Run `path args...` and capture (exit code, stdout+stderr). `None` on spawn failure/timeout.
async fn probe(path: &Path, args: &[&str]) -> Option<(i32, String)> {
    let mut cmd = command_for(path);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.code().unwrap_or(-1), text))
}

/// Extract the version number from `--version` output such as `codex-cli 0.155.0`,
/// `1.18.30` or `2.1.29 (Claude Code)`.
pub fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .flat_map(|l| l.split_whitespace())
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(|t| {
            t.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_string()
        })
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `opencode auth list` prints one `●` line per configured credential/provider.
pub fn parse_opencode_auth_list(output: &str) -> bool {
    strip_ansi(output).lines().any(|l| l.contains('●'))
}

async fn login_status(kind: HarnessKind, path: &Path) -> Option<bool> {
    match kind {
        // exit 0 = logged in, 1 = not (docs: code.claude.com/docs/en/authentication)
        HarnessKind::ClaudeCode => probe(path, &["auth", "status"]).await.map(|(c, _)| c == 0),
        HarnessKind::Codex => probe(path, &["login", "status"]).await.map(|(c, _)| c == 0),
        HarnessKind::OpenCode => {
            let (code, out) = probe(path, &["auth", "list"]).await?;
            (code == 0).then(|| parse_opencode_auth_list(&out))
        }
    }
}

/// Detect a single harness.
pub async fn detect_one(kind: HarnessKind) -> HarnessInfo {
    let Some(path) = find_binary(kind) else {
        return HarnessInfo {
            kind,
            path: None,
            version: None,
            logged_in: None,
        };
    };
    let (version, logged_in) =
        tokio::join!(probe(&path, &["--version"]), login_status(kind, &path));
    HarnessInfo {
        kind,
        version: version.and_then(|(_, out)| parse_version(&out)),
        logged_in,
        path: Some(path),
    }
}

/// Detect all supported harnesses concurrently (order: Claude Code, Codex, OpenCode).
pub async fn detect_all() -> Vec<HarnessInfo> {
    let (a, b, c) = tokio::join!(
        detect_one(HarnessKind::ClaudeCode),
        detect_one(HarnessKind::Codex),
        detect_one(HarnessKind::OpenCode)
    );
    vec![a, b, c]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("codex-cli 0.155.0\n").as_deref(),
            Some("0.155.0")
        );
        assert_eq!(parse_version("1.18.30").as_deref(), Some("1.18.30"));
        assert_eq!(
            parse_version("2.1.290 (Claude Code)").as_deref(),
            Some("2.1.290")
        );
        assert_eq!(parse_version("v3.0.1-beta").as_deref(), Some("3.0.1-beta"));
        assert_eq!(parse_version("no digits here"), None);
    }

    #[test]
    fn opencode_auth_list() {
        let some = "\u{1b}[0m\n\u{1b}[90m┌\u{1b}[39m  Credentials\n\u{1b}[34m●\u{1b}[39m  OpenCode Go \u{1b}[90mapi\n";
        assert!(parse_opencode_auth_list(some));
        assert!(!parse_opencode_auth_list(
            "┌  Credentials\n└  0 credentials\n"
        ));
    }

    #[test]
    fn names() {
        assert_eq!(kind_for_binary("claude"), Some(HarnessKind::ClaudeCode));
        assert_eq!(
            kind_for_binary("C:\\x\\codex.cmd"),
            Some(HarnessKind::Codex)
        );
        assert_eq!(
            kind_for_binary("/usr/bin/opencode"),
            Some(HarnessKind::OpenCode)
        );
        assert_eq!(kind_for_binary("bash"), None);
    }

    #[test]
    fn finds_in_dirs() {
        let d = tempfile::tempdir().unwrap();
        let file = if cfg!(windows) { "codex.cmd" } else { "codex" };
        std::fs::write(d.path().join(file), "x").unwrap();
        let found = find_in_dirs("codex", &[d.path().to_path_buf()]).unwrap();
        assert_eq!(found, d.path().join(file));
        assert!(find_in_dirs("claude", &[d.path().to_path_buf()]).is_none());
    }

    #[test]
    fn known_dirs_include_kind_specific() {
        let claude = known_dirs(HarnessKind::ClaudeCode);
        assert!(claude.iter().any(|p| p.ends_with(".claude/local")) || home_dir().is_none());
        let oc = known_dirs(HarnessKind::OpenCode);
        assert!(oc.iter().any(|p| p.ends_with(".opencode/bin")) || home_dir().is_none());
    }

    #[test]
    fn unknown_program_falls_back_to_bare_name() {
        assert_eq!(
            resolve_program("definitely-not-a-real-binary-xyz"),
            PathBuf::from("definitely-not-a-real-binary-xyz")
        );
    }
}
