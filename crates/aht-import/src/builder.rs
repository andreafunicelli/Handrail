//! Shared bundle builder: nested groups, secrets, key files and skip accounting.

use crate::text::expand_tilde;
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::{
    Connection, ConnectionKind, Environment, Group, Secret, SecretKind, SshAuth,
};
use aht_core::{ConnectionId, GroupId, SecretId};
use base64::Engine;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub struct Builder {
    pub tool: Tool,
    pub groups: Vec<Group>,
    pub connections: Vec<Connection>,
    pub secrets: Vec<Secret>,
    pub warnings: Vec<String>,
    pub import_passwords: bool,
    group_index: HashMap<Vec<String>, GroupId>,
    key_cache: HashMap<PathBuf, SecretId>,
    skipped: BTreeMap<String, usize>,
}

impl Builder {
    pub fn new(tool: Tool, opts: &ImportOptions) -> Self {
        Self {
            tool,
            groups: Vec::new(),
            connections: Vec::new(),
            secrets: Vec::new(),
            warnings: Vec::new(),
            import_passwords: opts.import_passwords,
            group_index: HashMap::new(),
            key_cache: HashMap::new(),
            skipped: BTreeMap::new(),
        }
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if !self.warnings.contains(&m) {
            self.warnings.push(m);
        }
    }

    /// Count a skipped (non-SSH or unusable) entry; summarised in `finish`.
    pub fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_insert(0) += 1;
    }

    /// Get or create the nested group for `path` (empty path = no group).
    pub fn group_path(&mut self, path: &[String]) -> Option<GroupId> {
        let path: Vec<String> = path
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut parent: Option<GroupId> = None;
        for depth in 1..=path.len() {
            let key = path[..depth].to_vec();
            if let Some(id) = self.group_index.get(&key) {
                parent = Some(*id);
                continue;
            }
            let order = self.groups.iter().filter(|g| g.parent_id == parent).count() as i32;
            let id = GroupId::new();
            self.groups.push(Group {
                id,
                name: path[depth - 1].clone(),
                parent_id: parent,
                order,
            });
            self.group_index.insert(key, id);
            parent = Some(id);
        }
        parent
    }

    pub fn secret(&mut self, kind: SecretKind, label: String, value: String) -> SecretId {
        let id = SecretId::new();
        self.secrets.push(Secret {
            id,
            kind,
            label,
            value,
        });
        id
    }

    /// Auth for a saved password. Empty password or `import_passwords == false` => prompt.
    pub fn password_auth(&mut self, conn_name: &str, password: &str) -> SshAuth {
        if password.is_empty() || !self.import_passwords {
            return SshAuth::Prompt;
        }
        let id = self.secret(
            SecretKind::Password,
            format!("{conn_name} password"),
            password.to_string(),
        );
        SshAuth::Password { secret: id }
    }

    /// Auth for an identity file path. Reads OpenSSH/PEM keys; `.ppk` and unreadable files
    /// fall back to `Prompt` with a warning and a note on the connection.
    pub fn key_auth(&mut self, conn_name: &str, path: &str, notes: &mut String) -> SshAuth {
        let note = |notes: &mut String| {
            if !notes.is_empty() {
                notes.push('\n');
            }
            notes.push_str(&format!("Identity file: {path}"));
        };
        let p = expand_tilde(path);
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ppk")) {
            self.warn(format!(
                "{conn_name}: PuTTY key '{path}' not imported; convert with puttygen to OpenSSH format"
            ));
            note(notes);
            return SshAuth::Prompt;
        }
        if !self.import_passwords {
            note(notes);
            return SshAuth::Prompt;
        }
        if let Some(id) = self.key_cache.get(&p) {
            return SshAuth::Key {
                key: *id,
                passphrase: None,
            };
        }
        let content = match std::fs::read(&p) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(_) => {
                self.warn(format!("{conn_name}: key file '{path}' not found"));
                note(notes);
                return SshAuth::Prompt;
            }
        };
        if content.contains("PuTTY-User-Key-File") {
            self.warn(format!(
                "{conn_name}: PuTTY key '{path}' not imported; convert with puttygen to OpenSSH format"
            ));
            note(notes);
            return SshAuth::Prompt;
        }
        if !content.trim_start().starts_with("-----BEGIN") {
            self.warn(format!(
                "{conn_name}: '{path}' is not an OpenSSH/PEM private key"
            ));
            note(notes);
            return SshAuth::Prompt;
        }
        if key_is_encrypted(&content) {
            self.warn(format!(
                "{conn_name}: key '{path}' is passphrase-protected; you will be asked for the passphrase"
            ));
        }
        let id = self.secret(SecretKind::PrivateKey, format!("{conn_name} key"), content);
        self.key_cache.insert(p, id);
        SshAuth::Key {
            key: id,
            passphrase: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        name: &str,
        group: Option<GroupId>,
        host: &str,
        port: u16,
        user: &str,
        auth: SshAuth,
        notes: String,
    ) -> ConnectionId {
        let id = ConnectionId::new();
        self.connections.push(Connection {
            id,
            name: name.to_string(),
            kind: ConnectionKind::Ssh,
            group_id: group,
            host: host.to_string(),
            port,
            user: user.to_string(),
            auth,
            jump: Vec::new(),
            env: Environment::None,
            tags: Vec::new(),
            color: None,
            approval_mode: None,
            local_shell: None,
            notes,
            tunnels: Vec::new(),
            rdp_admin: false,
            icon: None,
        });
        id
    }

    pub fn finish(mut self) -> ImportBundle {
        let skipped = std::mem::take(&mut self.skipped);
        for (what, n) in skipped {
            self.warnings.push(format!("Skipped {n} {what}"));
        }
        ImportBundle {
            tool: self.tool,
            groups: self.groups,
            connections: self.connections,
            secrets: self.secrets,
            warnings: self.warnings,
        }
    }
}

