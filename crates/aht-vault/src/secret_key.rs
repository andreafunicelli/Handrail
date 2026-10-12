//! The 128-bit Secret Key that is combined with the master password (spec §5).
//!
//! Human format: `A1-XXXXXX-XXXXXX-XXXXX-XXXXX-XXXXX` — version prefix `A1`, then 26 Crockford
//! base32 characters encoding the 16 key bytes (2 zero padding bits) plus one checksum
//! character, grouped 6-6-5-5-5. Parsing ignores case, hyphens and whitespace and applies the
//! Crockford substitutions (`O`→`0`, `I`/`L`→`1`).

use crate::error::{Result, VaultError};
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const VERSION: &str = "A1";
const DATA_CHARS: usize = 26;

/// A 128-bit Secret Key. Never printed by `Debug`; use [`SecretKey::reveal`] to display it once
/// in the Emergency Kit.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey(Zeroizing<[u8; 16]>);

impl SecretKey {
    /// Generate a fresh random Secret Key.
    pub fn generate() -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 16]);
        crate::crypto::fill_random(bytes.as_mut())?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// The formatted key, e.g. `A1-3F9KQ2-...`. Handle with care.
    pub fn reveal(&self) -> Zeroizing<String> {
        let value = u128::from_be_bytes(*self.0);
        let mut chars = Zeroizing::new(Vec::with_capacity(DATA_CHARS + 1));
        for i in 0..DATA_CHARS {
            let idx = if i < DATA_CHARS - 1 {
                (value >> (123 - 5 * i)) & 31
            } else {
                (value & 7) << 2
            };
            chars.push(ALPHABET[idx as usize]);
        }
        chars.push(ALPHABET[checksum(&self.0) as usize]);
        let mut out = Zeroizing::new(String::with_capacity(40));
        out.push_str(VERSION);
        let mut pos = 0;
        for group in [6usize, 6, 5, 5, 5] {
            out.push('-');
            for &c in &chars[pos..pos + group] {
                out.push(c as char);
            }
            pos += group;
        }
        out
    }

    /// Parse a formatted Secret Key, tolerating case, hyphens and whitespace.
    pub fn parse(input: &str) -> Result<Self> {
        let norm: Zeroizing<Vec<u8>> = Zeroizing::new(
            input
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '-')
                .map(|c| match c.to_ascii_uppercase() {
                    'O' => '0',
                    'I' | 'L' => '1',
                    other => other,
                } as u8)
                .collect(),
        );
        let bad = |m: &str| VaultError::InvalidSecretKey(m.to_string());
        if norm.len() != VERSION.len() + DATA_CHARS + 1 {
            return Err(bad("wrong length"));
        }
        if &norm[..2] != VERSION.as_bytes() {
            return Err(bad("unknown version prefix"));
        }
        let mut digits = Zeroizing::new([0u8; DATA_CHARS + 1]);
        for (d, &c) in digits.iter_mut().zip(&norm[2..]) {
            *d = ALPHABET
                .iter()
                .position(|&a| a == c)
                .ok_or_else(|| bad("invalid character"))? as u8;
        }
        let mut value: u128 = 0;
        for &d in &digits[..DATA_CHARS - 1] {
            value = (value << 5) | d as u128;
        }
        let last = digits[DATA_CHARS - 1];
        if last & 3 != 0 {
            return Err(bad("checksum mismatch"));
        }
        value = (value << 3) | (last >> 2) as u128;
        let bytes = Zeroizing::new(value.to_be_bytes());
        if checksum(&bytes) != digits[DATA_CHARS] {
            return Err(bad("checksum mismatch"));
        }
        Ok(Self(bytes))
    }
}

fn checksum(bytes: &[u8; 16]) -> u8 {
    let mut h = Sha256::new();
    h.update(VERSION.as_bytes());
    h.update(bytes);
    h.finalize()[0] & 31
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

impl std::str::FromStr for SecretKey {
    type Err = VaultError;
    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_shape() {
        let k = SecretKey::generate().unwrap();
        let s = k.reveal();
        let groups: Vec<&str> = s.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            vec![2, 6, 6, 5, 5, 5]
        );
        assert_eq!(groups[0], "A1");
    }

    #[test]
    fn round_trip_many() {
        for _ in 0..200 {
            let k = SecretKey::generate().unwrap();
            assert_eq!(SecretKey::parse(&k.reveal()).unwrap(), k);
        }
        for bytes in [[0u8; 16], [0xff; 16]] {
            let k = SecretKey::from_bytes(bytes);
            assert_eq!(SecretKey::parse(&k.reveal()).unwrap(), k);
        }
    }

    #[test]
    fn parse_is_tolerant() {
        let k = SecretKey::generate().unwrap();
        let s = k.reveal();
        let messy = format!(
            "  {} ",
            s.to_lowercase()
                .replace('-', " ")
                .replace('0', "o")
                .replace('1', "l")
        );
        assert_eq!(SecretKey::parse(&messy).unwrap(), k);
        let compact: String = s.chars().filter(|c| *c != '-').collect();
        assert_eq!(compact.parse::<SecretKey>().unwrap(), k);
    }

    #[test]
    fn checksum_detects_typos() {
        let mut detected = 0;
        let mut total = 0;
        // Change each data character of many keys to a different symbol: a 5-bit checksum misses
        // about one typo in 32, so across all of them nearly every one must be rejected. (One
        // key alone has too few characters for a stable bound.)
        for _ in 0..100 {
            let k = SecretKey::generate().unwrap();
            let s = k.reveal().to_string();
            for pos in 3..s.len() {
                let c = s.as_bytes()[pos];
                if c == b'-' {
                    continue;
                }
                let repl = if c == b'Z' { 'Y' } else { 'Z' };
                let mut t = s.clone().into_bytes();
                t[pos] = repl as u8;
                total += 1;
                match SecretKey::parse(std::str::from_utf8(&t).unwrap()) {
                    Err(_) => detected += 1,
                    Ok(other) => assert_ne!(other, k),
                }
            }
        }
        assert!(detected * 100 >= total * 92, "{detected}/{total}");
    }

    #[test]
    fn rejects_garbage() {
        assert!(SecretKey::parse("").is_err());
        assert!(SecretKey::parse("A1-XXXX").is_err());
        let k = SecretKey::generate().unwrap();
        let wrong_prefix = k.reveal().replacen("A1", "B1", 1);
        assert!(SecretKey::parse(&wrong_prefix).is_err());
        let bad_char = k.reveal().replacen('-', "-U", 1);
        assert!(SecretKey::parse(&bad_char).is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let k = SecretKey::generate().unwrap();
        let dbg = format!("{k:?}");
        assert_eq!(dbg, "SecretKey(<redacted>)");
        assert!(!dbg.contains(&k.reveal()[3..9]));
    }
}
