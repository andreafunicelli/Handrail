//! OpenSSH `known_hosts` store.
//!
//! The app owns one read-write file; any number of additional files (typically
//! `~/.ssh/known_hosts`) are consulted read-only. If the app file has *any* entry for a host, only
//! the app file is consulted for that host, so that a key the user explicitly replaced in the app
//! is not shadowed by a stale entry in a read-only file.

use aht_core::{Error, Result};
use data_encoding::BASE64;
use hmac::{Hmac, KeyInit, Mac};
use russh::keys::{Algorithm, HashAlg, PublicKey};
use sha1::Sha1;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Result of looking a host key up in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// Key matches a recorded entry.
    Known,
    /// No entry of this key's algorithm exists for the host (trust on first use).
    New { fingerprint: String },
    /// An entry of the same algorithm exists with a different key. Possible MITM.
    Changed { old: String, new: String },
    /// The key is marked `@revoked` in a known_hosts file: never accept it.
    Revoked { fingerprint: String },
}

/// SHA256 fingerprint in OpenSSH format, e.g. `SHA256:abc...`.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// The host token as written in known_hosts: `host` for port 22, `[host]:port` otherwise.
pub fn host_token(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

#[derive(Clone, Debug)]
struct Entry {
    patterns: String,
    key: PublicKey,
    /// An `@revoked` line: this key must be refused.
    revoked: bool,
}

fn parse_file(path: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Entry> {
    let line = line.trim();
    let (line, revoked) = match line.strip_prefix("@revoked") {
        Some(rest) if rest.starts_with(char::is_whitespace) => (rest.trim_start(), true),
        _ => (line, false),
    };
    if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
        // `@cert-authority` lines are not supported; skip them.
        return None;
    }
    let mut parts = line.split_whitespace();
    let patterns = parts.next()?.to_string();
    let _alg = parts.next()?;
    let b64 = parts.next()?;
    let key = russh::keys::parse_public_key_base64(b64).ok()?;
    Some(Entry {
        patterns,
        key,
        revoked,
    })
}

/// Does `token` (e.g. `[h]:2222`) match a comma-separated known_hosts pattern list?
fn matches_patterns(token: &str, patterns: &str) -> bool {
    let mut matched = false;
    for pat in patterns.split(',') {
        if let Some(hashed) = pat.strip_prefix("|1|") {
            if hashed_matches(token, hashed) {
                matched = true;
            }
        } else if let Some(neg) = pat.strip_prefix('!') {
            if glob(neg, token) {
                return false;
            }
        } else if glob(pat, token) {
            matched = true;
        }
    }
    matched
}

fn hashed_matches(token: &str, hashed: &str) -> bool {
    let mut it = hashed.split('|');
    let (Some(salt), Some(hash)) = (it.next(), it.next()) else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (
        BASE64.decode(salt.as_bytes()),
        BASE64.decode(hash.as_bytes()),
    ) else {
        return false;
    };
    let Ok(mac) = <Hmac<Sha1> as KeyInit>::new_from_slice(&salt) else {
        return false;
    };
    // OpenSSH hashes the lowercased host name, as the plain-text patterns match case-blind.
    mac.chain_update(token.to_lowercase().as_bytes())
        .verify_slice(&hash)
        .is_ok()
}

/// Case-insensitive glob with `*` and `?`.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Known-hosts store: one app-managed writable file plus read-only extras.
#[derive(Clone, Debug)]
pub struct KnownHosts {
    app_file: PathBuf,
    read_only: Vec<PathBuf>,
    lock: std::sync::Arc<std::sync::Mutex<()>>,
}

impl KnownHosts {
    /// Store backed by `app_file` only.
    pub fn new(app_file: impl Into<PathBuf>) -> Self {
        Self {
            app_file: app_file.into(),
            read_only: Vec::new(),
            lock: Default::default(),
        }
    }

    /// Store backed by `app_file`, also reading `~/.ssh/known_hosts` (if a home dir is known).
    pub fn with_user_known_hosts(app_file: impl Into<PathBuf>) -> Self {
        let mut s = Self::new(app_file);
        if let Some(p) = user_known_hosts_path() {
            s.read_only.push(p);
        }
        s
    }

    /// Add another read-only file to consult.
    pub fn add_read_only(mut self, path: impl Into<PathBuf>) -> Self {
        self.read_only.push(path.into());
        self
    }

    fn entries_for(&self, token: &str) -> Vec<Entry> {
        let app: Vec<Entry> = parse_file(&self.app_file)
            .into_iter()
            .filter(|e| !e.revoked && matches_patterns(token, &e.patterns))
            .collect();
        if !app.is_empty() {
            return app;
        }
        self.read_only
            .iter()
            .flat_map(|p| parse_file(p))
            .filter(|e| !e.revoked && matches_patterns(token, &e.patterns))
            .collect()
    }

    /// Whether any file (the app's or a read-only one) revokes `key` for `token`.
    fn is_revoked(&self, token: &str, key: &PublicKey) -> bool {
        std::iter::once(&self.app_file)
            .chain(&self.read_only)
            .flat_map(|p| parse_file(p))
            .any(|e| {
                e.revoked
                    && e.key.key_data() == key.key_data()
                    && matches_patterns(token, &e.patterns)
            })
    }

    /// Algorithms of the keys recorded for `host:port`, to ask the server for one of those.
    pub fn algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.entries_for(&host_token(host, port))
            .iter()
            .map(|e| e.key.algorithm())
            .collect()
    }

    /// Look up `key` for `host:port`.
    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let token = host_token(host, port);
        if self.is_revoked(&token, key) {
            return HostKeyStatus::Revoked {
                fingerprint: fingerprint(key),
            };
        }
        let entries = self.entries_for(&token);
        if entries.iter().any(|e| e.key.key_data() == key.key_data()) {
            return HostKeyStatus::Known;
        }
        // Any stored key counts, not only one of the same type: otherwise a man in the middle
        // could offer a different key type and get the mild "new host" prompt.
        if let Some(old) = entries
            .iter()
            .find(|e| e.key.algorithm() == key.algorithm())
            .or(entries.first())
        {
            return HostKeyStatus::Changed {
                old: fingerprint(&old.key),
                new: fingerprint(key),
            };
        }
        HostKeyStatus::New {
            fingerprint: fingerprint(key),
        }
    }

    /// Append a trusted key for `host:port` to the app file.
    pub fn add(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.append(host, port, key)
    }

    /// Remove every app-file entry for `host:port` (of any key type: `check` reports a key of
    /// another type as a change, so an old one must not stay trusted), then add `key`. The file
    /// is replaced in one step, so a crash leaves either the old or the new contents.
    pub fn replace(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let token = host_token(host, port);
        let text = match std::fs::read_to_string(&self.app_file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let mut out = String::new();
        for l in text.lines() {
            let drop =
                parse_line(l).is_some_and(|e| !e.revoked && matches_patterns(&token, &e.patterns));
            if !drop {
                out.push_str(l);
                out.push('\n');
            }
        }
        out.push_str(&entry_line(host, port, key)?);
        replace_file(&self.app_file, out.as_bytes())?;
        Ok(())
    }

    fn append(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        if let Some(parent) = self.app_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = entry_line(host, port, key)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.app_file)?;
        f.write_all(line.as_bytes())?;
        Ok(())
    }
}

