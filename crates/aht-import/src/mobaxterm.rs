//! MobaXterm `MobaXterm.ini` / `.mxtsessions` bookmark sections.
//!
//! MobaXterm is closed source; the layout (`[Bookmarks*]` sections with `SubRep`/`ImgNum`,
//! session lines `name=#icon#type%host%port%user%...`, type 0 = SSH) follows third-party
//! reverse-engineering (e.g. 0xtosh/mobaxtermsessionconvert) and is parsed defensively.

use crate::builder::Builder;
use crate::ini;
use crate::text::{parse_port, read_text};
use crate::{ImportBundle, ImportOptions, Tool};
use aht_core::model::SshAuth;
use aht_core::Result;
use std::path::Path;

pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    Ok(parse_text(&read_text(path)?, opts))
}

pub(crate) fn parse_text(text: &str, opts: &ImportOptions) -> ImportBundle {
    let mut b = Builder::new(Tool::MobaXterm, opts);
    let sections = ini::parse(text);

    if sections.iter().any(|s| {
        (s.name.eq_ignore_ascii_case("Passwords") || s.name.eq_ignore_ascii_case("Credentials"))
            && !s.entries.is_empty()
    }) {
        b.warn("MobaXterm saved passwords are encrypted with your master password and were not imported");
    }

    for s in sections.iter().filter(|s| {
        s.name.eq_ignore_ascii_case("Bookmarks")
            || s.name
                .get(..10)
                .is_some_and(|p| p.eq_ignore_ascii_case("Bookmarks_"))
    }) {
        let path: Vec<String> = s
            .get("SubRep")
            .unwrap_or("")
            .split('\\')
            .map(str::to_string)
            .collect();
        let group = b.group_path(&path);
        for (name, value) in &s.entries {
            if name.eq_ignore_ascii_case("SubRep") || name.eq_ignore_ascii_case("ImgNum") {
                continue;
            }
            let Some(rest) = value.strip_prefix('#') else {
                continue;
            };
            let Some((_icon, session)) = rest.split_once('#') else {
                continue;
            };
            let f: Vec<&str> = session.split('%').collect();
            if f.first().map(|t| t.trim()) != Some("0") {
                b.skip("non-SSH MobaXterm sessions");
                continue;
            }
            let host = f.get(1).map_or("", |h| h.trim());
            if host.is_empty() {
                b.skip("MobaXterm sessions without a host");
                continue;
            }
            let port = f.get(2).and_then(|p| parse_port(p)).unwrap_or(22);
            let user = f.get(3).map_or("", |u| u.trim());
            let mut notes = String::new();
            let key = f.iter().skip(4).find(|x| looks_like_key(x));
            let auth = match key {
                Some(k) => b.key_auth(name, k.trim(), &mut notes),
                None => SshAuth::Prompt,
            };
            b.add(name, group, host, port, user, auth, notes);
        }
    }
    b.finish()
}

fn looks_like_key(s: &str) -> bool {
    let s = s.trim();
    let lower = s.to_lowercase();
    let file = lower.rsplit(['\\', '/']).next().unwrap_or("");
    !s.is_empty()
        && (lower.ends_with(".ppk")
            || lower.ends_with(".pem")
            || lower.ends_with(".key")
            || file.starts_with("id_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_dir;

    fn parse() -> (ImportBundle, tempfile::TempDir) {
        let tmp = stage_dir("mobaxterm");
        let b = parse_file(&tmp.path().join("MobaXterm.ini"), &ImportOptions::default()).unwrap();
        (b, tmp)
    }

    #[test]
    fn sessions_and_folder_hierarchy() {
        let (b, _t) = parse();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["root srv", "db1", "keyed"]);
        let gname = |c: &aht_core::model::Connection| {
            c.group_id
                .map(|g| b.groups.iter().find(|x| x.id == g).unwrap().name.clone())
        };
        assert_eq!(gname(&b.connections[0]), None);
        assert_eq!(gname(&b.connections[1]), Some("DB".into()));
        // Servers\Prod\DB => 3 nested groups, each with the right parent.
        let db = b.groups.iter().find(|g| g.name == "DB").unwrap();
        let prod = b
            .groups
            .iter()
            .find(|g| g.id == db.parent_id.unwrap())
            .unwrap();
        assert_eq!(prod.name, "Prod");
        let servers = b
            .groups
            .iter()
            .find(|g| g.id == prod.parent_id.unwrap())
            .unwrap();
        assert_eq!(
            (servers.name.as_str(), servers.parent_id),
            ("Servers", None)
        );
    }

    #[test]
    fn fields_ports_users_and_skips() {
        let (b, _t) = parse();
        let c = &b.connections[0];
        assert_eq!(
            (c.host.as_str(), c.port, c.user.as_str()),
            ("10.0.0.5", 22, "root")
        );
        let d = &b.connections[1];
        assert_eq!(
            (d.host.as_str(), d.port, d.user.as_str()),
            ("db.example.test", 2200, "")
        );
        assert_eq!(d.auth, SshAuth::Prompt);
        assert!(b.warnings.iter().any(|w| w.contains("Skipped 2 non-SSH")));
        assert!(b.warnings.iter().any(|w| w.contains("master password")));
    }

    #[test]
    fn key_path_detected_and_ppk_warns() {
        let (b, _t) = parse();
        assert!(matches!(b.connections[2].auth, SshAuth::Key { .. }));
        let b2 = parse_text(
            "[Bookmarks_1]\nSubRep=\nImgNum=42\nx=#109#0%h%22%u%%-1%-1%%%%%0%0%0%Y:\\keys\\dev.ppk%%-1#MobaFont%10\n",
            &ImportOptions::default(),
        );
        assert_eq!(b2.connections.len(), 1);
        assert!(b2.warnings.iter().any(|w| w.contains("puttygen")));
    }

    #[test]
    fn mxtsessions_export_without_ini_extras() {
        let b = parse_text(
            "[Bookmarks]\nSubRep=Imported\nImgNum=42\nfoo=#109#0%foo.example.test%22%me%%-1\n",
            &ImportOptions::default(),
        );
        assert_eq!(b.connections[0].host, "foo.example.test");
        assert_eq!(b.groups[0].name, "Imported");
    }
}
