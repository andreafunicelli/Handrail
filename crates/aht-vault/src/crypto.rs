//! Key derivation and AEAD primitives (spec §5).

use crate::error::{Result, VaultError};
use crate::secret_key::SecretKey;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;
use zeroize::Zeroizing;

const NONCE_LEN: usize = 24;
const HKDF_INFO_ENC: &[u8] = b"aihubterm vault v1";
const HKDF_INFO_FILE_ID: &[u8] = b"aihubterm vault v1 file-id";

pub(crate) fn fill_random(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|e| VaultError::Kdf(format!("os rng failed: {e}")))
}

pub(crate) fn random_hex(n_bytes: usize) -> Result<String> {
    let mut b = vec![0u8; n_bytes];
    fill_random(&mut b)?;
    Ok(hex::encode(b))
}

/// Argon2id cost parameters, stored in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory cost in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Parallelism.
    pub p: u32,
}

impl Default for KdfParams {
    /// m = 64 MiB, t = 3, p = 1.
    fn default() -> Self {
        Self {
            m_kib: 64 * 1024,
            t: 3,
            p: 1,
        }
    }
}

impl KdfParams {
    /// Very cheap parameters. **Tests only** — never use for a real vault.
    pub fn insecure_fast() -> Self {
        Self {
            m_kib: 8,
            t: 1,
            p: 1,
        }
    }
}

/// The derived vault keys: an encryption key and a key for deriving opaque file ids.
pub(crate) struct VaultKey {
    enc: Zeroizing<[u8; 32]>,
    file_id: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(<redacted>)")
    }
}

impl VaultKey {
    /// Argon2id(password, salt) → 32 bytes; HKDF-SHA256(ikm = that ‖ secret key, salt) → keys.
    pub(crate) fn derive(
        password: &str,
        secret_key: &SecretKey,
        salt: &[u8],
        params: KdfParams,
    ) -> Result<Self> {
        let p = Params::new(params.m_kib, params.t, params.p, Some(32))
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
        let mut ikm = Zeroizing::new([0u8; 48]);
        argon
            .hash_password_into(password.as_bytes(), salt, &mut ikm[..32])
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        ikm[32..].copy_from_slice(secret_key.as_bytes());
        let hk = hkdf::Hkdf::<Sha256>::new(Some(salt), &ikm[..]);
        let mut enc = Zeroizing::new([0u8; 32]);
        let mut file_id = Zeroizing::new([0u8; 32]);
        hk.expand(HKDF_INFO_ENC, enc.as_mut())
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        hk.expand(HKDF_INFO_FILE_ID, file_id.as_mut())
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        Ok(Self { enc, file_id })
    }

    /// Encrypt; output is `nonce ‖ ciphertext+tag`.
    pub(crate) fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = XChaCha20Poly1305::new_from_slice(self.enc.as_ref())
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut nonce)?;
        let ct = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| VaultError::Corrupt("encryption failed".into()))?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt `nonce ‖ ciphertext+tag`. Any tampering yields `Corrupt`.
    pub(crate) fn open(&self, aad: &[u8], data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if data.len() < NONCE_LEN + 16 {
            return Err(VaultError::Corrupt("ciphertext too short".into()));
        }
        let cipher = XChaCha20Poly1305::new_from_slice(self.enc.as_ref())
            .map_err(|e| VaultError::Kdf(e.to_string()))?;
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&data[..NONCE_LEN]);
        cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &data[NONCE_LEN..],
                    aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| VaultError::Corrupt("authentication failed".into()))
    }

    /// Opaque, stable 128-bit file id for a logical item: HMAC(file-id key, type ‖ 0 ‖ id).
    /// Unlinkable to the item id without the vault key, identical on every device.
    pub(crate) fn file_id(&self, item_type: &str, id: &str) -> String {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(self.file_id.as_ref())
            .expect("hmac accepts any key length");
        mac.update(item_type.as_bytes());
        mac.update(&[0]);
        mac.update(id.as_bytes());
        hex::encode(&mac.finalize().into_bytes()[..16])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pw: &str, sk: &SecretKey) -> VaultKey {
        VaultKey::derive(pw, sk, b"0123456789abcdef", KdfParams::insecure_fast()).unwrap()
    }

    #[test]
    fn seal_open_round_trip_and_aad_binding() {
        let sk = SecretKey::generate().unwrap();
        let k = key("pw", &sk);
        let ct = k.seal(b"aad", b"hello").unwrap();
        assert_eq!(&k.open(b"aad", &ct).unwrap()[..], b"hello");
        assert!(matches!(k.open(b"other", &ct), Err(VaultError::Corrupt(_))));
        assert!(matches!(
            k.open(b"aad", &ct[..10]),
            Err(VaultError::Corrupt(_))
        ));
        // Fresh nonce each time.
        assert_ne!(k.seal(b"aad", b"hello").unwrap(), ct);
    }

    #[test]
    fn derivation_depends_on_both_factors() {
        let sk = SecretKey::generate().unwrap();
        let sk2 = SecretKey::generate().unwrap();
        let ct = key("pw", &sk).seal(b"", b"x").unwrap();
        assert!(key("pw", &sk).open(b"", &ct).is_ok());
        assert!(key("pw2", &sk).open(b"", &ct).is_err());
        assert!(key("pw", &sk2).open(b"", &ct).is_err());
    }

    #[test]
    fn file_ids_are_stable_and_opaque() {
        let sk = SecretKey::generate().unwrap();
        let k = key("pw", &sk);
        let a = k.file_id("connection", "abc");
        assert_eq!(a, key("pw", &sk).file_id("connection", "abc"));
        assert_eq!(a.len(), 32);
        assert_ne!(a, k.file_id("group", "abc"));
        assert!(!a.contains("abc"));
    }

    #[test]
    fn default_params_are_accepted_by_argon2() {
        let d = KdfParams::default();
        assert!(Params::new(d.m_kib, d.t, d.p, Some(32)).is_ok());
    }

    #[test]
    fn debug_is_redacted() {
        let k = key("pw", &SecretKey::generate().unwrap());
        assert_eq!(format!("{k:?}"), "VaultKey(<redacted>)");
    }
}
