//! WinSCP sessions from `WinSCP.ini`, the Windows registry or a `.reg` export.
//!
//! Verified against the WinSCP sources (`source/core/Security.cpp`, `SessionData.cpp`,
//! `Security.h`): `PWALG_SIMPLE_MAGIC = 0xA3`, `PWALG_SIMPLE_FLAG = 0xFF`, the password is
//! obfuscated (not encrypted) as `username + hostname + password`, and `FSProtocol` values are
//! 0 = SCP, 1 = SFTP (+SCP fallback), 2 = SFTP, 5 = FTP, 6 = WebDAV, 7 = S3. A missing
//! `FSProtocol` means the default, SFTP. Sessions are stored by a percent-escaped name where `/`
//! separates folders.

use crate::builder::Builder;
use crate::ini;
use crate::reg::{self, get_str, get_u32, Fields, RegValue};
use crate::text::{parse_port, percent_decode, read_text};
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::SshAuth;
use aht_core::{Error, Result};
use std::path::Path;

#[cfg(windows)]
const REG_SESSIONS: &str = r"Software\Martin Prikryl\WinSCP 2\Sessions";

const MAGIC: u8 = 0xA3;
const FLAG: u8 = 0xFF;

/// Reverse of WinSCP's `DecryptPassword` (simple obfuscation). `key` is `UserName + HostName`.
pub(crate) fn deobfuscate(hex: &str, key: &str) -> Option<String> {
    let h = hex.trim().as_bytes();
    if h.is_empty() || h.len() & 1 == 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(h.len() / 2);
    for pair in h.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        bytes.push(!(((hi << 4) | lo) as u8 ^ MAGIC));
    }
    let mut it = bytes.into_iter();
    let mut next = || it.next().unwrap_or(0);
    let flag = next();
    let len = if flag == FLAG {
        match next() {
            0 => next() as usize,
            2 => ((next() as usize) << 8) + next() as usize,
            _ => return None,
        }
    } else {
        flag as usize
    };
    let shift = next() as usize;
    for _ in 0..shift {
        next();
    }
    let data: Vec<u8> = (0..len).map(|_| next()).collect();
    let text = String::from_utf8(data).ok()?;
    if flag == FLAG {
        text.strip_prefix(key).map(str::to_string)
    } else {
        Some(text)
    }
}

#[cfg(test)]
pub(crate) fn obfuscate(password: &str, key: &str, shift: u8) -> String {
    let enc = |c: u8| format!("{:02X}", !c ^ MAGIC);
    let plain = format!("{key}{password}");
    let mut out = enc(FLAG);
    if plain.len() > 255 {
        out += &enc(2);
        out += &enc((plain.len() >> 8) as u8);
        out += &enc((plain.len() & 0xFF) as u8);
    } else {
        out += &enc(0);
        out += &enc(plain.len() as u8);
    }
    out += &enc(shift);
    for i in 0..shift {
        out += &enc(i.wrapping_mul(37));
    }
    for b in plain.bytes() {
        out += &enc(b);
    }
    while out.len() < 100 {
        out += &enc(0x5A);
    }
    out
}

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
        .ok_or_else(|| Error::NotFound("WinSCP registry sessions".into()))?;
    Ok(from_sessions(
        raw.into_iter()
            .map(|(n, f)| (percent_decode(&n), f))
            .collect(),
        false,
        opts,
    ))
}

/// `path` is `WinSCP.ini` or a `.reg` export containing the WinSCP `Sessions` key.
pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    let text = read_text(path)?;
    if text.contains("[HKEY_") {
        return Ok(from_reg_text(&text, opts));
    }
    Ok(from_ini_text(&text, opts))
}

pub(crate) fn from_reg_text(text: &str, opts: &ImportOptions) -> ImportBundle {
    let marker = "\\winscp 2\\sessions\\";
    let mut sessions = Vec::new();
    for k in reg::parse(text) {
        if let Some(i) = k.path.to_lowercase().find(marker) {
            let name = &k.path[i + marker.len()..];
            if !name.contains('\\') {
                sessions.push((percent_decode(name), k.values));
            }
        }
    }
    from_sessions(sessions, false, opts)
}

