//! Vault lifecycle and stored records (connections, groups, secrets, providers, profiles).

use crate::state::{err, AppState};
use aht_core::model::{Connection, Group, HarnessProfile, Provider, Secret, SecretKind};
use aht_core::{Error, Result, SecretId};
use aht_vault::{
    ConflictRecord, KdfParams, SecretKey, Setting, SnapshotInfo, Vault, VaultItem, VaultStatus,
};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

#[derive(Serialize)]
pub struct CreatedVault {
    secret_key: String,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_status(state: State<'_, AppState>) -> Result<VaultStatus> {
    Ok(state.vault.lock().await.status())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_create(state: State<'_, AppState>, password: String) -> Result<CreatedVault> {
    if password.chars().count() < 10 {
        return Err(Error::Invalid(
            "master password must be at least 10 characters".into(),
        ));
    }
    let dir = state.paths.vault_dir.clone();
    let (vault, key) =
        tokio::task::spawn_blocking(move || Vault::create(dir, &password, KdfParams::default()))
            .await
            .map_err(err)?
            .map_err(Error::from)?;
    if let Some(id) = vault.vault_id() {
        // Best effort: without a keychain the user types the Secret Key on unlock.
        if let Err(e) = state.keystore.set(id, &key) {
            tracing::warn!("could not store secret key in keychain: {e}");
        }
    }
    *state.vault.lock().await = vault;
    Ok(CreatedVault {
        secret_key: key.reveal().to_string(),
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_unlock(
    state: State<'_, AppState>,
    password: String,
    secret_key: Option<String>,
) -> Result<()> {
    let vault = state.vault.clone();
    let mut guard = vault.lock_owned().await;
    let vault_id = guard
        .vault_id()
        .map(str::to_owned)
        .ok_or(Error::NotFound("vault".into()))?;
    let typed = secret_key.as_deref().filter(|s| !s.trim().is_empty());
    let key = match typed {
        Some(s) => SecretKey::parse(s).map_err(Error::from)?,
        None => state
            .keystore
            .get(&vault_id)
            .ok()
            .flatten()
            // Missing or unreadable keychain entry: the user types the Secret Key instead.
            .ok_or_else(|| Error::Auth("secret_key_required".into()))?,
    };
    let key = tokio::task::spawn_blocking(move || -> Result<SecretKey> {
        guard.unlock(&password, &key).map_err(Error::from)?;
        Ok(key)
    })
    .await
    .map_err(err)??;
    if typed.is_some() {
        let _ = state.keystore.set(&vault_id, &key);
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_lock(state: State<'_, AppState>) -> Result<()> {
    state.vault.lock().await.lock();
    // Nothing decrypted may outlive the lock: agents hold API keys, previews hold passwords.
    state.terminal.on_lock().await;
    state.import_previews.lock().await.clear();
    *state.pending_github_token.lock().await = None;
    Ok(())
}

/// Settings under `sync.` hold credentials and are managed by the sync commands only.
fn check_setting_key(key: &str) -> Result<()> {
    if key.starts_with("sync.") {
        return Err(Error::Denied(format!("setting {key} is not accessible")));
    }
    Ok(())
}

async fn list<T: VaultItem>(state: &AppState) -> Result<Vec<T>> {
    state.vault.lock().await.list::<T>().map_err(Error::from)
}

async fn put<T: VaultItem>(state: &AppState, item: &T) -> Result<()> {
    state.vault.lock().await.put(item).map_err(Error::from)
}

async fn delete<T: VaultItem>(state: &AppState, id: &str) -> Result<()> {
    state
        .vault
        .lock()
        .await
        .delete::<T>(id)
        .map(|_| ())
        .map_err(Error::from)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_connections(state: State<'_, AppState>) -> Result<Vec<Connection>> {
    list(&state).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_connection(
    state: State<'_, AppState>,
    connection: Connection,
) -> Result<Connection> {
    put(&state, &connection).await?;
    Ok(connection)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> Result<()> {
    delete::<Connection>(&state, &id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_groups(state: State<'_, AppState>) -> Result<Vec<Group>> {
    list(&state).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_group(state: State<'_, AppState>, group: Group) -> Result<Group> {
    put(&state, &group).await?;
    Ok(group)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_group(state: State<'_, AppState>, id: String) -> Result<()> {
    // Children move up to the deleted group's parent so nothing becomes orphaned.
    let mut vault = state.vault.lock().await;
    let groups = vault.list::<Group>().map_err(Error::from)?;
    let Some(target) = groups.iter().find(|g| g.id.to_string() == id).cloned() else {
        return Ok(());
    };
    for mut g in groups
        .into_iter()
        .filter(|g| g.parent_id == Some(target.id))
    {
        g.parent_id = target.parent_id;
        vault.put(&g).map_err(Error::from)?;
    }
    for mut c in vault.list::<Connection>().map_err(Error::from)? {
        if c.group_id == Some(target.id) {
            c.group_id = target.parent_id;
            vault.put(&c).map_err(Error::from)?;
        }
    }
    vault.delete::<Group>(&id).map(|_| ()).map_err(Error::from)
}

/// Secret metadata without the value: the UI never reads secrets back.
#[derive(Serialize)]
pub struct SecretMeta {
    id: SecretId,
    kind: SecretKind,
    label: String,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_secret(state: State<'_, AppState>, secret: Secret) -> Result<SecretId> {
    put(&state, &secret).await?;
    Ok(secret.id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_secrets(state: State<'_, AppState>) -> Result<Vec<SecretMeta>> {
    Ok(list::<Secret>(&state)
        .await?
        .into_iter()
        .map(|s| SecretMeta {
            id: s.id,
            kind: s.kind,
            label: s.label,
        })
        .collect())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_providers(state: State<'_, AppState>) -> Result<Vec<Provider>> {
    list(&state).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_provider(
    state: State<'_, AppState>,
    mut provider: Provider,
    api_key: Option<String>,
) -> Result<Provider> {
    if let Some(value) = api_key.filter(|k| !k.trim().is_empty()) {
        let id = provider.api_key.unwrap_or_default();
        let secret = Secret {
            id,
            kind: SecretKind::ApiKey,
            label: format!("{} API key", provider.name),
            value: value.trim().to_string(),
        };
        put(&state, &secret).await?;
        provider.api_key = Some(id);
    }
    put(&state, &provider).await?;
    Ok(provider)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<()> {
    let mut vault = state.vault.lock().await;
    if let Some(p) = vault.get::<Provider>(&id).map_err(Error::from)? {
        if let Some(key) = p.api_key {
            vault
                .delete::<Secret>(&key.to_string())
                .map_err(Error::from)?;
        }
    }
    vault
        .delete::<Provider>(&id)
        .map(|_| ())
        .map_err(Error::from)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_harness_profiles(state: State<'_, AppState>) -> Result<Vec<HarnessProfile>> {
    list(&state).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_harness_profile(
    state: State<'_, AppState>,
    profile: HarnessProfile,
) -> Result<HarnessProfile> {
    put(&state, &profile).await?;
    Ok(profile)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_harness_profile(state: State<'_, AppState>, id: String) -> Result<()> {
    delete::<HarnessProfile>(&state, &id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<Value>> {
    check_setting_key(&key)?;
    let vault = state.vault.lock().await;
    Ok(vault
        .get::<Setting>(&key)
        .map_err(Error::from)?
        .map(|s| s.value))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_setting(state: State<'_, AppState>, key: String, value: Value) -> Result<()> {
    check_setting_key(&key)?;
    put(&state, &Setting { key, value }).await
}

// ---------- snapshots & conflict history ----------

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_snapshots(state: State<'_, AppState>) -> Result<Vec<SnapshotInfo>> {
    let mut list = state
        .vault
        .lock()
        .await
        .list_snapshots()
        .map_err(Error::from)?;
    list.reverse(); // newest first
    Ok(list)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_snapshot_create(state: State<'_, AppState>) -> Result<SnapshotInfo> {
    state.vault.lock().await.snapshot().map_err(Error::from)
}

/// Roll back to a snapshot. A safety snapshot of the current state is taken first, and the
/// rollback is written as fresh edits so it wins on the next sync.
#[tauri::command(rename_all = "snake_case")]
pub async fn vault_snapshot_restore(state: State<'_, AppState>, name: String) -> Result<()> {
    state
        .vault
        .lock()
        .await
        .restore_snapshot(&name)
        .map_err(Error::from)
}

/// A version that lost a sync merge, without its payload: secrets never reach the UI.
#[derive(Serialize)]
pub struct ConflictView {
    file_id: String,
    recorded_at_ms: u64,
    item_type: String,
    /// Name of the item as the losing version had it (connection name, secret label...).
    label: String,
    /// When the losing version was written, and on which device.
    loser_ms: u64,
    loser_device: String,
    loser_deleted: bool,
    /// When the version that won was written.
    winner_ms: u64,
}

fn conflict_view(r: ConflictRecord) -> ConflictView {
    let p = &r.loser.payload;
    let field = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let label = match r.loser.item_type.as_str() {
        "secret" => field("label"),
        "setting" => field("key"),
        _ => field("name"),
    }
    .filter(|l| !l.is_empty())
    .unwrap_or_else(|| r.loser.id.clone());
    ConflictView {
        file_id: r.file_id,
        recorded_at_ms: r.recorded_at_ms,
        item_type: r.loser.item_type,
        label,
        loser_ms: r.loser.updated_at.wall_ms,
        loser_device: r.loser.device_id.to_string(),
        loser_deleted: r.loser.deleted,
        winner_ms: r.winner.wall_ms,
    }
}

/// Versions discarded by last-writer-wins merges on this device, newest first.
#[tauri::command(rename_all = "snake_case")]
pub async fn vault_conflicts(state: State<'_, AppState>) -> Result<Vec<ConflictView>> {
    let history = state
        .vault
        .lock()
        .await
        .conflict_history()
        .map_err(Error::from)?;
    Ok(history.into_iter().rev().map(conflict_view).collect())
}

/// Bring a discarded version back as a fresh edit; it then syncs to the other devices.
#[tauri::command(rename_all = "snake_case")]
pub async fn vault_conflict_restore(
    state: State<'_, AppState>,
    file_id: String,
    recorded_at_ms: u64,
) -> Result<()> {
    state
        .vault
        .lock()
        .await
        .restore_conflict(&file_id, recorded_at_ms)
        .map_err(Error::from)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vault_conflicts_clear(state: State<'_, AppState>) -> Result<()> {
    state
        .vault
        .lock()
        .await
        .clear_conflict_history()
        .map_err(Error::from)
}

/// Look up a secret's plaintext for internal use (SSH auth, provider keys). Never exposed via IPC.
pub async fn secret_value(state: &AppState, id: SecretId) -> Result<String> {
    state
        .vault
        .lock()
        .await
        .get::<Secret>(&id.to_string())
        .map_err(Error::from)?
        .map(|s| s.value)
        .ok_or_else(|| Error::NotFound(format!("secret {id}")))
}
