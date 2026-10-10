//! Read-only listing of the skills/plugins each harness has installed for the current user.
//!
//! Observed layouts (2026-10):
//! * Claude Code: `~/.claude/skills/<name>/SKILL.md` (YAML frontmatter `name`, `description`;
//!   entries may be symlinks/junctions). Plugins: `~/.claude/plugins/installed_plugins.json`,
//!   `{"version":2,"plugins":{"<plugin>@<marketplace>":[{"scope","installPath","version",...}]}}`;
//!   the plugin manifest `<installPath>/.claude-plugin/plugin.json` may carry a `description`.
//! * Codex: skills in `~/.codex/skills/<name>/SKILL.md`; custom prompts `~/.codex/prompts/*.md`
//!   are reported as "plugins" (no stable plugin registry file is read).
//! * OpenCode: `~/.config/opencode/skills/<name>/SKILL.md`; agents and commands
//!   (`agent(s)/*.md`, `command(s)/*.md`) are reported as "plugins".

use aht_core::model::HarnessKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ext {
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extensions {
    pub skills: Vec<Ext>,
    pub plugins: Vec<Ext>,
}

/// List extensions for the current user (`$HOME` / `%USERPROFILE%`). Missing dirs yield empty lists.
pub fn list_extensions(kind: HarnessKind) -> Extensions {
    match crate::detect::home_dir() {
        Some(home) => list_extensions_in(kind, &home),
        None => Extensions::default(),
    }
}

/// Same as [`list_extensions`] with an explicit home directory (used by tests).
pub fn list_extensions_in(kind: HarnessKind, home: &Path) -> Extensions {
    let mut ext = match kind {
        HarnessKind::ClaudeCode => Extensions {
            skills: scan_skills(&home.join(".claude/skills")),
            plugins: claude_plugins(&home.join(".claude/plugins/installed_plugins.json")),
        },
        HarnessKind::Codex => Extensions {
            skills: scan_skills(&home.join(".codex/skills")),
            plugins: scan_markdown(&home.join(".codex/prompts")),
        },
        HarnessKind::OpenCode => {
            let cfg = home.join(".config/opencode");
            let mut plugins = Vec::new();
            for d in ["agent", "agents", "command", "commands"] {
                plugins.extend(scan_markdown(&cfg.join(d)));
            }
            Extensions {
                skills: scan_skills(&cfg.join("skills")),
                plugins,
            }
        }
    };
    ext.skills.sort_by(|a, b| a.name.cmp(&b.name));
    ext.plugins.sort_by(|a, b| a.name.cmp(&b.name));
    ext
}

/// Map profile plugin identifiers to directories usable with `claude --plugin-dir`.
/// An identifier may be an existing directory path, a full `name@marketplace` key, or a bare name.
pub fn resolve_plugin_dirs(kind: HarnessKind, ids: &[String]) -> Vec<PathBuf> {
    let installed = list_extensions(kind).plugins;
    ids.iter()
        .filter_map(|id| {
            let p = Path::new(id);
            if p.is_dir() {
                return Some(p.to_path_buf());
            }
            installed
                .iter()
                .find(|e| e.name == *id || e.name.split('@').next() == Some(id.as_str()))
                .map(|e| e.path.clone())
        })
        .collect()
}

/// Parse `name` / `description` from a SKILL.md YAML frontmatter block. Handles plain, quoted and
/// folded/literal (`>`/`|`) scalars; everything else is ignored.
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let body: Vec<&str> = lines.take_while(|l| l.trim() != "---").collect();
    let get = |key: &str| -> Option<String> {
        let idx = body.iter().position(|l| {
            l.strip_prefix(key)
                .is_some_and(|r| r.trim_start().starts_with(':'))
        })?;
        let rest = body[idx].split_once(':')?.1.trim();
        let value = if matches!(rest, ">" | "|" | ">-" | "|-" | ">+" | "|+") || rest.is_empty() {
            let cont: Vec<&str> = body[idx + 1..]
                .iter()
                .take_while(|l| l.starts_with([' ', '\t']) || l.trim().is_empty())
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            cont.join(" ")
        } else {
            unquote(rest)
        };
        (!value.is_empty()).then_some(value)
    };
    (get("name"), get("description"))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

fn scan_skills(dir: &Path) -> Vec<Ext> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    rd.flatten()
        .filter_map(|e| {
            let path = e.path();
            let md = path.join("SKILL.md");
            if !md.is_file() {
                return None;
            }
            let dir_name = e.file_name().to_string_lossy().into_owned();
            let (name, description) = std::fs::read_to_string(&md)
                .map(|t| parse_frontmatter(&t))
                .unwrap_or((None, None));
            Some(Ext {
                name: name.unwrap_or(dir_name),
                path,
                description,
            })
        })
        .collect()
}

