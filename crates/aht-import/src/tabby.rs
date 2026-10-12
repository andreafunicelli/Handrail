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
use saphyr::{LoadableYamlNode, Yaml};
use std::collections::HashMap;
use std::path::Path;

pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    parse_text(&read_text(path)?, opts)
}

/// Strip any `!tag`, so tagged nodes read like plain ones.
fn untag<'a, 'i>(v: &'a Yaml<'i>) -> &'a Yaml<'i> {
    match v {
        Yaml::Tagged(_, inner) => untag(inner),
        _ => v,
    }
}

/// Mapping lookup by string key; `None` if `v` is not a mapping or lacks the key.
fn get<'a, 'i>(v: &'a Yaml<'i>, k: &str) -> Option<&'a Yaml<'i>> {
    untag(v).as_mapping_get(k).map(untag)
}

fn seq<'a, 'i>(v: &'a Yaml<'i>, k: &str) -> impl Iterator<Item = &'a Yaml<'i>> {
    get(v, k)
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
        .map(untag)
}

fn s<'a>(v: &'a Yaml<'_>, k: &str) -> Option<&'a str> {
    get(v, k).and_then(Yaml::as_str)
}

/// Parse exactly one YAML document. Empty input reads as null; a stream with several
/// documents is rejected, as Tabby writes a single one.
fn load(text: &str) -> Result<Yaml<'_>> {
    let mut docs = Yaml::load_from_str(text).map_err(|e| e.to_string());
    if docs.as_ref().is_ok_and(|d| d.len() > 1) {
        docs = Err("more than one YAML document".to_string());
    }
    docs.map(|mut d| d.pop().unwrap_or(Yaml::BadValue))
        .map_err(|e| Error::Invalid(format!("invalid Tabby config.yaml: {e}")))
}

