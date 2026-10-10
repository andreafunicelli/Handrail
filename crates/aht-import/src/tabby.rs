//! Tabby `config.yaml`.
//!
//! Verified against Eugeny/tabby sources: `tabby-ssh/src/api/interfaces.ts` (`SSHProfileOptions`:
//! `host`, `port`, `user`, `auth: null|'password'|'publicKey'|'agent'|'keyboardInteractive'`,
//! `password`, `privateKeys`, `jumpHost` = another profile id) and
//! `tabby-core/src/api/profileProvider.ts` (`Profile.group`, `ProfileGroup { id, name,
//! parentGroupId }`). Saved secrets live in Tabby's encrypted vault and are not imported.

use crate::builder::Builder;
use crate::text::{parse_port, read_text};
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::SshAuth;
use aht_core::{ConnectionId, Error, Result};
use serde_yaml_ng::Value;
use std::collections::HashMap;
use std::path::Path;

pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    parse_text(&read_text(path)?, opts)
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

pub(crate) fn parse_text(text: &str, opts: &ImportOptions) -> Result<ImportBundle> {
    let doc: Value = serde_yaml_ng::from_str(text)
        .map_err(|e| Error::Invalid(format!("invalid Tabby config.yaml: {e}")))?;
    let mut b = Builder::new(Tool::Tabby, opts);

    // id -> (name, parent id)
    let mut groups: HashMap<String, (String, Option<String>)> = HashMap::new();
    for g in doc
        .get("groups")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
    {
        if let (Some(id), Some(name)) = (s(g, "id"), s(g, "name")) {
            groups.insert(
                id.to_string(),
                (name.to_string(), s(g, "parentGroupId").map(str::to_string)),
            );
        }
    }
    let group_chain = |start: &str| -> Vec<String> {
        let mut id = start.to_string();
        let mut chain = Vec::new();
        while let Some((name, parent)) = groups.get(&id) {
            chain.push(name.clone());
            if chain.len() > 32 {
                break; // cycle guard
            }
            match parent {
                Some(p) => id = p.clone(),
                None => break,
            }
        }
        chain.reverse();
        chain
    };

    let mut by_profile_id: HashMap<String, ConnectionId> = HashMap::new();
    let mut jumps: Vec<(ConnectionId, String, String)> = Vec::new();
    for p in doc
        .get("profiles")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
    {
        if s(p, "type") != Some("ssh") {
            b.skip("non-SSH Tabby profiles");
            continue;
        }
        let pid = s(p, "id").unwrap_or("");
        if p.get("isTemplate").and_then(Value::as_bool) == Some(true) || pid == "ssh:template" {
            continue;
        }
        let Some(o) = p.get("options") else {
            continue;
        };
        let host = s(o, "host").unwrap_or("").trim();
        if host.is_empty() {
            b.skip("Tabby profiles without a host");
            continue;
        }
        let name = s(p, "name").filter(|n| !n.is_empty()).unwrap_or(host);
        let port = o
            .get("port")
            .and_then(|v| {
                v.as_u64()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(str::to_string))
            })
            .and_then(|p| parse_port(&p))
            .unwrap_or(22);
        let user = s(o, "user").unwrap_or("root");
        let keys: Vec<&str> = o
            .get("privateKeys")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();

        let mut notes = String::new();
        let auth = match s(o, "auth") {
            Some("agent") => SshAuth::Agent,
            Some("password") => match s(o, "password").filter(|p| !p.is_empty()) {
                Some(pw) => b.password_auth(name, pw),
                None => SshAuth::Prompt,
            },
            Some("publicKey") => match keys.first() {
                Some(k) => b.key_auth(name, k, &mut notes),
                None => SshAuth::Prompt,
            },
            _ => match keys.first() {
                Some(k) => b.key_auth(name, k, &mut notes),
                None => SshAuth::Prompt,
            },
        };
        if keys.len() > 1 {
            b.warn(format!("{name}: only the first private key was imported"));
        }
        let group = s(p, "group")
            .filter(|g| !g.is_empty())
            .map(group_chain)
            .and_then(|chain| b.group_path(&chain));
        let id = b.add(name, group, host, port, user, auth, notes);
        if !pid.is_empty() {
            by_profile_id.insert(pid.to_string(), id);
        }
        if let Some(j) = s(o, "jumpHost").filter(|j| !j.is_empty()) {
            jumps.push((id, name.to_string(), j.to_string()));
        }
    }

    for (id, name, target) in jumps {
        match by_profile_id.get(&target) {
            Some(j) if *j != id => {
                if let Some(c) = b.connections.iter_mut().find(|c| c.id == id) {
                    c.jump = vec![*j];
                }
            }
            _ => b.warn(format!(
                "{name}: jump host '{target}' is not an imported SSH profile"
            )),
        }
    }

    if doc.get("vault").is_some_and(|v| !v.is_null()) {
        b.warn("Tabby vault contents (saved passwords/passphrases) are encrypted and were not imported");
    }
    Ok(b.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_dir;

    fn parse() -> (ImportBundle, tempfile::TempDir) {
        let tmp = stage_dir("tabby");
        let b = parse_file(&tmp.path().join("config.yaml"), &ImportOptions::default()).unwrap();
        (b, tmp)
    }

    #[test]
    fn profiles_groups_and_auth_modes() {
        let (b, _t) = parse();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["bastion", "app", "agent host", "pw host"]);
        let app = &b.connections[1];
        assert_eq!(
            (app.host.as_str(), app.port, app.user.as_str()),
            ("app.example.test", 2200, "deploy")
        );
        assert!(matches!(app.auth, SshAuth::Key { .. }));
        assert_eq!(b.connections[2].auth, SshAuth::Agent);
        assert_eq!(b.connections[3].auth, SshAuth::Prompt);
        // Nested group: Infra > Prod.
        let prod = b.groups.iter().find(|g| g.name == "Prod").unwrap();
        let infra = b.groups.iter().find(|g| g.name == "Infra").unwrap();
        assert_eq!(prod.parent_id, Some(infra.id));
        assert_eq!(app.group_id, Some(prod.id));
        assert_eq!(b.connections[0].group_id, Some(infra.id));
        assert_eq!(b.connections[2].group_id, None);
    }

    #[test]
    fn jump_host_resolved_and_skips_noted() {
        let (b, _t) = parse();
        assert_eq!(b.connections[1].jump, vec![b.connections[0].id]);
        assert!(b.warnings.iter().any(|w| w.contains("Skipped 1 non-SSH")));
        assert!(b.warnings.iter().any(|w| w.contains("vault")));
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("pw host") && w.contains("jump")));
    }
}
