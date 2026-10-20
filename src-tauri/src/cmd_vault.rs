//! Vault lifecycle and stored records (connections, groups, secrets, identities, providers,
//! profiles).

use crate::state::{err, AppState};
use aht_core::model::{
    Connection, Group, HarnessProfile, Identity, Provider, S3Account, Secret, SecretKind, Snippet,
    SshAuth,
};
use aht_core::{Error, Result, SecretId};
use aht_vault::{
    ConflictRecord, KdfParams, SecretKey, Setting, SnapshotInfo, Vault, VaultItem, VaultStatus,
};
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

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

/// Write the Emergency Kit to the Downloads folder (home if there is none) and return its path.
/// The webview cannot save `blob:` downloads itself. Never overwrites: "… (2).txt" and so on.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_emergency_kit(app: AppHandle, contents: String) -> Result<String> {
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(err)?;
    let path = tokio::task::spawn_blocking(move || {
        save_new_file(&dir, "Handrail Emergency Kit", "txt", contents.as_bytes())
    })
    .await
    .map_err(err)??;
    Ok(path.display().to_string())
}

/// Create `<dir>/<stem>.<ext>`, or `<stem> (2).<ext>`... if taken. Owner-only on Unix: the file
/// holds the Secret Key.
fn save_new_file(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for n in 1..=100 {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let path = dir.join(name);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&path) {
            Ok(mut f) => {
                f.write_all(bytes)?;
                f.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(format!(
        "too many files named {stem} in {}",
        dir.display()
    )))
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
    state.s3.lock().await.clear();
    *state.pending_github_token.lock().await = None;
    Ok(())
}

/// Delete the vault from this device, locked or not (a forgotten master password is the usual
/// reason): every item, snapshot and the Secret Key in the keychain, plus what only made sense
/// with it: open sessions, the sync settings and the AI profiles' folders. Copies synced to
/// GitHub or a cloud folder stay; known_hosts stays too (it belongs to the device).
#[tauri::command(rename_all = "snake_case")]
pub async fn vault_delete(state: State<'_, AppState>) -> Result<()> {
    // No sync round may run against the vault while it goes.
    let _sync = state.sync_lock.lock().await;
    // Locked first: an SSH session (or reconnect) can no longer read its credentials.
    vault_lock(state.clone()).await?;
    state.terminal.close_all().await;
    state.rdp.close_all().await;
    let vault = state.vault.clone();
    let mut guard = vault.lock_owned().await;
    let vault_id = guard.vault_id().map(str::to_owned);
    tokio::task::spawn_blocking(move || guard.erase())
        .await
        .map_err(err)?
        .map_err(Error::from)?;
    // Anything that registered while the first pass ran (a local shell, an open in flight).
    state.terminal.close_all().await;
    if let Some(id) = vault_id {
        if let Err(e) = state.keystore.delete(&id) {
            tracing::warn!("could not remove the secret key from the keychain: {e}");
        }
    }
    match std::fs::remove_dir_all(&state.paths.profiles_dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!("could not remove the AI profiles' folders: {e}")
        }
        _ => {}
    }
    match std::fs::remove_file(&state.paths.sync_state_file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
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

#[tauri::command(rename_all = "snake_case")]
pub async fn save_secret(state: State<'_, AppState>, secret: Secret) -> Result<SecretId> {
    put(&state, &secret).await?;
    Ok(secret.id)
}

/// A saved secret without its value (the value never goes to the UI).
#[derive(Serialize)]
pub struct SecretInfo {
    id: SecretId,
    kind: SecretKind,
    label: String,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_secrets(state: State<'_, AppState>) -> Result<Vec<SecretInfo>> {
    Ok(list::<Secret>(&state)
        .await?
        .into_iter()
        .map(|s| SecretInfo {
            id: s.id,
            kind: s.kind,
            label: s.label,
        })
        .collect())
}

/// Rename a secret, or replace its value for every connection that uses it.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_secret(
    state: State<'_, AppState>,
    id: SecretId,
    label: Option<String>,
    value: Option<String>,
) -> Result<()> {
    let mut vault = state.vault.lock().await;
    let mut secret = vault
        .get::<Secret>(&id.to_string())
        .map_err(Error::from)?
        .ok_or_else(|| Error::NotFound(format!("secret {id}")))?;
    if let Some(l) = label {
        let l = l.trim();
        if l.is_empty() {
            return Err(Error::Invalid("a saved secret needs a name".into()));
        }
        secret.label = l.to_string();
    }
    if let Some(v) = value {
        if v.is_empty() {
            return Err(Error::Invalid("the new value is empty".into()));
        }
        secret.value = v;
    }
    vault.put(&secret).map_err(Error::from)?;
    // S3 clients hold their decrypted key: the next use builds a new one.
    state.s3.lock().await.clear();
    Ok(())
}

fn auth_uses(auth: &SshAuth, id: SecretId) -> bool {
    match auth {
        SshAuth::Password { secret } => *secret == id,
        SshAuth::Key { key, passphrase } => *key == id || *passphrase == Some(id),
        SshAuth::Agent | SshAuth::Prompt => false,
    }
}

/// What uses secret `id`: names of connections, identities, S3 accounts and AI providers. A
/// connection that logs in with an identity counts through the identity, not its own `auth`.
fn secret_users(
    id: SecretId,
    connections: &[Connection],
    identities: &[Identity],
    s3: &[S3Account],
    providers: &[Provider],
) -> Vec<String> {
    connections
        .iter()
        .filter(|c| c.identity.is_none() && auth_uses(&c.auth, id))
        .map(|c| c.name.clone())
        .chain(
            identities
                .iter()
                .filter(|i| auth_uses(&i.auth, id))
                .map(|i| i.label.clone()),
        )
        .chain(
            s3.iter()
                .filter(|a| a.secret == Some(id))
                .map(|a| a.name.clone()),
        )
        .chain(
            providers
                .iter()
                .filter(|p| p.api_key == Some(id))
                .map(|p| p.name.clone()),
        )
        .collect()
}

/// Delete a saved secret nothing uses any more.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_secret(state: State<'_, AppState>, id: SecretId) -> Result<()> {
    let mut vault = state.vault.lock().await;
    let users = secret_users(
        id,
        &vault.list::<Connection>().map_err(Error::from)?,
        &vault.list::<Identity>().map_err(Error::from)?,
        &vault.list::<S3Account>().map_err(Error::from)?,
        &vault.list::<Provider>().map_err(Error::from)?,
    );
    if !users.is_empty() {
        return Err(Error::Invalid(format!(
            "still used by {}",
            users.join(", ")
        )));
    }
    vault
        .delete::<Secret>(&id.to_string())
        .map(|_| ())
        .map_err(Error::from)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_snippets(state: State<'_, AppState>) -> Result<Vec<Snippet>> {
    list(&state).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_snippet(state: State<'_, AppState>, snippet: Snippet) -> Result<Snippet> {
    if snippet.name.trim().is_empty() || snippet.command.trim().is_empty() {
        return Err(Error::Invalid(
            "a snippet needs a name and a command".into(),
        ));
    }
    put(&state, &snippet).await?;
    Ok(snippet)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_snippet(state: State<'_, AppState>, id: String) -> Result<()> {
    delete::<Snippet>(&state, &id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_s3_accounts(state: State<'_, AppState>) -> Result<Vec<S3Account>> {
    list(&state).await
}

/// Save an S3 account; `secret_key`, when given, replaces the stored secret access key.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_s3_account(
    state: State<'_, AppState>,
    mut account: S3Account,
    secret_key: Option<String>,
) -> Result<S3Account> {
    if account.name.trim().is_empty() || account.access_key_id.trim().is_empty() {
        return Err(Error::Invalid(
            "an S3 account needs a name and an access key ID".into(),
        ));
    }
    if let Some(value) = secret_key.filter(|k| !k.trim().is_empty()) {
        let id = account.secret.unwrap_or_default();
        let secret = Secret {
            id,
            kind: SecretKind::ApiKey,
            label: format!("{} S3 secret key", account.name),
            value: value.trim().to_string(),
        };
        put(&state, &secret).await?;
        account.secret = Some(id);
    }
    if account.secret.is_none() {
        return Err(Error::Invalid(
            "an S3 account needs a secret access key".into(),
        ));
    }
    put(&state, &account).await?;
    Ok(account)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_identities(state: State<'_, AppState>) -> Result<Vec<Identity>> {
    list(&state).await
}

/// Save an identity. Its user name, password or key apply from the next login of every
/// connection that uses it.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_identity(state: State<'_, AppState>, mut identity: Identity) -> Result<Identity> {
    identity.label = identity.label.trim().to_string();
    identity.user = identity.user.trim().to_string();
    if identity.label.is_empty() {
        return Err(Error::Invalid("an identity needs a name".into()));
    }
    if identity.user.is_empty() {
        return Err(Error::Invalid("an identity needs a user name".into()));
    }
    let mut vault = state.vault.lock().await;
    let exists = |id: SecretId| -> Result<bool> {
        Ok(vault
            .get::<Secret>(&id.to_string())
            .map_err(Error::from)?
            .is_some())
    };
    let ok = match &identity.auth {
        SshAuth::Password { secret } => exists(*secret)?,
        SshAuth::Key { key, passphrase } => exists(*key)? && passphrase.map_or(Ok(true), exists)?,
        SshAuth::Agent => true,
        SshAuth::Prompt => {
            return Err(Error::Invalid(
                "an identity needs a password, a key or the SSH agent".into(),
            ))
        }
    };
    if !ok {
        return Err(Error::NotFound(
            "the identity's password or key is no longer in the vault".into(),
        ));
    }
    vault.put(&identity).map_err(Error::from)?;
    Ok(identity)
}

/// Delete an identity no connection uses. Its password or key stay in the vault (they may be
/// shared); unused ones can be cleaned up in Settings.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_identity(state: State<'_, AppState>, id: String) -> Result<()> {
    let mut vault = state.vault.lock().await;
    let users: Vec<String> = vault
        .list::<Connection>()
        .map_err(Error::from)?
        .into_iter()
        .filter(|c| c.identity.is_some_and(|i| i.to_string() == id))
        .map(|c| c.name)
        .collect();
    if !users.is_empty() {
        return Err(Error::Invalid(format!(
            "still used by {}",
            users.join(", ")
        )));
    }
    vault
        .delete::<Identity>(&id)
        .map(|_| ())
        .map_err(Error::from)
}

/// For a connection that logs in with an identity, put the identity's user and
/// authentication in place of its own. A missing identity (deleted, or not synced to this
/// device yet) is an error rather than a silent fallback to the connection's old login.
pub(crate) fn apply_identity(vault: &Vault, c: &mut Connection) -> Result<()> {
    let Some(id) = c.identity else {
        return Ok(());
    };
    let identity = vault
        .get::<Identity>(&id.to_string())
        .map_err(Error::from)?
        .ok_or_else(|| {
            Error::NotFound(format!(
                "the identity {} logs in with is no longer in the vault: choose another in its \
                 settings",
                c.name
            ))
        })?;
    c.user = identity.user;
    c.auth = identity.auth;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_s3_account(state: State<'_, AppState>, id: String) -> Result<()> {
    // Drops the client and the secret it holds.
    state.s3.lock().await.remove(&id);
    let mut vault = state.vault.lock().await;
    if let Some(a) = vault.get::<S3Account>(&id).map_err(Error::from)? {
        if let Some(key) = a.secret {
            vault
                .delete::<Secret>(&key.to_string())
                .map_err(Error::from)?;
        }
    }
    vault
        .delete::<S3Account>(&id)
        .map(|_| ())
        .map_err(Error::from)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_users_finds_every_reference() {
        let key = SecretId::new();
        let other = SecretId::new();
        let conn = |name: &str, auth: SshAuth| {
            let mut c: Connection = serde_json::from_value(serde_json::json!({
                "id": aht_core::ConnectionId::new(), "name": name, "kind": "ssh", "group_id": null,
                "host": "h", "port": 22, "user": "u", "auth": {"type": "agent"},
                "color": null, "approval_mode": null, "local_shell": null
            }))
            .unwrap();
            c.auth = auth;
            c
        };
        let connections = vec![
            conn(
                "web-01",
                SshAuth::Key {
                    key,
                    passphrase: None,
                },
            ),
            conn(
                "web-02",
                SshAuth::Key {
                    key: other,
                    passphrase: Some(key),
                },
            ),
            conn("db", SshAuth::Password { secret: other }),
            conn("pi", SshAuth::Agent),
        ];
        assert_eq!(
            secret_users(key, &connections, &[], &[], &[]),
            ["web-01", "web-02"]
        );
        assert_eq!(
            secret_users(SecretId::new(), &connections, &[], &[], &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn secret_users_counts_identities_not_the_connections_on_them() {
        let key = SecretId::new();
        let identity = Identity {
            id: aht_core::IdentityId::new(),
            label: "ops".into(),
            user: "ops".into(),
            auth: SshAuth::Key {
                key,
                passphrase: None,
            },
        };
        let mut on_identity: Connection = serde_json::from_value(serde_json::json!({
            "id": aht_core::ConnectionId::new(), "name": "web-03", "kind": "ssh", "group_id": null,
            "host": "h", "port": 22, "user": "u", "auth": {"type": "password", "secret": key},
            "color": null, "approval_mode": null, "local_shell": null
        }))
        .unwrap();
        on_identity.identity = Some(identity.id);
        assert_eq!(
            secret_users(key, &[on_identity], &[identity], &[], &[]),
            ["ops"]
        );
    }

    #[test]
    fn save_new_file_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("aht-kit-{}", uuid::Uuid::new_v4()));
        let a = save_new_file(&dir, "Kit", "txt", b"one").unwrap();
        let b = save_new_file(&dir, "Kit", "txt", b"two").unwrap();
        assert_eq!(a, dir.join("Kit.txt"));
        assert_eq!(b, dir.join("Kit (2).txt"));
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(std::fs::read(&b).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
