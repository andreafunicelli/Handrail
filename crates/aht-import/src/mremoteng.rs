//! mRemoteNG `confCons.xml`.
//!
//! Verified against mRemoteNG v1.78.2-dev sources: `XmlConnectionsDeserializer.cs`,
//! `XmlConnectionsDecryptor.cs`, `AeadCryptographyProvider.cs`, `Pkcs5S2KeyGenerator.cs` and
//! `LegacyRijndaelCryptographyProvider.cs`.
//!
//! * ConfVersion >= 2.6: AES-256-GCM. Blob (base64) = salt(16) || nonce(16) || ciphertext || tag(16);
//!   the salt is also the AAD; key = PBKDF2-HMAC-SHA1(password, salt, `KdfIterations`, 32 bytes)
//!   where the password is converted with BouncyCastle's `Pkcs5PasswordToBytes` (low byte of each
//!   UTF-16 unit). The nonce is 128 bit, not the usual 96.
//! * Older files: AES-128-CBC, key = MD5(password), IV = first 16 bytes of the blob, PKCS#7.
//! * `Protected` decrypts to `ThisIsNotProtected` (default password) or `ThisIsProtected`; a
//!   successful decryption authenticates the master password (`ConnectionsFileIsAuthentic`).
//! * `FullFileEncryption="True"`: the root element's text is one blob that decrypts to the
//!   child `<Node>` XML.
//! * The default master password is `mR3m`.

use crate::builder::Builder;
use crate::text::{parse_port, read_text};
use crate::{ImportBundle, ImportOptions, Tool};
use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, KeyInit};
use aht_core::{Error, Result};
use base64::Engine;
use roxmltree::Node;
use std::path::Path;

const DEFAULT_PASSWORD: &str = "mR3m";

type Gcm256N16 = AesGcm<Aes256, U16>;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// BouncyCastle `PbeParametersGenerator.Pkcs5PasswordToBytes`: low 8 bits of each UTF-16 unit.
fn pkcs5_password_bytes(p: &str) -> Vec<u8> {
    p.encode_utf16().map(|u| u as u8).collect()
}

fn derive_key(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(&pkcs5_password_bytes(password), salt, iterations, &mut key);
    key
}

fn decrypt_gcm(blob_b64: &str, password: &str, iterations: u32) -> Result<String> {
    let bad =
        || Error::Auth("mRemoteNG data could not be decrypted (wrong master password?)".into());
    let data = b64().decode(blob_b64.trim()).map_err(|_| bad())?;
    if data.len() < 16 + 16 + 16 {
        return Err(bad());
    }
    let (salt, rest) = data.split_at(16);
    let (nonce, ct) = rest.split_at(16);
    let key = derive_key(password, salt, iterations);
    let cipher = Gcm256N16::new_from_slice(&key).map_err(|_| bad())?;
    let nonce = aes_gcm::Nonce::<U16>::try_from(nonce).map_err(|_| bad())?;
    let plain = cipher
        .decrypt(&nonce, Payload { msg: ct, aad: salt })
        .map_err(|_| bad())?;
    String::from_utf8(plain).map_err(|_| bad())
}

fn decrypt_legacy(blob_b64: &str, password: &str) -> Result<String> {
    use cbc::cipher::block_padding::Pkcs7;
    use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
    use md5::{Digest, Md5};
    let bad =
        || Error::Auth("mRemoteNG data could not be decrypted (wrong master password?)".into());
    let data = b64().decode(blob_b64.trim()).map_err(|_| bad())?;
    if data.len() < 32 || password.is_empty() {
        return Err(bad());
    }
    let key = Md5::digest(password.as_bytes());
    let (iv, ct) = data.split_at(16);
    let dec = cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, iv).map_err(|_| bad())?;
    let plain = dec.decrypt_padded_vec::<Pkcs7>(ct).map_err(|_| bad())?;
    String::from_utf8(plain).map_err(|_| bad())
}

enum Cipher {
    Gcm { iterations: u32 },
    Legacy,
    Unsupported(String),
}

struct Decryptor {
    cipher: Cipher,
    password: String,
}