pub(crate) fn from_ini_text(text: &str, opts: &ImportOptions) -> ImportBundle {
    let sections = ini::parse(text);
    let master = sections.iter().any(|s| {
        s.name.eq_ignore_ascii_case("Configuration\\Security")
            && s.get("UseMasterPassword") == Some("1")
    });
    let mut sessions = Vec::new();
    for s in &sections {
        let Some(name) = s.name.strip_prefix("Sessions\\") else {
            continue;
        };
        if name.contains('\\') {
            continue;
        }
        let f: Fields = s
            .entries
            .iter()
            .map(|(k, v)| {
                // Passwords are hex and must stay untouched; other strings may be escaped.
                let v = if k.eq_ignore_ascii_case("Password") {
                    v.clone()
                } else {
                    percent_decode(v)
                };
                (k.clone(), RegValue::Str(v))
            })
            .collect();
        sessions.push((percent_decode(name), f));
    }
    from_sessions(sessions, master, opts)
}

fn from_sessions(
    sessions: Vec<(String, Fields)>,
    master_password: bool,
    opts: &ImportOptions,
) -> ImportBundle {
    let mut b = Builder::new(Tool::WinScp, opts);
    if master_password {
        b.warn("WinSCP master password is enabled; saved passwords were not imported");
    }
    for (full_name, f) in sessions {
        if full_name == "Default Settings" {
            continue;
        }
        let fs = get_u32(&f, "FSProtocol").unwrap_or(1);
        if !matches!(fs, 0..=2) {
            b.skip("WinSCP sessions using FTP/WebDAV/S3");
            continue;
        }
        let host = get_str(&f, "HostName").unwrap_or_default();
        let host = host.trim();
        if host.is_empty() {
            b.skip("WinSCP sessions without a host");
            continue;
        }
        let user = get_str(&f, "UserName").unwrap_or_default();
        let port = get_u32(&f, "PortNumber")
            .and_then(|p| parse_port(&p.to_string()))
            .unwrap_or(22);

        let mut parts: Vec<String> = full_name.split('/').map(str::to_string).collect();
        let name = parts
            .pop()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| host.to_string());
        let group = b.group_path(&parts);

        let mut notes = String::new();
        let mut auth = SshAuth::Prompt;
        if let Some(k) = get_str(&f, "PublicKeyFile").filter(|k| !k.is_empty()) {
            auth = b.key_auth(&name, &k, &mut notes);
        } else if let Some(hex) = get_str(&f, "Password").filter(|p| !p.is_empty()) {
            if !master_password {
                match deobfuscate(&hex, &format!("{user}{host}")) {
                    Some(pw) => auth = b.password_auth(&name, &pw),
                    None => b.warn(format!("{name}: saved password could not be decoded")),
                }
            }
        }
        if get_str(&f, "Tunnel").as_deref() == Some("1") {
            b.warn(format!("{name}: SSH tunnel setting is not imported"));
        }
        b.add(&name, group, host, port, &user, auth, notes);
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_dir;

    #[test]
    fn obfuscation_roundtrip_short_long_and_unicode() {
        for (pw, shift) in [("hunter2", 0u8), ("pässwörd✓", 7), ("x", 30)] {
            let enc = obfuscate(pw, "userhost.example.test", shift);
            assert_eq!(
                deobfuscate(&enc, "userhost.example.test").as_deref(),
                Some(pw)
            );
        }
        let long = "a".repeat(300);
        let enc = obfuscate(&long, "k", 0);
        assert_eq!(deobfuscate(&enc, "k").as_deref(), Some(long.as_str()));
    }

    #[test]
    fn wrong_key_or_garbage_is_rejected() {
        let enc = obfuscate("pw", "alicehost", 3);
        assert_eq!(deobfuscate(&enc, "bobhost"), None);
        assert_eq!(deobfuscate("zz", "k"), None);
        assert_eq!(deobfuscate("", "k"), None);
    }

    #[test]
    fn known_vector_matches_documented_algorithm() {
        // 0xFF -> "A3"^... : flag 0xFF encodes to !0xFF ^ 0xA3 = 0xA3; check by hand.
        assert_eq!(&obfuscate("", "", 0)[..2], "A3");
        // length byte 0x03 for key "abc" + empty password => !0x03 ^ 0xA3 = 0x5F.
        assert_eq!(&obfuscate("", "abc", 0)[4..6], "5F");
    }

    fn fixture_bundle() -> (ImportBundle, tempfile::TempDir) {
        let tmp = stage_dir("winscp");
        // Fixture holds `{{PW1}}` placeholders; fill with freshly obfuscated values.
        let ini_path = tmp.path().join("WinSCP.ini");
        let t = std::fs::read_to_string(&ini_path)
            .unwrap()
            .replace("{{PW1}}", &obfuscate("s3cret", "adminweb.example.test", 5))
            .replace("{{PW2}}", &obfuscate("zzz", "someoneother", 0));
        std::fs::write(&ini_path, t).unwrap();
        let b = parse_file(&ini_path, &ImportOptions::default()).unwrap();
        (b, tmp)
    }

    #[test]
    fn ini_sessions_folders_and_passwords() {
        let (b, _t) = fixture_bundle();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["web", "scp box", "keyed", "badpw"]);
        let web = &b.connections[0];
        assert_eq!(
            (web.host.as_str(), web.port, web.user.as_str()),
            ("web.example.test", 2022, "admin")
        );
        let SshAuth::Password { secret } = &web.auth else {
            panic!("{:?}", web.auth)
        };
        assert_eq!(
            b.secrets.iter().find(|s| s.id == *secret).unwrap().value,
            "s3cret"
        );
        // Folder "Work/Sub" => Work > Sub.
        let sub = b.groups.iter().find(|g| g.name == "Sub").unwrap();
        let work = b.groups.iter().find(|g| g.name == "Work").unwrap();
        assert_eq!(sub.parent_id, Some(work.id));
        assert_eq!(web.group_id, Some(sub.id));
        assert_eq!(b.connections[1].group_id, Some(work.id));
        assert_eq!(b.connections[1].port, 22);
    }

    #[test]
    fn non_ssh_skipped_and_bad_password_warns() {
        let (b, _t) = fixture_bundle();
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("Skipped 2 WinSCP sessions using")));
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("badpw") && w.contains("decoded")));
        assert_eq!(b.connections[3].auth, SshAuth::Prompt);
        assert!(matches!(b.connections[2].auth, SshAuth::Key { .. }));
    }

    #[test]
    fn master_password_skips_passwords() {
        let t = "[Configuration\\Security]\nUseMasterPassword=1\n[Sessions\\a]\nHostName=h\nUserName=u\nPassword=AABB\n";
        let b = from_ini_text(t, &ImportOptions::default());
        assert_eq!(b.connections[0].auth, SshAuth::Prompt);
        assert!(b.warnings.iter().any(|w| w.contains("master password")));
    }

    #[test]
    fn reg_export_works_too() {
        let t = format!("Windows Registry Editor Version 5.00\n\n[HKEY_CURRENT_USER\\Software\\Martin Prikryl\\WinSCP 2\\Sessions\\Dir%2FRegBox]\n\"HostName\"=\"h.example.test\"\n\"UserName\"=\"r\"\n\"PortNumber\"=dword:0000015e\n\"Password\"=\"{}\"\n", obfuscate("pw!", "rh.example.test", 2));
        let b = from_reg_text(&t, &ImportOptions::default());
        assert_eq!(b.connections[0].port, 350);
        assert_eq!(b.groups[0].name, "Dir");
        assert!(matches!(b.connections[0].auth, SshAuth::Password { .. }));
    }
}
