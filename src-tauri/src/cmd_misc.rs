//! Import, sync, harness discovery and provider model listing.

use crate::cmd_vault::secret_value;
use crate::state::{err, AppState};
use aht_core::model::{Connection, Group, HarnessKind, Provider};
use aht_core::{ConnectionId, Error, Result};
use aht_import::{DetectedSource, ImportOptions, Tool};
use aht_sync::github_device_flow::{self, DeviceCode, PollResult};
use aht_sync::{FolderBackend, GitHubBackend, SyncBackend, SyncReport, SyncState};
use aht_vault::{Setting, Vault};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;

// ---------- import ----------

#[tauri::command(rename_all = "snake_case")]
pub async fn import_detect() -> Result<Vec<DetectedSource>> {
    tokio::task::spawn_blocking(aht_import::detect)
        .await
        .map_err(err)
}

#[derive(Deserialize)]
pub struct PreviewArgs {
    source_id: Option<String>,
    tool: Option<Tool>,
    file_path: Option<PathBuf>,
    mremoteng_password: Option<String>,
    #[serde(default = "yes")]
    import_passwords: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize)]
pub struct Duplicate {
    index: usize,
    existing_id: ConnectionId,
}

/// What the UI sees of an import: everything except secret values.
#[derive(Serialize)]
pub struct ImportPreview {
    preview_id: String,
    tool: Tool,
    groups: Vec<Group>,
    connections: Vec<Connection>,
    secret_count: usize,
    warnings: Vec<String>,
    duplicates: Vec<Duplicate>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn import_preview(
    state: State<'_, AppState>,
    args: PreviewArgs,
) -> Result<ImportPreview> {
    let opts = ImportOptions {
        mremoteng_password: args.mremoteng_password,
        import_passwords: args.import_passwords,
    };
    let bundle =
        tokio::task::spawn_blocking(move || match (args.source_id, args.tool, args.file_path) {
            (Some(id), _, _) => {
                let src = aht_import::detect()
                    .into_iter()
                    .find(|s| s.id == id)
                    .ok_or_else(|| Error::NotFound(format!("import source {id}")))?;
                aht_import::parse_source(&src, &opts).map_err(err)
            }
            (None, Some(tool), Some(path)) => {
                aht_import::parse_file(tool, &path, &opts).map_err(err)
            }
            _ => Err(Error::Invalid("need source_id or tool + file_path".into())),
        })
        .await
        .map_err(err)??;
    let existing = state
        .vault
        .lock()
        .await
        .list::<Connection>()
        .map_err(Error::from)?;
    let duplicates = aht_import::find_duplicates(&existing, &bundle)
        .into_iter()
        .map(|(index, existing_id)| Duplicate { index, existing_id })
        .collect();
    let preview_id = uuid::Uuid::new_v4().to_string();
    let preview = ImportPreview {
        preview_id: preview_id.clone(),
        tool: bundle.tool,
        groups: bundle.groups.clone(),
        connections: bundle.connections.clone(),
        secret_count: bundle.secrets.len(),
        warnings: bundle.warnings.clone(),
        duplicates,
    };
    state
        .import_previews
        .lock()
        .await
        .insert(preview_id, bundle);
    Ok(preview)
}

#[derive(Serialize)]
pub struct ImportResult {
    imported: usize,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn import_apply(
    state: State<'_, AppState>,
    preview_id: String,
    skip_indices: Vec<usize>,
) -> Result<ImportResult> {
    let bundle = state
        .import_previews
        .lock()
        .await
        .remove(&preview_id)
        .ok_or_else(|| Error::NotFound("import preview expired".into()))?;
    let mut vault = state.vault.lock().await;
    for g in &bundle.groups {
        vault.put(g).map_err(Error::from)?;
    }
    let mut imported = 0;
    for (i, c) in bundle.connections.iter().enumerate() {
        if skip_indices.contains(&i) {
            continue;
        }
        // Store only the secrets this connection references.
        for s in bundle.secrets.iter().filter(|s| references(c, s.id)) {
            vault.put(s).map_err(Error::from)?;
        }
        vault.put(c).map_err(Error::from)?;
        imported += 1;
    }
    Ok(ImportResult { imported })
}

fn references(c: &Connection, id: aht_core::SecretId) -> bool {
    use aht_core::model::SshAuth;
    match &c.auth {
        SshAuth::Password { secret } => *secret == id,
        SshAuth::Key { key, passphrase } => *key == id || *passphrase == Some(id),
        _ => false,
    }
}

// ---------- harnesses & providers ----------

#[tauri::command(rename_all = "snake_case")]
pub async fn detect_harnesses() -> Result<Vec<aht_harness::HarnessInfo>> {
    Ok(aht_harness::detect_all().await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_harness_extensions(kind: HarnessKind) -> Result<aht_harness::Extensions> {
    tokio::task::spawn_blocking(move || aht_harness::list_extensions(kind))
        .await
        .map_err(err)
}

pub async fn provider_client(
    state: &AppState,
    provider_id: &str,
) -> Result<(Provider, std::sync::Arc<dyn aht_providers::ChatProvider>)> {
    let provider = state
        .vault
        .lock()
        .await
        .get::<Provider>(provider_id)
        .map_err(Error::from)?
        .ok_or_else(|| Error::NotFound(format!("provider {provider_id}")))?;
    let key = match provider.api_key {
        Some(id) => Some(secret_value(state, id).await?),
        None => None,
    };
    let client = aht_providers::from_config(&provider, key);
    Ok((provider, client))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_models(state: State<'_, AppState>, provider_id: String) -> Result<Vec<String>> {
    let (_, client) = provider_client(&state, &provider_id).await?;
    let models = client.list_models().await.map_err(aht_core::Error::from)?;
    Ok(models.into_iter().map(|m| m.id).collect())
}

// ---------- sync ----------

const SYNC_TOKEN_KEY: &str = "sync.github_token";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncConfig {
    Github {
        owner: String,
        repo: String,
        branch: String,
    },
    Folder {
        path: PathBuf,
    },
}

#[derive(Default, Serialize, Deserialize)]
struct SyncFile {
    config: Option<SyncConfig>,
    state: SyncState,
    last_sync_ms: Option<i64>,
    last_report: Option<SyncReport>,
}

fn load_sync(state: &AppState) -> SyncFile {
    std::fs::read(&state.paths.sync_state_file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_sync(state: &AppState, f: &SyncFile) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(f).map_err(err)?;
    std::fs::write(&state.paths.sync_state_file, bytes)?;
    Ok(())
}

#[derive(Serialize)]
pub struct SyncStatus {
    config: Option<SyncConfig>,
    last_sync_ms: Option<i64>,
    last_report: Option<SyncReport>,
    /// Remote holds a vault this device has not joined yet.
    can_join: bool,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus> {
    let f = load_sync(&state);
    let initialized = state.vault.lock().await.status().initialized;
    Ok(SyncStatus {
        can_join: f.config.is_some() && !initialized,
        config: f.config,
        last_sync_ms: f.last_sync_ms,
        last_report: f.last_report,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sync_configure_github_start(state: State<'_, AppState>) -> Result<DeviceCode> {
    let code = github_device_flow::start(&github_device_flow::client_id(), "repo")
        .await
        .map_err(Error::from)?;
    *state.github_device.lock().await = Some(code.clone());
    Ok(code)
}

#[derive(Serialize)]
pub struct PollOutcome {
    done: bool,
    /// "pending" | "slow_down" | "expired" | "denied" | "ok"
    status: &'static str,
    repo: Option<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sync_configure_github_poll(state: State<'_, AppState>) -> Result<PollOutcome> {
    let code = state
        .github_device
        .lock()
        .await
        .clone()
        .ok_or_else(|| Error::Invalid("start the GitHub sign-in first".into()))?;
    let pending = |status| {
        Ok(PollOutcome {
            done: false,
            status,
            repo: None,
        })
    };
    let token = match github_device_flow::poll(&github_device_flow::client_id(), &code.device_code)
        .await
        .map_err(Error::from)?
    {
        PollResult::Pending => return pending("pending"),
        PollResult::SlowDown => return pending("slow_down"),
        PollResult::Expired => return pending("expired"),
        PollResult::Denied => return pending("denied"),
        PollResult::Token(t) => t,
    };
    *state.github_device.lock().await = None;
    let owner = github_login(&token).await?;
    let repo = "aihubterm-vault".to_string();
    let backend = GitHubBackend::new(&owner, &repo, "main", &token);
    backend.ensure_repo().await.map_err(Error::from)?;
    store_token(&state, token).await?;
    let mut f = load_sync(&state);
    f.config = Some(SyncConfig::Github {
        owner: owner.clone(),
        repo: repo.clone(),
        branch: "main".into(),
    });
    f.state = SyncState::default();
    save_sync(&state, &f)?;
    Ok(PollOutcome {
        done: true,
        status: "ok",
        repo: Some(format!("{owner}/{repo}")),
    })
}

async fn github_login(token: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    let user: User = reqwest::Client::new()
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .header("User-Agent", "AIHubTerm")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?
        .error_for_status()
        .map_err(|e| Error::Auth(e.to_string()))?
        .json()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    Ok(user.login)
}

/// The GitHub token lives in the (synced, encrypted) vault. Before the vault exists on a new
/// device it is held in memory until `sync_join` + unlock.
async fn store_token(state: &AppState, token: String) -> Result<()> {
    let mut vault = state.vault.lock().await;
    if vault.status().locked {
        *state.pending_github_token.lock().await = Some(token);
        return Ok(());
    }
    vault
        .put(&Setting {
            key: SYNC_TOKEN_KEY.into(),
            value: token.into(),
        })
        .map_err(Error::from)
}

async fn github_token(state: &AppState) -> Result<String> {
    if let Some(t) = state.pending_github_token.lock().await.clone() {
        return Ok(t);
    }
    state
        .vault
        .lock()
        .await
        .get::<Setting>(SYNC_TOKEN_KEY)
        .map_err(Error::from)?
        .and_then(|s| s.value.as_str().map(str::to_owned))
        .ok_or_else(|| Error::Auth("GitHub sign-in required".into()))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sync_configure_folder(state: State<'_, AppState>, path: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&path)?;
    let mut f = load_sync(&state);
    f.config = Some(SyncConfig::Folder { path });
    f.state = SyncState::default();
    save_sync(&state, &f)
}

async fn backend(state: &AppState, cfg: &SyncConfig) -> Result<Box<dyn SyncBackend>> {
    Ok(match cfg {
        SyncConfig::Folder { path } => Box::new(FolderBackend::new(path.clone())),
        SyncConfig::Github {
            owner,
            repo,
            branch,
        } => Box::new(GitHubBackend::new(
            owner,
            repo,
            branch,
            github_token(state).await?,
        )),
    })
}

/// New device: download the remote manifest and create the local (locked) vault from it.
/// The user then unlocks with master password + Secret Key.
#[tauri::command(rename_all = "snake_case")]
pub async fn sync_join(state: State<'_, AppState>) -> Result<()> {
    let f = load_sync(&state);
    let cfg = f
        .config
        .ok_or_else(|| Error::Invalid("sync not configured".into()))?;
    let b = backend(&state, &cfg).await?;
    let manifest = aht_sync::fetch_manifest(b.as_ref())
        .await
        .map_err(Error::from)?;
    let joined = Vault::join(&state.paths.vault_dir, &manifest).map_err(Error::from)?;
    *state.vault.lock().await = joined;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sync_now(state: State<'_, AppState>) -> Result<SyncReport> {
    let mut f = load_sync(&state);
    let cfg = f
        .config
        .clone()
        .ok_or_else(|| Error::Invalid("sync not configured".into()))?;
    let b = backend(&state, &cfg).await?;
    // Persist a token obtained before the vault was unlocked on this device.
    if let Some(t) = state.pending_github_token.lock().await.take() {
        if let Err(e) = store_token(&state, t.clone()).await {
            *state.pending_github_token.lock().await = Some(t);
            return Err(e);
        }
    }
    let report = {
        let mut vault = state.vault.lock().await;
        if vault.status().locked {
            return Err(Error::VaultLocked);
        }
        aht_sync::sync_once(&mut vault, b.as_ref(), &mut f.state)
            .await
            .map_err(Error::from)?
    };
    f.last_sync_ms = Some(chrono_ms());
    f.last_report = Some(report.clone());
    save_sync(&state, &f)?;
    Ok(report)
}

fn chrono_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