/// True for PEM keys with an `ENCRYPTED` header and OpenSSH keys with a non-`none` cipher.
fn key_is_encrypted(pem: &str) -> bool {
    if pem.contains("ENCRYPTED") {
        return true;
    }
    if !pem.contains("BEGIN OPENSSH PRIVATE KEY") {
        return false;
    }
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(body.trim()) else {
        return false;
    };
    let magic = b"openssh-key-v1\0";
    if !bytes.starts_with(magic) || bytes.len() < magic.len() + 4 {
        return false;
    }
    let rest = &bytes[magic.len()..];
    let n = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    rest.get(4..4 + n).is_some_and(|c| c != b"none")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_groups_have_parent_links_and_are_reused() {
        let mut b = Builder::new(Tool::MobaXterm, &ImportOptions::default());
        let leaf = b
            .group_path(&["Prod".into(), "EU".into(), "DB".into()])
            .unwrap();
        let again = b
            .group_path(&["Prod".into(), "EU".into(), "DB".into()])
            .unwrap();
        let eu = b.group_path(&["Prod".into(), "EU".into()]).unwrap();
        let us = b.group_path(&["Prod".into(), "US".into()]).unwrap();
        assert_eq!(leaf, again);
        assert_eq!(b.groups.len(), 4);
        let g = |id| b.groups.iter().find(|g| g.id == id).unwrap();
        assert_eq!(g(leaf).parent_id, Some(eu));
        assert_eq!(
            g(eu).parent_id.map(|p| g(p).name.clone()),
            Some("Prod".into())
        );
        assert_eq!(g(g(eu).parent_id.unwrap()).parent_id, None);
        assert_eq!((g(eu).order, g(us).order), (0, 1));
        assert_eq!(b.group_path(&[]), None);
    }

    #[test]
    fn key_files_are_imported_ppk_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let pem = dir.path().join("id_test");
        std::fs::write(
            &pem,
            "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----\n",
        )
        .unwrap();
        let ppk = dir.path().join("k.ppk");
        std::fs::write(&ppk, "PuTTY-User-Key-File-3: ssh-ed25519\n").unwrap();
        let mut b = Builder::new(Tool::Putty, &ImportOptions::default());
        let mut notes = String::new();
        let a = b.key_auth("h", pem.to_str().unwrap(), &mut notes);
        assert!(matches!(a, SshAuth::Key { .. }));
        // Same key twice shares one secret.
        b.key_auth("h2", pem.to_str().unwrap(), &mut notes);
        assert_eq!(b.secrets.len(), 1);
        let a = b.key_auth("h", ppk.to_str().unwrap(), &mut notes);
        assert_eq!(a, SshAuth::Prompt);
        assert!(b.warnings.iter().any(|w| w.contains("puttygen")));
        assert!(notes.contains("k.ppk"));
    }

    #[test]
    fn encrypted_openssh_key_detected() {
        // "openssh-key-v1\0" + len(4) "aes256-ctr"
        let mut raw = b"openssh-key-v1\0".to_vec();
        raw.extend_from_slice(&10u32.to_be_bytes());
        raw.extend_from_slice(b"aes256-ctr");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
        let pem = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{b64}\n-----END OPENSSH PRIVATE KEY-----"
        );
        assert!(key_is_encrypted(&pem));
        let mut raw = b"openssh-key-v1\0".to_vec();
        raw.extend_from_slice(&4u32.to_be_bytes());
        raw.extend_from_slice(b"none");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
        let pem = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{b64}\n-----END OPENSSH PRIVATE KEY-----"
        );
        assert!(!key_is_encrypted(&pem));
    }

    #[test]
    fn passwords_respect_option() {
        let mut b = Builder::new(
            Tool::Putty,
            &ImportOptions {
                import_passwords: false,
                ..Default::default()
            },
        );
        assert_eq!(b.password_auth("x", "pw"), SshAuth::Prompt);
        assert!(b.secrets.is_empty());
    }
}