pub(crate) fn parse_text(text: &str, opts: &ImportOptions) -> Result<ImportBundle> {
    let doc = &load(text)?;
    let mut b = Builder::new(Tool::Tabby, opts);

    // id -> (name, parent id)
    let mut groups: HashMap<String, (String, Option<String>)> = HashMap::new();
    for g in seq(doc, "groups") {
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
    for p in seq(doc, "profiles") {
        if s(p, "type") != Some("ssh") {
            b.skip("non-SSH Tabby profiles");
            continue;
        }
        let pid = s(p, "id").unwrap_or("");
        if get(p, "isTemplate").and_then(Yaml::as_bool) == Some(true) || pid == "ssh:template" {
            continue;
        }
        let Some(o) = get(p, "options") else {
            continue;
        };
        let host = s(o, "host").unwrap_or("").trim();
        if host.is_empty() {
            b.skip("Tabby profiles without a host");
            continue;
        }
        let name = s(p, "name").filter(|n| !n.is_empty()).unwrap_or(host);
        let port = get(o, "port")
            .and_then(|v| {
                v.as_integer()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(str::to_string))
            })
            .and_then(|p| parse_port(&p))
            .unwrap_or(22);
        let user = s(o, "user").unwrap_or("root");
        let keys: Vec<&str> = seq(o, "privateKeys").filter_map(Yaml::as_str).collect();

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

    if get(doc, "vault").is_some_and(|v| !v.is_null()) {
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

    fn parse_str(text: &str) -> Result<ImportBundle> {
        parse_text(text, &ImportOptions::default())
    }

    fn invalid_msg(text: &str) -> String {
        match parse_str(text) {
            Err(Error::Invalid(m)) => m,
            Err(e) => panic!("expected Error::Invalid, got {e}"),
            Ok(_) => panic!("expected an error for {text:?}"),
        }
    }

    #[test]
    fn malformed_yaml_is_invalid() {
        let m = invalid_msg("profiles:\n  - id: x\n   type: [ssh\n");
        assert!(m.starts_with("invalid Tabby config.yaml: "), "{m}");
        assert!(invalid_msg("profiles: [a, b\n").contains("invalid Tabby config.yaml"));
    }

    #[test]
    fn duplicate_keys_and_multiple_documents_are_invalid() {
        invalid_msg("profiles: []\nprofiles: []\n");
        invalid_msg("profiles: []\n---\nprofiles: []\n");
    }

    #[test]
    fn empty_or_unexpected_shapes_import_nothing() {
        for text in [
            "",
            "---\n",
            "# only a comment\n",
            "just a string",
            "- 1\n- 2\n",
            "profiles: 3\ngroups: {}\n",
        ] {
            let b = parse_str(text).unwrap();
            assert!(b.connections.is_empty() && b.groups.is_empty(), "{text:?}");
            assert!(!b.warnings.iter().any(|w| w.contains("vault")), "{text:?}");
        }
        let b = parse_str("vault: null\n").unwrap();
        assert!(!b.warnings.iter().any(|w| w.contains("vault")));
    }

    #[test]
    fn scalar_shapes_ports_and_flow_style() {
        let b = parse_str(
            r#"
profiles:
  - {id: a, type: ssh, name: quoted port, options: {host: a.test, port: "2222", user: u}}
  - id: b
    type: ssh
    name: ''
    options:
      host: '  b.test  '
      port: not-a-port
  - id: c
    type: ssh
    name: 123
    options: {host: c.test, port: 70000}
  - id: d
    type: ssh
    name: tagged
    options: !opts
      host: !h d.test
      port: 0x50
      auth: password
      password: ""
  - id: e
    type: ssh
    isTemplate: true
    name: template
    options: {host: e.test}
  - id: f
    type: ssh
    name: no options
"#,
        )
        .unwrap();
        let got: Vec<_> = b
            .connections
            .iter()
            .map(|c| (c.name.as_str(), c.host.as_str(), c.port, c.user.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("quoted port", "a.test", 2222, "u"),
                ("b.test", "b.test", 22, "root"),
                ("c.test", "c.test", 22, "root"),
                ("tagged", "d.test", 80, "root"),
            ]
        );
        assert_eq!(b.connections[3].auth, SshAuth::Prompt);
    }

    #[test]
    fn anchors_and_aliases_are_resolved() {
        // js-yaml (which Tabby uses to write config.yaml) emits `&ref_N` anchors for shared objects.
        let b = parse_str(
            r#"
groups:
  - &g {id: grp, name: Shared}
profiles:
  - id: one
    type: ssh
    name: one
    group: grp
    options: &ref_0
      host: shared.test
      port: 2201
      user: ops
      auth: agent
  - id: two
    type: ssh
    name: two
    group: *g
    options: *ref_0
"#,
        )
        .unwrap();
        assert_eq!(b.connections.len(), 2);
        for c in &b.connections {
            assert_eq!(
                (c.host.as_str(), c.port, c.user.as_str()),
                ("shared.test", 2201, "ops")
            );
            assert_eq!(c.auth, SshAuth::Agent);
        }
        let g = b.groups.iter().find(|g| g.name == "Shared").unwrap();
        assert_eq!(b.connections[0].group_id, Some(g.id));
        // `group: *g` is a mapping, not a group id string, so it is ignored.
        assert_eq!(b.connections[1].group_id, None);
    }

    #[test]
    fn group_cycles_and_self_jump_are_handled() {
        let b = parse_str(
            r#"
groups:
  - {id: x, name: X, parentGroupId: y}
  - {id: y, name: Y, parentGroupId: x}
  - {name: no id}
profiles:
  - id: loop
    type: ssh
    name: loop
    group: x
    options: {host: loop.test, jumpHost: loop, privateKeys: [/nonexistent/a, /nonexistent/b]}
"#,
        )
        .unwrap();
        assert_eq!(b.connections.len(), 1);
        assert!(b.connections[0].jump.is_empty());
        assert!(b.connections[0].group_id.is_some());
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("loop") && w.contains("jump")));
        assert!(b.warnings.iter().any(|w| w.contains("first private key")));
    }
}
