//! Registry-style values shared by the `.reg` file parser, the live Windows registry readers
//! and INI-based sources (which only ever produce strings).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    Str(String),
    Dword(u32),
}

pub type Fields = HashMap<String, RegValue>;

pub fn get_str(f: &Fields, k: &str) -> Option<String> {
    match f.get(k)? {
        RegValue::Str(s) => Some(s.clone()),
        RegValue::Dword(n) => Some(n.to_string()),
    }
}

pub fn get_u32(f: &Fields, k: &str) -> Option<u32> {
    match f.get(k)? {
        RegValue::Dword(n) => Some(*n),
        RegValue::Str(s) => s.trim().parse().ok(),
    }
}

pub struct RegKey {
    pub path: String,
    pub values: Fields,
}

/// Parse a `.reg` export (REGEDIT4 or "Windows Registry Editor Version 5.00").
/// Only `REG_SZ` and `REG_DWORD` values are kept.
pub fn parse(text: &str) -> Vec<RegKey> {
    // Join `hex:..,\` continuation lines.
    let mut lines: Vec<String> = Vec::new();
    let mut pending = String::new();
    for raw in text.lines() {
        let l = raw.trim_end_matches('\r');
        let piece = if pending.is_empty() {
            l.trim()
        } else {
            l.trim_start()
        };
        if piece.ends_with('\\') && !piece.ends_with('"') {
            pending.push_str(&piece[..piece.len() - 1]);
        } else {
            pending.push_str(piece);
            lines.push(std::mem::take(&mut pending));
        }
    }
    if !pending.is_empty() {
        lines.push(pending);
    }

    let mut out: Vec<RegKey> = Vec::new();
    for l in lines {
        if let Some(rest) = l.strip_prefix('[') {
            let path = rest.trim_end_matches(']').to_string();
            if path.starts_with('-') {
                continue;
            }
            out.push(RegKey {
                path,
                values: Fields::new(),
            });
        } else if l.starts_with('"') {
            let Some(cur) = out.last_mut() else { continue };
            let Some((name, rest)) = take_quoted(&l) else {
                continue;
            };
            let Some(val) = rest.trim_start().strip_prefix('=') else {
                continue;
            };
            let val = val.trim();
            if val.starts_with('"') {
                if let Some((s, _)) = take_quoted(val) {
                    cur.values.insert(name, RegValue::Str(s));
                }
            } else if let Some(h) = val.strip_prefix("dword:") {
                if let Ok(n) = u32::from_str_radix(h.trim(), 16) {
                    cur.values.insert(name, RegValue::Dword(n));
                }
            }
        }
    }
    out
}

/// Read a leading `"..."` token with `\\` and `\"` escapes; returns (content, remainder).
fn take_quoted(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, n)) = chars.next() {
                    out.push(n);
                }
            }
            '"' => return Some((out, &s[i + 1..])),
            c => out.push(c),
        }
    }
    None
}

/// Read all direct subkeys of `HKCU\<path>` as `(raw key name, values)`.
#[cfg(windows)]
pub fn read_hkcu_subkeys(path: &str) -> Option<Vec<(String, Fields)>> {
    use winreg::enums::{HKEY_CURRENT_USER, REG_DWORD, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;
    let root = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(path)
        .ok()?;
    let mut out = Vec::new();
    for name in root.enum_keys().flatten() {
        let Ok(k) = root.open_subkey(&name) else {
            continue;
        };
        let mut f = Fields::new();
        for (n, v) in k.enum_values().flatten() {
            match v.vtype {
                REG_SZ | REG_EXPAND_SZ => {
                    if let Ok(s) = String::from_reg_value(&v) {
                        f.insert(n, RegValue::Str(s));
                    }
                }
                REG_DWORD => {
                    if let Ok(d) = u32::from_reg_value(&v) {
                        f.insert(n, RegValue::Dword(d));
                    }
                }
                _ => {}
            }
        }
        out.push((name, f));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strings_and_dwords() {
        let t = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\A\\B%20C]\r\n\"Host\"=\"h\\\\x\\\"q\"\r\n\"PortNumber\"=dword:00000016\r\n\"Bin\"=hex:01,02,\\\r\n  03\r\n@=\"d\"\r\n";
        let k = parse(t);
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].path, "HKEY_CURRENT_USER\\A\\B%20C");
        assert_eq!(get_str(&k[0].values, "Host").unwrap(), "h\\x\"q");
        assert_eq!(get_u32(&k[0].values, "PortNumber"), Some(22));
        assert!(!k[0].values.contains_key("Bin"));
    }
}