impl Decryptor {
    fn decrypt(&self, blob: &str) -> Result<String> {
        if blob.trim().is_empty() {
            return Ok(String::new());
        }
        match &self.cipher {
            Cipher::Gcm { iterations } => decrypt_gcm(blob, &self.password, *iterations),
            Cipher::Legacy => decrypt_legacy(blob, &self.password),
            Cipher::Unsupported(what) => Err(Error::Invalid(format!(
                "unsupported mRemoteNG encryption: {what} (only AES/GCM and legacy AES/CBC)"
            ))),
        }
    }
}

pub fn parse_file(path: &Path, opts: &ImportOptions) -> Result<ImportBundle> {
    parse_text(&read_text(path)?, opts)
}

fn attr<'a>(n: Node<'a, '_>, name: &str) -> &'a str {
    n.attribute(name).unwrap_or("")
}

fn is_true(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

pub(crate) fn parse_text(text: &str, opts: &ImportOptions) -> Result<ImportBundle> {
    let password = opts
        .mremoteng_password
        .clone()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| DEFAULT_PASSWORD.to_string());
    let mut text = text.trim_start_matches('\u{feff}').to_string();

    // Pre-2.6 full-file encryption: the whole file is a legacy blob, not XML.
    if !text.trim_start().starts_with('<') {
        text = decrypt_legacy(&text, &password)?;
    }

    let doc = roxmltree::Document::parse_with_options(
        &text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| Error::Invalid(format!("invalid mRemoteNG XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Connections" {
        return Err(Error::Invalid("not an mRemoteNG confCons.xml".into()));
    }
    let conf_version: f64 = attr(root, "ConfVersion")
        .replace(',', ".")
        .parse()
        .unwrap_or(0.0);

    let cipher = if conf_version >= 2.6 {
        let engine = attr(root, "EncryptionEngine");
        let mode = attr(root, "BlockCipherMode");
        if engine.eq_ignore_ascii_case("AES") && mode.eq_ignore_ascii_case("GCM") {
            Cipher::Gcm {
                iterations: attr(root, "KdfIterations").parse().unwrap_or(1000),
            }
        } else {
            Cipher::Unsupported(format!("{engine}/{mode}"))
        }
    } else {
        Cipher::Legacy
    };
    let dec = Decryptor { cipher, password };

    if conf_version > 1.3 {
        let protected = attr(root, "Protected");
        // Plaintext is `ThisIsNotProtected` (default password) or `ThisIsProtected` (custom one);
        // a successful decryption (GCM tag / padding) is what proves the password is right.
        if !protected.is_empty() {
            dec.decrypt(protected)?;
        }
    }

    let mut b = Builder::new(Tool::MRemoteNg, opts);
    if conf_version > 2.8 {
        b.warn(format!(
            "ConfVersion {conf_version} is newer than the supported 2.8; results may be incomplete"
        ));
    }

    let inherited = Creds::default();
    if conf_version >= 2.6 && is_true(attr(root, "FullFileEncryption")) {
        let blob: String = root
            .children()
            .filter(|c| c.is_text())
            .filter_map(|c| c.text())
            .collect();
        let inner = dec.decrypt(&blob)?;
        let wrapped = format!("<Connections>{inner}</Connections>");
        let inner_doc = roxmltree::Document::parse(&wrapped)
            .map_err(|e| Error::Invalid(format!("invalid decrypted mRemoteNG XML: {e}")))?;
        walk(
            inner_doc.root_element(),
            &mut Vec::new(),
            &inherited,
            &mut b,
            &dec,
        );
    } else {
        walk(root, &mut Vec::new(), &inherited, &mut b, &dec);
    }
    Ok(b.finish())
}

/// Effective (possibly inherited) credentials of a container; password still encrypted.
#[derive(Clone, Default)]
struct Creds {
    user: String,
    pass_blob: String,
    domain: String,
}

fn effective_creds(n: Node, parent: &Creds) -> Creds {
    Creds {
        user: if is_true(attr(n, "InheritUsername")) {
            parent.user.clone()
        } else {
            attr(n, "Username").to_string()
        },
        pass_blob: if is_true(attr(n, "InheritPassword")) {
            parent.pass_blob.clone()
        } else {
            attr(n, "Password").to_string()
        },
        domain: if is_true(attr(n, "InheritDomain")) {
            parent.domain.clone()
        } else {
            attr(n, "Domain").to_string()
        },
    }
}

fn walk(parent: Node, path: &mut Vec<String>, inherited: &Creds, b: &mut Builder, dec: &Decryptor) {
    for n in parent.children().filter(|c| c.has_tag_name("Node")) {
        let name = attr(n, "Name").to_string();
        if attr(n, "Type").eq_ignore_ascii_case("Container") {
            path.push(name);
            b.group_path(path);
            let creds = effective_creds(n, inherited);
            walk(n, path, &creds, b, dec);
            path.pop();
            continue;
        }
        let proto = attr(n, "Protocol");
        let rdp = proto.eq_ignore_ascii_case("RDP");
        if !(rdp || proto.eq_ignore_ascii_case("SSH2") || proto.eq_ignore_ascii_case("SSH1")) {
            b.skip("mRemoteNG connections that are neither SSH nor RDP");
            continue;
        }
        let host = attr(n, "Hostname").trim();
        if host.is_empty() {
            b.skip("mRemoteNG connections without a host");
            continue;
        }
        let creds = effective_creds(n, inherited);
        let auth = if creds.pass_blob.is_empty() {
            aht_core::model::SshAuth::Prompt
        } else {
            match dec.decrypt(&creds.pass_blob) {
                Ok(pw) => b.password_auth(&name, &pw),
                Err(e) => {
                    b.warn(format!("{name}: password not imported: {e}"));
                    aht_core::model::SshAuth::Prompt
                }
            }
        };
        let group = b.group_path(path);
        let display = if name.is_empty() { host } else { &name };
        if rdp {
            // Windows logs in as DOMAIN\user; mRemoteNG keeps the domain apart.
            let domain = creds.domain.trim();
            let user = if domain.is_empty() || creds.user.contains('\\') || creds.user.contains('@')
            {
                creds.user.clone()
            } else {
                format!("{domain}\\{}", creds.user)
            };
            if matches!(auth, aht_core::model::SshAuth::Prompt) {
                b.warn(format!(
                    "{display}: no saved password; add one before connecting (RDP needs it)"
                ));
            }
            b.add_rdp(
                display,
                group,
                host,
                parse_port(attr(n, "Port")).unwrap_or(3389),
                &user,
                auth,
                attr(n, "Descr").to_string(),
                is_true(attr(n, "UseConsoleSession")),
            );
        } else {
            b.add(
                display,
                group,
                host,
                parse_port(attr(n, "Port")).unwrap_or(22),
                &creds.user,
                auth,
                attr(n, "Descr").to_string(),
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// Encryptor mirroring `AeadCryptographyProvider.Encrypt` with a fixed salt/nonce.
    pub fn encrypt_gcm(
        plain: &str,
        password: &str,
        iterations: u32,
        salt: [u8; 16],
        nonce: [u8; 16],
    ) -> String {
        let key = derive_key(password, &salt, iterations);
        let cipher = Gcm256N16::new_from_slice(&key).unwrap();
        let n = aes_gcm::Nonce::<U16>::try_from(&nonce[..]).unwrap();
        let ct = cipher
            .encrypt(
                &n,
                Payload {
                    msg: plain.as_bytes(),
                    aad: &salt,
                },
            )
            .unwrap();
        let mut out = salt.to_vec();
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        b64().encode(out)
    }

    /// Encryptor mirroring `LegacyRijndaelCryptographyProvider.Encrypt` with a fixed IV.
    pub fn encrypt_legacy(plain: &str, password: &str, iv: [u8; 16]) -> String {
        use cbc::cipher::block_padding::Pkcs7;
        use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
        use md5::{Digest, Md5};
        let key = Md5::digest(password.as_bytes());
        let enc = cbc::Encryptor::<aes::Aes128>::new_from_slices(&key, &iv).unwrap();
        let ct = enc.encrypt_padded_vec::<Pkcs7>(plain.as_bytes());
        let mut out = iv.to_vec();
        out.extend_from_slice(&ct);
        b64().encode(out)
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::fixture;
    use aht_core::model::SshAuth;

    const PROTECTED_PLAINTEXT: &str = "ThisIsNotProtected";
    const SALT: [u8; 16] = *b"0123456789abcdef";
    const NONCE: [u8; 16] = *b"fedcba9876543210";

    /// Protected attribute of an upstream mRemoteNG test file (default password, 1000 iterations);
    /// generated by the real C# implementation, so it validates our decryptor independently.
    const UPSTREAM_PROTECTED: &str =
        "8LmIO3+MWBY0zTmfjfOEdCGxhTAwnlohb1veTGNZFt6lAYvY2UOzWyjVzkx6V93smpbP0ZOuexN15u7rvwJEjawC";

    fn pw_of(b: &ImportBundle, conn: &str) -> Option<String> {
        let c = b.connections.iter().find(|c| c.name == conn)?;
        match &c.auth {
            SshAuth::Password { secret } => b
                .secrets
                .iter()
                .find(|s| s.id == *secret)
                .map(|s| s.value.clone()),
            _ => None,
        }
    }

    fn xml(master: &str, iters: u32, body_pw: &str) -> String {
        let protected = encrypt_gcm(PROTECTED_PLAINTEXT, master, iters, SALT, NONCE);
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Connections Name="Connections" Export="False" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="{iters}" FullFileEncryption="False" Protected="{protected}" ConfVersion="2.6">
  <Node Name="srv" Type="Connection" Descr="note" Username="root" Password="{body_pw}" Hostname="srv.example.test" Protocol="SSH2" Port="22" />
</Connections>"#
        )
    }

    #[test]
    fn decrypts_vector_produced_by_upstream_csharp() {
        assert_eq!(
            decrypt_gcm(UPSTREAM_PROTECTED, "mR3m", 1000).unwrap(),
            PROTECTED_PLAINTEXT
        );
        assert!(decrypt_gcm(UPSTREAM_PROTECTED, "wrong", 1000).is_err());
        // A per-connection Password attribute from the same upstream file.
        let field = "fdZPNiJlK9u/uRr22ViSGc8w69qHSDjWw4oeqJWCqFS+kT03rMOOdCczpCtvs6sX59iIcxetSA==";
        assert_eq!(decrypt_gcm(field, "mR3m", 1000).unwrap(), "folder1");
    }

    #[test]
    fn gcm_roundtrip_and_per_field_default_password() {
        let pw = encrypt_gcm("s3cret!", "mR3m", 1000, SALT, NONCE);
        let b = parse_text(&xml("mR3m", 1000, &pw), &ImportOptions::default()).unwrap();
        assert_eq!(pw_of(&b, "srv").as_deref(), Some("s3cret!"));
        assert_eq!(b.connections[0].user, "root");
        assert_eq!(b.connections[0].notes, "note");
    }

    #[test]
    fn custom_master_password_and_iterations() {
        let pw = encrypt_gcm("pässword", "Master pw", 5000, SALT, NONCE);
        let text = xml("Master pw", 5000, &pw);
        let opts = ImportOptions {
            mremoteng_password: Some("Master pw".into()),
            ..Default::default()
        };
        let b = parse_text(&text, &opts).unwrap();
        assert_eq!(pw_of(&b, "srv").as_deref(), Some("pässword"));
        // Default password must be rejected via the Protected check.
        let err = parse_text(&text, &ImportOptions::default()).unwrap_err();
        assert!(matches!(err, Error::Auth(_)), "{err:?}");
    }

    #[test]
    fn full_file_encryption() {
        let inner = r#"<Node Name="folder" Type="Container"><Node Name="deep" Type="Connection" Username="u" Password="" Hostname="deep.example.test" Protocol="SSH2" Port="2022" /></Node>"#;
        let blob = encrypt_gcm(inner, "mR3m", 1000, SALT, NONCE);
        let protected = encrypt_gcm(PROTECTED_PLAINTEXT, "mR3m", 1000, SALT, NONCE);
        let text = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Connections Name="Connections" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="True" Protected="{protected}" ConfVersion="2.6">{blob}</Connections>"#
        );
        let b = parse_text(&text, &ImportOptions::default()).unwrap();
        assert_eq!(b.connections.len(), 1);
        assert_eq!(b.connections[0].port, 2022);
        assert_eq!(b.groups[0].name, "folder");
    }

    #[test]
    fn legacy_cbc_files() {
        let iv = *b"ivivivivivivivi!";
        let pw = encrypt_legacy("oldpass", "mR3m", iv);
        let text = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Connections Name="Connections" Export="False" Protected="{}" ConfVersion="2.5"><Node Name="old" Type="Connection" Username="x" Password="{pw}" Hostname="old.example.test" Protocol="SSH2" Port="22" /></Connections>"#,
            encrypt_legacy(PROTECTED_PLAINTEXT, "mR3m", iv)
        );
        let b = parse_text(&text, &ImportOptions::default()).unwrap();
        assert_eq!(pw_of(&b, "old").as_deref(), Some("oldpass"));
        // Whole-file legacy encryption.
        let whole = encrypt_legacy(&text, "mR3m", iv);
        let b = parse_text(&whole, &ImportOptions::default()).unwrap();
        assert_eq!(pw_of(&b, "old").as_deref(), Some("oldpass"));
    }

    #[test]
    fn hierarchy_protocols_and_inheritance() {
        let text = std::fs::read_to_string(fixture("mremoteng/confCons.xml")).unwrap();
        let b = parse_text(&text, &ImportOptions::default()).unwrap();
        let names: Vec<_> = b.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["top ssh", "web1", "rdp1", "legacy ssh1"]);
        assert_eq!(b.groups.len(), 3); // Prod, Prod/EU, Empty
        let eu = b.groups.iter().find(|g| g.name == "EU").unwrap();
        let prod = b.groups.iter().find(|g| g.name == "Prod").unwrap();
        assert_eq!(eu.parent_id, Some(prod.id));
        assert_eq!(b.connections[1].group_id, Some(eu.id));
        assert_eq!(b.connections[0].group_id, None);
        // Username inherited from the container "Prod".
        assert_eq!(b.connections[1].user, "prodadmin");
        assert_eq!(b.connections[1].port, 2222);
        // Windows servers come in as RDP (issue #31); only Telnet is left out.
        let rdp1 = b.connections.iter().find(|c| c.name == "rdp1").unwrap();
        assert_eq!(rdp1.kind, aht_core::model::ConnectionKind::Rdp);
        assert_eq!((rdp1.port, rdp1.group_id), (3389, Some(eu.id)));
        assert!(
            b.warnings
                .iter()
                .any(|w| w.contains("Skipped 1 mRemoteNG connections that are neither SSH nor RDP")),
            "{:?}",
            b.warnings
        );
    }

    #[test]
    fn rdp_connections_keep_domain_password_and_console() {
        let pw = encrypt_gcm("W1nd0ws!", "mR3m", 1000, SALT, NONCE);
        let protected = encrypt_gcm(PROTECTED_PLAINTEXT, "mR3m", 1000, SALT, NONCE);
        let text = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Connections Name="Connections" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="False" Protected="{protected}" ConfVersion="2.6">
  <Node Name="AD" Type="Container" Username="admin" Domain="CORP" Password="{pw}" Hostname="" Protocol="RDP" Port="3389">
    <Node Name="dc01" Type="Connection" Username="" Password="" Hostname="dc01.corp.test" Protocol="RDP" Port="" UseConsoleSession="True" InheritUsername="True" InheritDomain="True" InheritPassword="True" />
    <Node Name="file01" Type="Connection" Username="svc@corp.test" Domain="CORP" Password="" Hostname="file01.corp.test" Protocol="RDP" Port="3390" />
  </Node>
</Connections>"#
        );
        let b = parse_text(&text, &ImportOptions::default()).unwrap();
        let dc = b.connections.iter().find(|c| c.name == "dc01").unwrap();
        assert_eq!(dc.kind, aht_core::model::ConnectionKind::Rdp);
        assert_eq!(dc.user, "CORP\\admin");
        assert_eq!(dc.port, 3389);
        assert!(dc.rdp_admin);
        assert_eq!(pw_of(&b, "dc01").as_deref(), Some("W1nd0ws!"));
        // A UPN keeps its own form; no password means a warning, not a skip.
        let file = b.connections.iter().find(|c| c.name == "file01").unwrap();
        assert_eq!(
            (file.user.as_str(), file.port, file.rdp_admin),
            ("svc@corp.test", 3390, false)
        );
        assert!(b
            .warnings
            .iter()
            .any(|w| w.contains("file01: no saved password")));
    }

    #[test]
    fn unsupported_cipher_fails_cleanly() {
        let text = r#"<Connections EncryptionEngine="Twofish" BlockCipherMode="EAX" KdfIterations="1000" Protected="AAAA" ConfVersion="2.6"/>"#;
        let err = parse_text(text, &ImportOptions::default()).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }
}
