//! Host key verification.

use crate::known_hosts::{HostKeyStatus, KnownHosts};
use async_trait::async_trait;
use russh::keys::{Algorithm, PublicKey};
use std::sync::Arc;

/// What to do with a server's host key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyDecision {
    /// Key is trusted (already known, or newly accepted).
    Accept,
    /// Key changed and the user explicitly chose to replace the recorded key.
    /// This is the only decision that lets a changed key through.
    ReplaceAndTrust,
    /// Refuse the connection (e.g. user declined a new key).
    Reject,
    /// Key does not match the recorded one; connect fails with `Error::HostKeyMismatch`.
    Mismatch { old: String, new: String },
}

#[async_trait]
pub trait HostKeyVerifier: Send + Sync {
    async fn verify(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyDecision;
    /// Key types already recorded for `host:port`; the connection asks for these first.
    fn known_algorithms(&self, _host: &str, _port: u16) -> Vec<Algorithm> {
        Vec::new()
    }
}

/// UI callback used by [`PromptingVerifier`].
#[async_trait]
pub trait HostKeyPrompt: Send + Sync {
    /// First time seeing this host (or this key type). Return `true` to trust and record it.
    async fn confirm_new(&self, host: &str, port: u16, fingerprint: &str) -> bool;
    /// Recorded key differs. Return `true` ONLY if the user explicitly chose to replace it.
    async fn confirm_replace(&self, host: &str, port: u16, old: &str, new: &str) -> bool;
}

/// Verifier backed by a [`KnownHosts`] store that asks a [`HostKeyPrompt`] about new and
/// changed keys, and records the user's choice.
pub struct PromptingVerifier {
    pub store: KnownHosts,
    pub prompt: Arc<dyn HostKeyPrompt>,
}

impl PromptingVerifier {
    pub fn new(store: KnownHosts, prompt: Arc<dyn HostKeyPrompt>) -> Self {
        Self { store, prompt }
    }
}

#[async_trait]
impl HostKeyVerifier for PromptingVerifier {
    fn known_algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        self.store.algorithms(host, port)
    }

    async fn verify(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyDecision {
        match self.store.check(host, port, key) {
            HostKeyStatus::Known => HostKeyDecision::Accept,
            HostKeyStatus::New { fingerprint } => {
                if !self.prompt.confirm_new(host, port, &fingerprint).await {
                    return HostKeyDecision::Reject;
                }
                if let Err(e) = self.store.add(host, port, key) {
                    tracing::warn!("failed to record host key for {host}:{port}: {e}");
                }
                HostKeyDecision::Accept
            }
            HostKeyStatus::Changed { old, new } => {
                if !self.prompt.confirm_replace(host, port, &old, &new).await {
                    return HostKeyDecision::Mismatch { old, new };
                }
                match self.store.replace(host, port, key) {
                    Ok(()) => HostKeyDecision::ReplaceAndTrust,
                    Err(e) => {
                        tracing::warn!("failed to replace host key for {host}:{port}: {e}");
                        HostKeyDecision::Mismatch { old, new }
                    }
                }
            }
        }
    }
}

/// Non-interactive verifier: known keys pass, new keys are accepted and recorded only if
/// `accept_new`, changed keys always fail.
pub struct StrictVerifier {
    pub store: KnownHosts,
    pub accept_new: bool,
}

#[async_trait]
impl HostKeyVerifier for StrictVerifier {
    fn known_algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        self.store.algorithms(host, port)
    }

    async fn verify(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyDecision {
        match self.store.check(host, port, key) {
            HostKeyStatus::Known => HostKeyDecision::Accept,
            HostKeyStatus::New { .. } if self.accept_new => match self.store.add(host, port, key) {
                Ok(()) => HostKeyDecision::Accept,
                Err(_) => HostKeyDecision::Reject,
            },
            HostKeyStatus::New { .. } => HostKeyDecision::Reject,
            HostKeyStatus::Changed { old, new } => HostKeyDecision::Mismatch { old, new },
        }
    }
}