/// `token alg base64\n` for `key`.
fn entry_line(host: &str, port: u16, key: &PublicKey) -> Result<String> {
    let b64 = key
        .to_openssh()
        .map_err(|e| Error::Invalid(format!("encode host key: {e}")))?;
    // `to_openssh` yields "alg base64 [comment]"; drop the comment.
    let mut parts = b64.split_whitespace();
    let (alg, data) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    Ok(format!("{} {alg} {data}\n", host_token(host, port)))
}

/// Write `bytes` to a temp file beside `path` (keeping `path`'s permissions), then rename it over.
fn replace_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        if let Ok(m) = std::fs::metadata(path) {
            f.set_permissions(m.permissions())?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `~/.ssh/known_hosts`, if a home directory can be determined.
pub fn user_known_hosts_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};

    fn key() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }

    fn line(token: &str, k: &PublicKey) -> String {
        format!("{token} {}\n", k.to_openssh().unwrap())
    }

    #[test]
    fn glob_matching() {
        assert!(glob("*.example.com", "a.example.com"));
        assert!(glob("h?st", "HOST"));
        assert!(!glob("*.example.com", "example.org"));
        assert!(matches_patterns("a.b", "x,a.b"));
        assert!(!matches_patterns("a.b", "*.b,!a.b"));
    }

    #[test]
    fn new_known_changed() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("kh"));
        let (k1, k2) = (key(), key());
        assert!(matches!(
            kh.check("h", 2222, &k1),
            HostKeyStatus::New { .. }
        ));
        kh.add("h", 2222, &k1).unwrap();
        assert_eq!(kh.check("h", 2222, &k1), HostKeyStatus::Known);
        assert!(matches!(kh.check("h", 22, &k1), HostKeyStatus::New { .. }));
        match kh.check("h", 2222, &k2) {
            HostKeyStatus::Changed { old, new } => {
                assert_eq!(old, fingerprint(&k1));
                assert_eq!(new, fingerprint(&k2));
            }
            other => panic!("{other:?}"),
        }
        kh.replace("h", 2222, &k2).unwrap();
        assert_eq!(kh.check("h", 2222, &k2), HostKeyStatus::Known);
        assert_eq!(kh.algorithms("h", 2222).len(), 1, "the old key is gone");
        assert!(matches!(
            kh.check("h", 2222, &k1),
            HostKeyStatus::Changed { .. }
        ));
        // A key of another type for a known host is a change, not a new host.
        let ecdsa = PrivateKey::random(
            &mut rand::rng(),
            Algorithm::Ecdsa {
                curve: russh::keys::EcdsaCurve::NistP256,
            },
        )
        .unwrap()
        .public_key()
        .clone();
        assert!(matches!(
            kh.check("h", 2222, &ecdsa),
            HostKeyStatus::Changed { .. }
        ));
        // Replacing it with that other type leaves no key of the old type trusted.
        kh.add("other", 22, &k1).unwrap();
        kh.replace("h", 2222, &ecdsa).unwrap();
        assert_eq!(kh.check("h", 2222, &ecdsa), HostKeyStatus::Known);
        assert!(matches!(
            kh.check("h", 2222, &k2),
            HostKeyStatus::Changed { .. }
        ));
        assert_eq!(kh.check("other", 22, &k1), HostKeyStatus::Known);
        let n = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(n, 1, "no temp file left behind");
    }

    #[cfg(unix)]
    #[test]
    fn replace_keeps_the_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("kh");
        let kh = KnownHosts::new(&p);
        kh.add("h", 22, &key()).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        kh.replace("h", 22, &key()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn reads_hashed_and_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let k = key();
        // Build a hashed entry like `ssh-keygen -H` does.
        let salt = [7u8; 20];
        let mac = <Hmac<Sha1> as KeyInit>::new_from_slice(&salt)
            .unwrap()
            .chain_update(b"[srv]:2200")
            .finalize()
            .into_bytes();
        let token = format!("|1|{}|{}", BASE64.encode(&salt), BASE64.encode(&mac));
        let ro = dir.path().join("user_kh");
        std::fs::write(&ro, format!("# comment\n{}", line(&token, &k))).unwrap();
        let kh = KnownHosts::new(dir.path().join("app")).add_read_only(&ro);
        assert_eq!(kh.check("srv", 2200, &k), HostKeyStatus::Known);
        assert_eq!(
            kh.check("SRV", 2200, &k),
            HostKeyStatus::Known,
            "host case is ignored"
        );
        assert!(matches!(
            kh.check("other", 2200, &k),
            HostKeyStatus::New { .. }
        ));
        // App file entry takes precedence over the read-only file.
        let k2 = key();
        kh.replace("srv", 2200, &k2).unwrap();
        assert_eq!(kh.check("srv", 2200, &k2), HostKeyStatus::Known);
        // Read-only file untouched.
        assert!(std::fs::read_to_string(&ro).unwrap().contains(&token));
    }

    #[test]
    fn revoked_keys_are_refused_whatever_else_trusts_them() {
        let dir = tempfile::tempdir().unwrap();
        let (bad, good) = (key(), key());
        let ro = dir.path().join("user_kh");
        std::fs::write(&ro, format!("@revoked {}", line("*", &bad))).unwrap();
        let kh = KnownHosts::new(dir.path().join("app")).add_read_only(&ro);
        kh.add("srv", 22, &bad).unwrap();
        assert!(matches!(
            kh.check("srv", 22, &bad),
            HostKeyStatus::Revoked { .. }
        ));
        assert!(matches!(
            kh.check("srv", 22, &good),
            HostKeyStatus::Changed { .. }
        ));
        assert!(matches!(
            kh.check("other", 22, &good),
            HostKeyStatus::New { .. }
        ));
    }
}
