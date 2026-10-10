//! Small shared text helpers.

use aht_core::{Error, Result};
use std::path::{Path, PathBuf};

/// Read a text file, honouring UTF-8/UTF-16 BOMs; invalid UTF-8 falls back to Latin-1.
pub fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;
    Ok(decode_bytes(&bytes))
}

pub fn decode_bytes(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn utf16(b: &[u8], f: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = b
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| f([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

pub fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}

/// Parse a TCP port; `0`/garbage yields `None`.
pub fn parse_port(s: &str) -> Option<u16> {
    s.trim().parse::<u16>().ok().filter(|p| *p != 0)
}

/// Split `user@host` (user may be absent).
pub fn split_user_host(s: &str) -> (Option<&str>, &str) {
    match s.rsplit_once('@') {
        Some((u, h)) if !u.is_empty() => (Some(u), h),
        _ => (None, s),
    }
}

/// Fallback username when a source does not specify one.
pub fn default_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_boms() {
        assert_eq!(decode_bytes(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(decode_bytes(&[0xFF, 0xFE, b'a', 0, b'b', 0]), "ab");
        assert_eq!(decode_bytes(&[b'a', 0xE9]), "a\u{e9}");
    }

    #[test]
    fn splits_user_host() {
        assert_eq!(split_user_host("me@h"), (Some("me"), "h"));
        assert_eq!(split_user_host("h"), (None, "h"));
        assert_eq!(parse_port("0"), None);
        assert_eq!(percent_decode("My%20Server"), "My Server");
    }
}