fn scan_markdown(dir: &Path) -> Vec<Ext> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md") && e.path().is_file())
        .map(|e| {
            let path = e.path();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let description = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| parse_frontmatter(&t).1);
            Ext {
                name,
                path,
                description,
            }
        })
        .collect()
}

fn claude_plugins(file: &Path) -> Vec<Ext> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return vec![];
    };
    let Some(map) = v.get("plugins").and_then(|p| p.as_object()) else {
        return vec![];
    };
    map.iter()
        .filter_map(|(name, installs)| {
            // Several installs (per scope) may exist; take the first with an installPath.
            let path = installs
                .as_array()?
                .iter()
                .find_map(|i| i.get("installPath")?.as_str())
                .map(PathBuf::from)?;
            let description = std::fs::read_to_string(path.join(".claude-plugin/plugin.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|m| m.get("description")?.as_str().map(str::to_string));
            Some(Ext {
                name: name.clone(),
                path,
                description,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn frontmatter_variants() {
        let t = "---\nname: my-skill\ndescription: \"Does things: well\"\nother: 1\n---\nbody";
        assert_eq!(
            parse_frontmatter(t),
            (Some("my-skill".into()), Some("Does things: well".into()))
        );
        let folded = "---\nname: x\ndescription: >\n  first line\n  second line\nnext: y\n---\n";
        assert_eq!(
            parse_frontmatter(folded).1.as_deref(),
            Some("first line second line")
        );
        assert_eq!(parse_frontmatter("no frontmatter"), (None, None));
        // `names:` must not match `name`.
        assert_eq!(parse_frontmatter("---\nnames: a\n---\n").0, None);
    }

    #[test]
    fn claude_listing() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::create_dir_all(h.join(".claude/skills/alpha")).unwrap();
        fs::write(
            h.join(".claude/skills/alpha/SKILL.md"),
            "---\nname: alpha\ndescription: Alpha skill\n---\n# A",
        )
        .unwrap();
        fs::create_dir_all(h.join(".claude/skills/no-md")).unwrap();
        fs::create_dir_all(h.join(".claude/skills/beta")).unwrap();
        fs::write(h.join(".claude/skills/beta/SKILL.md"), "# no frontmatter").unwrap();
        let plugin = h.join("p/cache/x");
        fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        fs::write(
            plugin.join(".claude-plugin/plugin.json"),
            r#"{"name":"x","description":"Plugin X"}"#,
        )
        .unwrap();
        fs::create_dir_all(h.join(".claude/plugins")).unwrap();
        let json = serde_json::json!({"version":2,"plugins":{
            "x@mkt":[{"scope":"user","installPath":plugin.to_string_lossy(),"version":"1"}],
            "broken@mkt":[]
        }});
        fs::write(
            h.join(".claude/plugins/installed_plugins.json"),
            json.to_string(),
        )
        .unwrap();

        let e = list_extensions_in(HarnessKind::ClaudeCode, h);
        let names: Vec<_> = e.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta"]);
        assert_eq!(e.skills[0].description.as_deref(), Some("Alpha skill"));
        assert_eq!(e.plugins.len(), 1);
        assert_eq!(e.plugins[0].name, "x@mkt");
        assert_eq!(e.plugins[0].description.as_deref(), Some("Plugin X"));
    }

    #[test]
    fn missing_dirs_are_empty() {
        let home = tempfile::tempdir().unwrap();
        for k in [
            HarnessKind::ClaudeCode,
            HarnessKind::Codex,
            HarnessKind::OpenCode,
        ] {
            assert_eq!(list_extensions_in(k, home.path()), Extensions::default());
        }
    }

    #[test]
    fn codex_and_opencode() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::create_dir_all(h.join(".codex/skills/s1")).unwrap();
        fs::write(h.join(".codex/skills/s1/SKILL.md"), "---\nname: s1\n---\n").unwrap();
        fs::create_dir_all(h.join(".codex/prompts")).unwrap();
        fs::write(
            h.join(".codex/prompts/review.md"),
            "---\ndescription: Review\n---\n",
        )
        .unwrap();
        let c = list_extensions_in(HarnessKind::Codex, h);
        assert_eq!(c.skills[0].name, "s1");
        assert_eq!(c.plugins[0].name, "review");
        assert_eq!(c.plugins[0].description.as_deref(), Some("Review"));

        fs::create_dir_all(h.join(".config/opencode/agents")).unwrap();
        fs::write(h.join(".config/opencode/agents/planner.md"), "x").unwrap();
        let o = list_extensions_in(HarnessKind::OpenCode, h);
        assert_eq!(o.plugins[0].name, "planner");
    }
}
