//! OpenSSH client config (`~/.ssh/config`), per ssh_config(5).

use crate::builder::Builder;
use crate::text::{default_user, expand_tilde, parse_port, read_text, split_user_host};
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::SshAuth;
use aht_core::{ConnectionId, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_INCLUDE_DEPTH: usize = 8;

enum Directive {
    Host(Vec<String>),
    Match,
    Opt(String, Vec<String>),
}

struct Block {
    /// `None` = global section (applies to every host).
    patterns: Option<Vec<String>>,
    opts: Vec<(String, Vec<String>)>,
}

pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    let mut bases = Vec::new();
    if let Some(h) = dirs::home_dir() {
        bases.push(h.join(".ssh"));
    }
    if let Some(p) = path.parent() {
        bases.push(p.to_path_buf());
    }
    parse_with_bases(path, &bases, opts)
}

pub fn count_hosts(path: &Path) -> usize {
    let mut b = Builder::new(Tool::SshConfig, &ImportOptions::default());
    let bases: Vec<PathBuf> = path.parent().map(|p| p.to_path_buf()).into_iter().collect();
    let mut warnings = Vec::new();
    match read_directives(path, &bases, 0, &mut warnings) {
        Ok(d) => collect_blocks(d, &mut b)
            .iter()
            .flat_map(|bl| bl.patterns.iter().flatten())
            .filter(|p| is_concrete(p))
            .count(),
        Err(_) => 0,
    }
}

pub(crate) fn parse_with_bases(
    path: &Path,
    bases: &[PathBuf],
    opts: &ImportOptions,
) -> Result<ImportBundle> {
    let mut b = Builder::new(Tool::SshConfig, opts);
    let mut warnings = Vec::new();
    let directives = read_directives(path, bases, 0, &mut warnings)?;
    for w in warnings {
        b.warn(w);
    }
    let blocks = collect_blocks(directives, &mut b);

    // Concrete aliases in file order.
    let mut aliases: Vec<String> = Vec::new();
    for bl in &blocks {
        for p in bl.patterns.iter().flatten() {
            if is_concrete(p) && !aliases.contains(p) {
                aliases.push(p.clone());
            }
        }
    }

    let mut ids: HashMap<String, ConnectionId> = HashMap::new();
    let mut pending_jumps: Vec<(ConnectionId, String, String)> = Vec::new();
    for alias in &aliases {
        let eff = effective(&blocks, alias);
        let host = eff
            .first("hostname")
            .map(|h| h.replace("%h", alias))
            .unwrap_or_else(|| alias.clone());
        let port = eff.first("port").and_then(parse_port).unwrap_or(22);
        let user = eff
            .first("user")
            .map(str::to_string)
            .unwrap_or_else(default_user);
        let mut notes = String::new();
        let auth = match eff.all("identityfile").first() {
            Some(f) => b.key_auth(alias, f, &mut notes),
            None => SshAuth::Agent,
        };
        if eff.all("identityfile").len() > 1 {
            b.warn(format!("{alias}: only the first IdentityFile was imported"));
        }
        if eff.first("proxycommand").is_some_and(|v| v != "none") {
            b.warn(format!(
                "{alias}: ProxyCommand is not supported and was ignored"
            ));
        }
        let id = b.add(alias, None, &host, port, &user, auth, notes);
        ids.insert(alias.to_lowercase(), id);
        if let Some(j) = eff.first("proxyjump") {
            if !j.eq_ignore_ascii_case("none") {
                pending_jumps.push((id, alias.clone(), j.to_string()));
            }
        }
    }

    for (id, alias, spec) in pending_jumps {
        let mut chain = Vec::new();
        for hop in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (_, rest) = split_user_host(hop);
            let host_part = rest.rsplit_once(':').map_or(rest, |(h, _)| h);
            let found = ids
                .get(&hop.to_lowercase())
                .or_else(|| ids.get(&rest.to_lowercase()))
                .or_else(|| ids.get(&host_part.to_lowercase()));
            match found {
                Some(j) if *j != id => chain.push(*j),
                _ => b.warn(format!(
                    "{alias}: ProxyJump host '{hop}' is not among the imported hosts"
                )),
            }
        }
        if let Some(c) = b.connections.iter_mut().find(|c| c.id == id) {
            c.jump = chain;
        }
    }
    Ok(b.finish())
}

fn is_concrete(p: &str) -> bool {
    !p.is_empty() && !p.contains(['*', '?']) && !p.starts_with('!')
}

fn collect_blocks(directives: Vec<Directive>, b: &mut Builder) -> Vec<Block> {
    let mut blocks = vec![Block {
        patterns: None,
        opts: Vec::new(),
    }];
    let mut in_match = false;
    for d in directives {
        match d {
            Directive::Host(p) => {
                in_match = false;
                blocks.push(Block {
                    patterns: Some(p),
                    opts: Vec::new(),
                });
            }
            Directive::Match => {
                if !in_match {
                    b.warn("Match blocks are not supported and were ignored");
                }
                in_match = true;
                // Options in a Match block must not leak into the previous Host block.
                blocks.push(Block {
                    patterns: Some(vec!["!*".into()]),
                    opts: Vec::new(),
                });
            }
            Directive::Opt(k, v) => blocks.last_mut().unwrap().opts.push((k, v)),
        }
    }
    blocks
}

struct Effective(Vec<(String, Vec<String>)>);

impl Effective {
    fn first(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.first().map(String::as_str))
    }
    fn all(&self, key: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(k, _)| k == key)
            .filter_map(|(_, v)| v.first().map(String::as_str))
            .collect()
    }
}

