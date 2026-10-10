//! SecureCRT XML export (`File > Export`).
//!
//! SecureCRT is closed source. Structure confirmed against an upstream sample export shipped in
//! mRemoteNG's test resources (`test_securecrt.xml`): `<VanDyke><key name="Sessions">` with
//! nested `<key>` folders, session keys holding `<string name="Hostname">`,
//! `<string name="Username">`, `<string name="Protocol Name">` and `<dword name="[SSH2] Port">`
//! (decimal in XML exports; zero-padded 8 digit hex as in `.ini` files is also accepted).
//! Saved passwords (`Password V2`, prefixes `02:`/`03:`) are encrypted and never imported.

use crate::builder::Builder;
use crate::text::{parse_port, read_text};
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::SshAuth;
use aht_core::{Error, Result};
use roxmltree::Node;
use std::path::Path;

pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    parse_text(&read_text(path)?, opts)
}

fn field<'a>(key: Node<'a, '_>, name: &str) -> Option<&'a str> {
    key.children()
        .find(|c| c.is_element() && c.attribute("name") == Some(name) && !c.has_tag_name("key"))
        .map(|c| c.text().unwrap_or("").trim())
}

fn dword(s: &str) -> Option<u32> {
    let s = s.trim();
    let hexish = s.len() == 8
        && s.chars().all(|c| c.is_ascii_hexdigit())
        && (s.starts_with('0') || s.chars().any(|c| c.is_ascii_alphabetic()));
    if hexish {
        u32::from_str_radix(s, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn is_session(key: Node) -> bool {
    field(key, "Protocol Name").is_some() || field(key, "Hostname").is_some()
}

pub(crate) fn parse_text(text: &str, opts: &ImportOptions) -> Result<ImportBundle> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| Error::Invalid(format!("invalid SecureCRT XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "VanDyke" {
        return Err(Error::Invalid("not a SecureCRT XML export".into()));
    }
    let mut b = Builder::new(Tool::SecureCrt, opts);
    let mut saw_password = false;
    if let Some(sessions) = root
        .children()
        .find(|c| c.has_tag_name("key") && c.attribute("name") == Some("Sessions"))
    {
        walk(sessions, &mut Vec::new(), &mut b, &mut saw_password);
    }
    if saw_password {
        b.warn("SecureCRT saved passwords are encrypted and were not imported");
    }
    Ok(b.finish())
}

fn walk(parent: Node, path: &mut Vec<String>, b: &mut Builder, saw_password: &mut bool) {
    for key in parent.children().filter(|c| c.has_tag_name("key")) {
        let name = key.attribute("name").unwrap_or("").to_string();
        if name.starts_with("__") {
            continue; // e.g. __FolderData__
        }
        if !is_session(key) {
            path.push(name);
            b.group_path(path);
            walk(key, path, b, saw_password);
            path.pop();
            continue;
        }
        let proto = field(key, "Protocol Name").unwrap_or("");
        if !(proto.eq_ignore_ascii_case("SSH2") || proto.eq_ignore_ascii_case("SSH1")) {
            if !(path.is_empty() && name == "Default") {
                b.skip("non-SSH SecureCRT sessions");
            }
            continue;
        }
        let host = field(key, "Hostname").unwrap_or("");
        if host.is_empty() {
            if name != "Default" {
                b.skip("SecureCRT sessions without a host");
            }
            continue;
        }
        let port_field = if proto.eq_ignore_ascii_case("SSH1") {
            "[SSH1] Port"
        } else {
            "[SSH2] Port"
        };
        let port = field(key, port_field)
            .or_else(|| field(key, "Port"))
            .and_then(dword)
            .and_then(|p| parse_port(&p.to_string()))
            .unwrap_or(22);
        let user = field(key, "Username").unwrap_or("");
        if field(key, "Password V2").is_some_and(|p| !p.is_empty()) {
            *saw_password = true;
        }
        let firewall = field(key, "Firewall Name").unwrap_or("None");
        if !firewall.is_empty() && !firewall.eq_ignore_ascii_case("None") {
            b.warn(format!(
                "{name}: firewall/jump host '{firewall}' is not imported"
            ));
        }
        let mut notes = String::new();
        let auth = match field(key, "Identity Filename V2").filter(|f| !f.is_empty()) {
            Some(f) => b.key_auth(&name, f, &mut notes),
            None => SshAuth::Prompt,
        };
        let group = b.group_path(path);
        b.add(&name, group, host, port, user, auth, notes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_dir;

    fn parse() -> (ImportBundle, tempfile::TempDir) {
        let tmp = stage_dir("securecrt");
        let b = parse_file(&tmp.path().join("sessions.xml"), &ImportOptions::default()).unwrap();
        (b, tmp)
    }

    #[test]
    fn sessions_folders_ports_and_users() {
        let (b, _t) = parse();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "gamma"]);
        let a = &b.connections[0];
        assert_eq!(
            (a.host.as_str(), a.port, a.user.as_str()),
            ("alpha.example.test", 22, "ops")
        );
        // Hex-encoded dword (as in .ini exports) is understood too.
        assert_eq!(b.connections[1].port, 2222);
        assert_eq!(b.connections[2].port, 2022);
        let lab = b.groups.iter().find(|g| g.name == "Lab").unwrap();
        let dc = b.groups.iter().find(|g| g.name == "DC1").unwrap();
        assert_eq!(dc.parent_id, Some(lab.id));
        assert_eq!(b.connections[1].group_id, Some(dc.id));
        assert!(b.groups.iter().all(|g| !g.name.starts_with("__")));
    }

    #[test]
    fn skips_non_ssh_and_warns_about_passwords_and_keys() {
        let (b, _t) = parse();
        assert!(b.warnings.iter().any(|w| w.contains("Skipped 2 non-SSH")));
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("passwords are encrypted")));
        assert!(matches!(b.connections[2].auth, SshAuth::Key { .. }));
        assert_eq!(b.connections[0].auth, SshAuth::Prompt);
        assert!(b.warnings.iter().any(|w| w.contains("firewall")));
    }

    #[test]
    fn rejects_other_xml() {
        assert!(parse_text("<x/>", &ImportOptions::default()).is_err());
    }

    #[test]
    fn dword_forms() {
        assert_eq!(dword("22"), Some(22));
        assert_eq!(dword("00000016"), Some(22));
        assert_eq!(dword("16777215"), Some(16_777_215));
    }
}
