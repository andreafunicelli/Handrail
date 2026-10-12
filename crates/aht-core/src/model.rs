//! Persistent domain model. These are the payloads stored (encrypted) in the vault and synced.

use crate::{
    ConnectionId, GroupId, HarnessProfileId, ProviderId, S3AccountId, SecretId, SnippetId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    Ssh,
    /// Reserved for v1.2.
    Mysql,
    /// Reserved for v1.1.
    Sftp,
    /// Local shell (PowerShell, bash, WSL...).
    Local,
    /// Remote desktop: host, port (3389), user (`DOMAIN\user` or `user@domain`), a saved
    /// password, and optionally jump hosts to tunnel through.
    Rdp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Prod,
    Staging,
    Dev,
    #[default]
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SshAuth {
    Password {
        secret: SecretId,
    },
    Key {
        key: SecretId,
        passphrase: Option<SecretId>,
    },
    Agent,
    /// Ask interactively at connect time.
    Prompt,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    pub kind: ConnectionKind,
    pub group_id: Option<GroupId>,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    /// ProxyJump chain, outermost first.
    #[serde(default)]
    pub jump: Vec<ConnectionId>,
    #[serde(default)]
    pub env: Environment,
    #[serde(default)]
    pub tags: Vec<String>,
    pub color: Option<String>,
    pub approval_mode: Option<ApprovalMode>,
    /// For `Local`: shell program, e.g. "pwsh", "wsl.exe", "/bin/zsh".
    pub local_shell: Option<String>,
    #[serde(default)]
    pub notes: String,
    /// Port forwards started with every session of this connection.
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
    /// For `Rdp`: connect to the console (administrative) session, like `mstsc /admin`.
    #[serde(default)]
    pub rdp_admin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelKind {
    /// `ssh -L`: listen on this computer, connect from the server.
    Local,
    /// `ssh -R`: listen on the server, connect from this computer.
    Remote,
    /// `ssh -D`: a SOCKS5 proxy on this computer; every connection goes out from the server.
    Dynamic,
}

/// A port forward (SSH tunnel).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tunnel {
    pub kind: TunnelKind,
    /// Where to listen: this computer (local, dynamic) or the server (remote).
    #[serde(default = "loopback")]
    pub bind_host: String,
    pub bind_port: u16,
    /// Where connections go, as seen from the server (local) or from this computer (remote).
    /// Unused by dynamic forwards.
    #[serde(default)]
    pub dest_host: String,
    #[serde(default)]
    pub dest_port: u16,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn loopback() -> String {
    "127.0.0.1".into()
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub parent_id: Option<GroupId>,
    pub order: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    Password,
    PrivateKey,
    Passphrase,
    ApiKey,
    Token,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub id: SecretId,
    pub kind: SecretKind,
    pub label: String,
    pub value: String,
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("value", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Every command needs approval.
    #[default]
    ConfirmAll,
    /// Read-only commands run; writes/destructive need approval.
    ConfirmWrites,
    /// Everything runs except deny-listed commands.
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    OpenAiCompat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPreset {
    Anthropic,
    OpenAi,
    DeepSeek,
    OpenCodeGo,
    OpenCodeZen,
    OpenRouter,
    Ollama,
    LmStudio,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: ProviderId,
    pub name: String,
    pub kind: ProviderKind,
    pub preset: ProviderPreset,
    pub base_url: String,
    pub api_key: Option<SecretId>,
    pub default_model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    ClaudeCode,
    Codex,
    OpenCode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessDisplay {
    #[default]
    Sidebar,
    Native,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessProfile {
    pub id: HarnessProfileId,
    pub name: String,
    pub harness: HarnessKind,
    #[serde(default)]
    pub display: HarnessDisplay,
    /// When true, the harness's local shell/edit tools are disabled; it acts only via our MCP tools.
    pub server_mode: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    pub model: Option<String>,
}

/// Which "brain" drives the agent sidebar for a tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Brain {
    BuiltIn { provider: ProviderId, model: String },
    Harness { profile: HarnessProfileId },
}

/// A saved command. `{{name}}` placeholders are filled in by the user when it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: SnippetId,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Credentials for an S3 or S3-compatible store, browsed in the file browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Account {
    pub id: S3AccountId,
    pub name: String,
    /// Service URL for S3-compatible stores; empty for AWS.
    #[serde(default)]
    pub endpoint: String,
    /// "auto" for Cloudflare R2.
    pub region: String,
    /// Open this bucket directly instead of the bucket list.
    #[serde(default)]
    pub bucket: Option<String>,
    pub access_key_id: String,
    /// The secret access key, stored as a vault secret.
    pub secret: Option<SecretId>,
    /// `endpoint/bucket/key` addressing (MinIO and most self-hosted stores).
    #[serde(default)]
    pub path_style: bool,
}