/// Options that apply to `alias`: ssh uses the first value obtained for each keyword.
fn effective(blocks: &[Block], alias: &str) -> Effective {
    let mut out = Vec::new();
    for bl in blocks {
        let applies = match &bl.patterns {
            None => true,
            Some(pats) => {
                let mut pos = false;
                let mut neg = false;
                for p in pats {
                    if let Some(n) = p.strip_prefix('!') {
                        neg |= wildcard(n, alias);
                    } else {
                        pos |= wildcard(p, alias);
                    }
                }
                pos && !neg
            }
        };
        if applies {
            out.extend(bl.opts.iter().cloned());
        }
    }
    Effective(out)
}

/// Case-insensitive `*` / `?` matcher.
fn wildcard(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && go(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(&p, &t)
}

fn read_directives(
    path: &Path,
    bases: &[PathBuf],
    depth: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<Directive>> {
    let text = read_text(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((key, args)) = split_line(line) else {
            continue;
        };
        match key.as_str() {
            "host" => out.push(Directive::Host(args)),
            "match" => out.push(Directive::Match),
            "include" => {
                if depth >= MAX_INCLUDE_DEPTH {
                    warnings.push("Include nesting too deep; ignored".into());
                    continue;
                }
                for pat in args {
                    for f in resolve_include(&pat, bases) {
                        match read_directives(&f, bases, depth + 1, warnings) {
                            Ok(d) => out.extend(d),
                            Err(e) => warnings.push(format!("Include {}: {e}", f.display())),
                        }
                    }
                }
            }
            _ => out.push(Directive::Opt(key, args)),
        }
    }
    Ok(out)
}

fn resolve_include(pat: &str, bases: &[PathBuf]) -> Vec<PathBuf> {
    let expanded = expand_tilde(pat);
    let candidates: Vec<PathBuf> = if expanded.is_absolute() {
        vec![expanded]
    } else {
        bases.iter().map(|b| b.join(&expanded)).collect()
    };
    for c in candidates {
        let Some(s) = c.to_str() else { continue };
        let mut found: Vec<PathBuf> = glob::glob(s)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|p| p.is_file())
            .collect();
        if !found.is_empty() {
            found.sort();
            return found;
        }
    }
    Vec::new()
}

/// Split a config line into (lowercased keyword, arguments). Handles `Key=Value`, quotes
/// and trailing comments.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let kw_end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let key = line[..kw_end].to_lowercase();
    let rest = line[kw_end..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in rest.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '#' if !quoted && !has && cur.is_empty() => break,
            c if c.is_whitespace() && !quoted => {
                if has || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        args.push(cur);
    }
    Some((key, args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aht_core::model::SecretKind;

    fn parse() -> ImportBundle {
        let tmp = crate::stage_dir("ssh");
        let dir = tmp.path().to_path_buf();
        parse_with_bases(&dir.join("config"), &[dir], &ImportOptions::default()).unwrap()
    }

    fn conn<'a>(b: &'a ImportBundle, name: &str) -> &'a aht_core::model::Connection {
        b.connections.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn hosts_fields_defaults_and_wildcards() {
        let b = parse();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["web", "db", "bastion", "inc1", "inc2"]);
        let web = conn(&b, "web");
        assert_eq!(
            (web.host.as_str(), web.port, web.user.as_str()),
            ("web.example.test", 2222, "deploy")
        );
        // `Host *` supplies the user for hosts that do not set one.
        assert_eq!(conn(&b, "db").user, "fallbackuser");
        assert_eq!(conn(&b, "db").host, "db"); // no HostName => alias
        assert_eq!(conn(&b, "bastion").port, 22);
    }

    #[test]
    fn identity_file_imported_and_agent_default() {
        let b = parse();
        let web = conn(&b, "web");
        let SshAuth::Key { key, .. } = &web.auth else {
            panic!("expected key auth, got {:?}", web.auth)
        };
        let s = b.secrets.iter().find(|s| s.id == *key).unwrap();
        assert_eq!(s.kind, SecretKind::PrivateKey);
        assert!(s.value.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert_eq!(conn(&b, "bastion").auth, SshAuth::Agent);
    }

    #[test]
    fn proxyjump_maps_aliases_and_warns_otherwise() {
        let b = parse();
        let bastion = conn(&b, "bastion").id;
        assert_eq!(conn(&b, "db").jump, vec![bastion]);
        assert!(conn(&b, "inc1").jump.is_empty());
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("inc1") && w.contains("unknown-jump")));
    }

    #[test]
    fn include_globs_and_match_warning() {
        let b = parse();
        assert_eq!(conn(&b, "inc1").host, "10.9.9.1");
        assert_eq!(conn(&b, "inc2").port, 2200);
        assert!(b.warnings.iter().any(|w| w.contains("Match")));
        assert!(b.warnings.iter().any(|w| w.contains("ProxyCommand")));
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard("web*", "WEB1"));
        assert!(wildcard("a?c", "abc"));
        assert!(!wildcard("a?c", "ac"));
        assert!(wildcard("*", ""));
    }

    #[test]
    fn line_splitting() {
        assert_eq!(
            split_line("  Host = a  \"b c\" # x").unwrap(),
            ("host".into(), vec!["a".into(), "b c".into()])
        );
        assert_eq!(
            split_line("Port=22").unwrap(),
            ("port".into(), vec!["22".into()])
        );
        assert!(split_line("# c").is_none());
    }
}
