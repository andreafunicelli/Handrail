//! aht-import: import saved SSH connections from other tools.
//!
//! See `docs/superpowers/specs/2026-10-10-aihubterm-design.md` section 12.

mod builder;
mod ini;
mod mobaxterm;
mod mremoteng;
mod putty;
mod reg;
mod securecrt;
mod ssh_config;
mod tabby;
mod text;
mod winscp;

use aht_core::model::{Connection, Group, Secret};
use aht_core::{ConnectionId, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    SshConfig,
    Putty,
    MobaXterm,
    MRemoteNg,
    WinScp,
    SecureCrt,
    Tabby,
}

impl Tool {
    pub fn slug(self) -> &'static str {
        match self {
            Tool::SshConfig => "ssh_config",
            Tool::Putty => "putty",
            Tool::MobaXterm => "mobaxterm",
            Tool::MRemoteNg => "mremoteng",
            Tool::WinScp => "winscp",
            Tool::SecureCrt => "securecrt",
            Tool::Tabby => "tabby",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedSource {
    pub id: String,
    pub tool: Tool,
    /// `None` means a non-file source (the Windows registry).
    pub path: Option<PathBuf>,
    pub count_hint: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ImportBundle {
    pub tool: Tool,
    pub groups: Vec<Group>,
    pub connections: Vec<Connection>,
    pub secrets: Vec<Secret>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Master password of an mRemoteNG file. `None` means mRemoteNG's built-in default (`mR3m`).
    pub mremoteng_password: Option<String>,
    /// Import saved passwords and private key contents as secrets.
    pub import_passwords: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            mremoteng_password: None,
            import_passwords: true,
        }
    }
}

/// Look for the supported tools' data in their standard locations on this OS.
pub fn detect() -> Vec<DetectedSource> {
    let mut out = Vec::new();
    let mut push = |tool: Tool, path: Option<PathBuf>, hint: Option<usize>| {
        let id = match &path {
            Some(p) => format!("{}:{}", tool.slug(), p.display()),
            None => format!("{}:registry", tool.slug()),
        };
        out.push(DetectedSource {
            id,
            tool,
            path,
            count_hint: hint,
        });
    };

    if let Some(home) = dirs::home_dir() {
        let cfg = home.join(".ssh").join("config");
        if cfg.is_file() {
            let n = ssh_config::count_hosts(&cfg);
            push(Tool::SshConfig, Some(cfg), Some(n));
        }
        for d in [
            home.join(".putty").join("sessions"),
            home.join(".config").join("putty").join("sessions"),
        ] {
            if d.is_dir() {
                let n = std::fs::read_dir(&d).map(|r| r.count()).unwrap_or(0);
                push(Tool::Putty, Some(d), Some(n));
                break;
            }
        }
    }
    #[cfg(windows)]
    {
        if let Some(n) = putty::registry_count() {
            push(Tool::Putty, None, Some(n));
        }
        if let Some(n) = winscp::registry_count() {
            push(Tool::WinScp, None, Some(n));
        }
    }
    if let Some(appdata) = dirs::config_dir() {
        // On Windows `config_dir` is %APPDATA%.
        let cands = [
            (
                Tool::MobaXterm,
                appdata.join("MobaXterm").join("MobaXterm.ini"),
            ),
            (
                Tool::MRemoteNg,
                appdata.join("mRemoteNG").join("confCons.xml"),
            ),
            (Tool::WinScp, appdata.join("WinSCP.ini")),
            (Tool::Tabby, appdata.join("tabby").join("config.yaml")),
        ];
        for (tool, p) in cands {
            if p.is_file() {
                push(tool, Some(p), None);
            }
        }
    }
    out
}

pub fn parse_source(src: &DetectedSource, opts: &ImportOptions) -> Result<ImportBundle> {
    match &src.path {
        Some(p) => parse_file(src.tool, p, opts),
        None => match src.tool {
            #[cfg(windows)]
            Tool::Putty => putty::parse_registry(opts),
            #[cfg(windows)]
            Tool::WinScp => winscp::parse_registry(opts),
            t => Err(Error::Invalid(format!(
                "{} has no registry source on this OS",
                t.slug()
            ))),
        },
    }
}

pub fn parse_file(tool: Tool, path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    match tool {
        Tool::SshConfig => ssh_config::parse_file(path, opts),
        Tool::Putty => putty::parse_path(path, opts),
        Tool::MobaXterm => mobaxterm::parse_file(path, opts),
        Tool::MRemoteNg => mremoteng::parse_file(path, opts),
        Tool::WinScp => winscp::parse_file(path, opts),
        Tool::SecureCrt => securecrt::parse_file(path, opts),
        Tool::Tabby => tabby::parse_file(path, opts),
    }
}

/// Bundle connections that already exist (same host case-insensitive, port and user).
/// Returns `(bundle index, existing connection id)` pairs.
pub fn find_duplicates(
    existing: &[Connection],
    bundle: &ImportBundle,
) -> Vec<(usize, ConnectionId)> {
    let key = |c: &Connection| (c.host.to_lowercase(), c.port, c.user.clone());
    let map: Vec<_> = existing.iter().map(|c| (key(c), c.id)).collect();
    bundle
        .connections
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let k = key(c);
            map.iter().find(|(ek, _)| *ek == k).map(|(_, id)| (i, *id))
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Copy `tests/fixtures/<name>` into a temp dir, replacing `{{DIR}}` with that dir's path so
/// fixtures can reference sibling files by absolute path.
#[cfg(test)]
pub(crate) fn stage_dir(name: &str) -> tempfile::TempDir {
    fn copy(src: &Path, dst: &Path, dir: &str) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            let to = dst.join(e.file_name());
            if e.path().is_dir() {
                copy(&e.path(), &to, dir);
            } else {
                let bytes = std::fs::read(e.path()).unwrap();
                match String::from_utf8(bytes.clone()) {
                    Ok(t) => std::fs::write(to, t.replace("{{DIR}}", dir)).unwrap(),
                    Err(_) => std::fs::write(to, bytes).unwrap(),
                }
            }
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap().replace('\\', "/");
    copy(&fixture(name), tmp.path(), &dir);
    tmp
}

#[cfg(test)]
mod tests {
    use super::*;
    use aht_core::model::SshAuth;

    fn mk(host: &str, port: u16, user: &str) -> Connection {
        let mut b = builder::Builder::new(Tool::SshConfig, &ImportOptions::default());
        let id = b.add("n", None, host, port, user, SshAuth::Agent, String::new());
        b.connections.iter().find(|c| c.id == id).cloned().unwrap()
    }

    #[test]
    fn duplicates_match_host_port_user_case_insensitive_host() {
        let existing = vec![mk("Example.COM", 22, "root"), mk("other", 22, "bob")];
        let bundle = ImportBundle {
            tool: Tool::Putty,
            groups: vec![],
            connections: vec![
                mk("example.com", 22, "root"),
                mk("example.com", 2222, "root"),
                mk("example.com", 22, "Root"),
                mk("other", 22, "bob"),
            ],
            secrets: vec![],
            warnings: vec![],
        };
        let d = find_duplicates(&existing, &bundle);
        assert_eq!(d, vec![(0, existing[0].id), (3, existing[1].id)]);
    }

    #[test]
    fn registry_source_unsupported_for_other_tools() {
        let s = DetectedSource {
            id: "x".into(),
            tool: Tool::Tabby,
            path: None,
            count_hint: None,
        };
        assert!(parse_source(&s, &ImportOptions::default()).is_err());
    }

    #[test]
    fn detect_does_not_panic() {
        let _ = detect();
    }
}
