//! PuTTY saved sessions: Windows registry, `.reg` exports and Unix `~/.putty/sessions`.
//!
//! Key names (`HostName`, `PortNumber`, `Protocol`, `UserName`, `PublicKeyFile`) verified against
//! PuTTY's `settings.c`. Session names are percent-escaped by PuTTY's `mungestr`.

use crate::builder::Builder;
use crate::reg::{self, get_str, get_u32, Fields, RegValue};
use crate::text::{parse_port, percent_decode, read_text, split_user_host};
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::SshAuth;
use aht_core::{Error, Result};
use std::path::Path;

#[cfg(windows)]
const REG_SESSIONS: &str = r"Software\SimonTatham\PuTTY\Sessions";

#[cfg(windows)]
pub fn registry_count() -> Option<usize> {
    let s = reg::read_hkcu_subkeys(REG_SESSIONS)?;
    Some(
        s.iter()
            .filter(|(n, _)| percent_decode(n) != "Default Settings")
            .count(),
    )
}

#[cfg(windows)]
pub fn parse_registry(opts: &ImportOptions) -> Result<ImportBundle> {
    let raw = reg::read_hkcu_subkeys(REG_SESSIONS)
        .ok_or_else(|| Error::NotFound("PuTTY registry sessions".into()))?;
    Ok(from_sessions(
        raw.into_iter()
            .map(|(n, f)| (percent_decode(&n), f))
            .collect(),
        opts,
    ))
}

/// `path` is a `.reg` export, or a directory of Unix session files.
pub fn parse_path(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    if path.is_dir() {
        let mut sessions = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(path)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if !e.path().is_file() {
                continue;
            }
            let name = percent_decode(&e.file_name().to_string_lossy());
            sessions.push((name, parse_unix_file(&read_text(&e.path())?)));
        }
        return Ok(from_sessions(sessions, opts));
    }
    let text = read_text(path)?;
    if !text.contains("[HKEY_") {
        return Err(Error::Invalid(
            "not a PuTTY .reg export or sessions directory".into(),
        ));
    }
    Ok(from_reg_text(&text, opts))
}

pub(crate) fn from_reg_text(text: &str, opts: &ImportOptions) -> ImportBundle {
    let mut sessions = Vec::new();
    for k in reg::parse(text) {
        let lower = k.path.to_lowercase();
        let marker = "\\putty\\sessions\\";
        if let Some(i) = lower.find(marker) {
            let name = &k.path[i + marker.len()..];
            if !name.contains('\\') {
                sessions.push((percent_decode(name), k.values));
            }
        }
    }
    from_sessions(sessions, opts)
}

fn parse_unix_file(text: &str) -> Fields {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            (
                k.to_string(),
                RegValue::Str(v.trim_end_matches('\r').to_string()),
            )
        })
        .collect()
}

fn from_sessions(sessions: Vec<(String, Fields)>, opts: &ImportOptions) -> ImportBundle {
    let mut b = Builder::new(Tool::Putty, opts);
    for (name, f) in sessions {
        if name == "Default Settings" {
            continue;
        }
        let proto = get_str(&f, "Protocol").unwrap_or_default().to_lowercase();
        if proto != "ssh" {
            b.skip("non-SSH PuTTY sessions");
            continue;
        }
        let raw_host = get_str(&f, "HostName").unwrap_or_default();
        let (host_user, host) = split_user_host(raw_host.trim());
        if host.is_empty() {
            b.skip("PuTTY sessions without a host name");
            continue;
        }
        let user = get_str(&f, "UserName")
            .filter(|u| !u.is_empty())
            .or_else(|| host_user.map(str::to_string))
            .unwrap_or_default();
        let port = get_u32(&f, "PortNumber")
            .and_then(|p| parse_port(&p.to_string()))
            .unwrap_or(22);
        let mut notes = String::new();
        let auth = match get_str(&f, "PublicKeyFile").filter(|k| !k.is_empty()) {
            Some(k) => b.key_auth(&name, &k, &mut notes),
            None if get_u32(&f, "TryAgent") == Some(1) => SshAuth::Agent,
            None => SshAuth::Prompt,
        };
        b.add(&name, None, host, port, &user, auth, notes);
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fixture, stage_dir};

    fn from_fixture() -> (ImportBundle, tempfile::TempDir) {
        let tmp = stage_dir("putty");
        let b = parse_path(&tmp.path().join("sessions.reg"), &ImportOptions::default()).unwrap();
        (b, tmp)
    }

    #[test]
    fn reg_export_sessions() {
        let (b, _t) = from_fixture();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["My Server", "agent box", "pem user"]);
        let s = &b.connections[0];
        assert_eq!(
            (s.host.as_str(), s.port, s.user.as_str()),
            ("10.1.2.3", 2200, "alice")
        );
        // user@host form in HostName.
        let a = &b.connections[1];
        assert_eq!(
            (a.host.as_str(), a.user.as_str(), a.port),
            ("agent.example.test", "bob", 22)
        );
        assert_eq!(a.auth, SshAuth::Agent);
    }

    #[test]
    fn ppk_warns_pem_imports_non_ssh_skipped() {
        let (b, _t) = from_fixture();
        assert_eq!(b.connections[0].auth, SshAuth::Prompt);
        assert!(b.warnings.iter().any(|w| w.contains("puttygen")));
        assert!(matches!(b.connections[2].auth, SshAuth::Key { .. }));
        assert!(b.warnings.iter().any(|w| w.contains("Skipped 2 non-SSH")));
        assert!(b.connections.iter().all(|c| c.name != "Default Settings"));
    }

    #[test]
    fn unix_sessions_directory() {
        let b = parse_path(&fixture("putty/unix"), &ImportOptions::default()).unwrap();
        assert_eq!(b.connections.len(), 1);
        assert_eq!(b.connections[0].name, "unix box");
        assert_eq!(b.connections[0].port, 2022);
    }

    #[test]
    fn utf16_reg_file_is_decoded() {
        let text = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\u16]\r\n\"HostName\"=\"h.example.test\"\r\n\"Protocol\"=\"ssh\"\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.reg");
        std::fs::write(&p, bytes).unwrap();
        let b = parse_path(&p, &ImportOptions::default()).unwrap();
        assert_eq!(b.connections[0].host, "h.example.test");
    }
}
