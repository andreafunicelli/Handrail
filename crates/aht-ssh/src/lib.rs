//! aht-ssh — SSH client sessions (russh), host-key verification, and local PTY shells.
//! See docs/superpowers/specs/2026-10-10-aihubterm-design.md §6.

pub mod auth;
pub mod client;
pub mod forward;
pub mod known_hosts;
pub mod local;
pub mod reconnect;
pub mod sftp;
pub mod stats;
pub mod verify;

pub use auth::{AuthMaterial, KeyboardInteractiveHandler, KiPrompt};
pub use client::{connect_shell, ExecOutput, JumpHop, SshConnection, SshParams, SshShell};
pub use forward::{Forward, ForwardSpec};
pub use known_hosts::{fingerprint, HostKeyStatus, KnownHosts};
pub use local::{default_local_shells, spawn_local, DiscoveredShell, LocalPty, LocalShellSpec};
pub use reconnect::{reconnect, retry_with_backoff, ReconnectPolicy};
pub use sftp::{EntryKind, FileStamp, RemoteEntry, Sftp};
pub use stats::ServerStats;
pub use verify::{
    HostKeyDecision, HostKeyPrompt, HostKeyVerifier, PromptingVerifier, StrictVerifier,
};
