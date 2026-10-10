//! Authentication material. Secrets are passed in by the app (decrypted from the vault) and are
//! never persisted by this crate.

use async_trait::async_trait;
use std::sync::Arc;
use zeroize::Zeroizing;

/// A keyboard-interactive prompt from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KiPrompt {
    pub prompt: String,
    /// Whether the user's answer may be echoed.
    pub echo: bool,
}

/// UI callback for keyboard-interactive authentication.
#[async_trait]
pub trait KeyboardInteractiveHandler: Send + Sync {
    /// Answer every prompt (same order and count). Return `None` to cancel.
    async fn respond(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[KiPrompt],
    ) -> Option<Vec<Zeroizing<String>>>;
}

#[derive(Clone)]
pub enum AuthMaterial {
    Password(Zeroizing<String>),
    /// Private key (OpenSSH, PEM/PKCS#8 or PuTTY format): ed25519, ECDSA, RSA.
    Key {
        pem: Zeroizing<String>,
        passphrase: Option<Zeroizing<String>>,
    },
    /// Keys held by the running ssh-agent (Unix `SSH_AUTH_SOCK`; Windows OpenSSH agent pipe,
    /// then Pageant).
    Agent,
    KeyboardInteractive(Arc<dyn KeyboardInteractiveHandler>),
}

impl AuthMaterial {
    pub fn password(p: impl Into<String>) -> Self {
        Self::Password(Zeroizing::new(p.into()))
    }

    pub fn key(pem: impl Into<String>, passphrase: Option<String>) -> Self {
        Self::Key {
            pem: Zeroizing::new(pem.into()),
            passphrase: passphrase.map(Zeroizing::new),
        }
    }

    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Password(_) => "password",
            Self::Key { .. } => "public-key",
            Self::Agent => "ssh-agent",
            Self::KeyboardInteractive(_) => "keyboard-interactive",
        }
    }
}

impl std::fmt::Debug for AuthMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AuthMaterial::{}(<redacted>)", self.method_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_secrets() {
        let s = format!("{:?}", AuthMaterial::password("hunter2"));
        assert!(!s.contains("hunter2"));
        let s = format!("{:?}", AuthMaterial::key("PEMDATA", Some("pp".into())));
        assert!(!s.contains("PEMDATA") && !s.contains("pp)"));
    }
}
