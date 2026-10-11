//! File browser commands for S3 accounts. Paths look like `/bucket/folder/file`; `/` lists the
//! buckets. Transfers share the SFTP browser's Downloads handling and progress events.

use crate::cmd_vault::secret_value;
use crate::sftp::{downloads, free_local_path, local_name, picked_name, reporter, FileEntry};
use crate::state::AppState;
use aht_core::model::S3Account;
use aht_core::{Error, Result};
use aht_s3::{S3Client, S3Config};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};

/// `/bucket/a/b` → `("bucket", "a/b")`; `/` → `None`.
fn split(path: &str) -> Result<Option<(String, String)>> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(None);
    }
    let (bucket, key) = trimmed.split_once('/').unwrap_or((trimmed, ""));
    if key.split('/').any(|s| s == "." || s == "..") {
        return Err(Error::Invalid(format!("unsupported S3 path {path}")));
    }
    Ok(Some((bucket.to_string(), key.to_string())))
}

fn in_bucket(path: &str) -> Result<(String, String)> {
    split(path)?.ok_or_else(|| Error::Invalid("open a bucket first".into()))
}

/// A path naming an object or folder, not a bucket.
fn object(path: &str) -> Result<(String, String)> {
    let (bucket, key) = in_bucket(path)?;
    if key.is_empty() {
        return Err(Error::Invalid(
            "buckets are created and deleted in your provider's console".into(),
        ));
    }
    Ok((bucket, key))
}

async fn account(state: &AppState, id: &str) -> Result<S3Account> {
    state
        .vault
        .lock()
        .await
        .get::<S3Account>(id)
        .map_err(Error::from)?
        .ok_or_else(|| Error::NotFound(format!("S3 account {id}")))
}

/// The account's client, built on first use.
async fn client(state: &AppState, id: &str) -> Result<Arc<S3Client>> {
    if let Some(c) = state.s3.lock().await.get(id) {
        return Ok(c.clone());
    }
    let a = account(state, id).await?;
    let secret_id = a
        .secret
        .ok_or_else(|| Error::Invalid("no secret access key saved for this account".into()))?;
    let c = Arc::new(S3Client::new(S3Config {
        endpoint: a.endpoint,
        region: a.region,
        access_key_id: a.access_key_id,
        secret_access_key: secret_value(state, secret_id).await?,
        path_style: a.path_style,
    })?);
    state.s3.lock().await.insert(id.to_string(), c.clone());
    Ok(c)
}

/// Where the browser starts: the account's bucket, or the bucket list.
#[tauri::command(rename_all = "snake_case")]
pub async fn s3_home(state: State<'_, AppState>, account_id: String) -> Result<String> {
    let a = account(&state, &account_id).await?;
    Ok(match a.bucket.as_deref().map(str::trim) {
        Some(b) if !b.is_empty() => format!("/{b}"),
        _ => "/".into(),
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn s3_list(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
) -> Result<Vec<FileEntry>> {
    let c = client(&state, &account_id).await?;
    let dir = |name: String, path: String, modified: Option<u32>| FileEntry {
        name,
        path,
        kind: "dir",
        link: false,
        size: 0,
        modified,
        permissions: None,
    };
    let Some((bucket, key)) = split(&path)? else {
        return Ok(c
            .list_buckets()
            .await?
            .into_iter()
            .map(|b| {
                let path = format!("/{}", b.name);
                dir(b.name, path, b.created)
            })
            .collect());
    };
    let prefix = if key.is_empty() {
        key
    } else {
        format!("{key}/")
    };
    let listing = c.list(&bucket, &prefix).await?;
    let mut out: Vec<FileEntry> = listing
        .folders
        .into_iter()
        .map(|name| {
            let path = format!("/{bucket}/{prefix}{name}");
            dir(name, path, None)
        })
        .collect();
    out.extend(listing.objects.into_iter().map(|o| FileEntry {
        path: format!("/{bucket}/{}", o.key),
        name: o.name,
        kind: "file",
        link: false,
        size: o.size,
        modified: o.modified,
        permissions: None,
    }));
    Ok(out)
}

/// Create a folder (an empty `name/` marker object).
#[tauri::command(rename_all = "snake_case")]
pub async fn s3_mkdir(state: State<'_, AppState>, account_id: String, path: String) -> Result<()> {
    let (bucket, key) = object(&path)?;
    let c = client(&state, &account_id).await?;
    c.create_folder(&bucket, &format!("{key}/")).await
}

/// Rename a file: a server-side copy, then a delete. Folders are only key prefixes in S3.
#[tauri::command(rename_all = "snake_case")]
pub async fn s3_rename(
    state: State<'_, AppState>,
    account_id: String,
    from: String,
    to: String,
    is_dir: bool,
) -> Result<()> {
    if is_dir {
        return Err(Error::Invalid(
            "S3 folders cannot be renamed: every object inside would have to be copied".into(),
        ));
    }
    let ((bucket, from_key), (to_bucket, to_key)) = (object(&from)?, object(&to)?);
    if bucket != to_bucket {
        return Err(Error::Invalid("a rename stays in the same bucket".into()));
    }
    let c = client(&state, &account_id).await?;
    c.copy(&bucket, &from_key, &to_key).await?;
    c.delete(&bucket, &from_key).await
}

/// Delete a file, or an empty folder.
#[tauri::command(rename_all = "snake_case")]
pub async fn s3_remove(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    is_dir: bool,
) -> Result<()> {
    let (bucket, key) = object(&path)?;
    let c = client(&state, &account_id).await?;
    if is_dir {
        c.delete_folder(&bucket, &format!("{key}/")).await
    } else {
        c.delete(&bucket, &key).await
    }
}

/// Download an object into the Downloads folder; returns the local path.
#[tauri::command(rename_all = "snake_case")]
pub async fn s3_download(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    transfer_id: String,
) -> Result<String> {
    let (bucket, key) = object(&path)?;
    let c = client(&state, &account_id).await?;
    let local = free_local_path(&downloads(&app)?, &local_name(&path)?);
    c.download(&bucket, &key, &local, reporter(&app, transfer_id))
        .await?;
    Ok(local.display().to_string())
}

/// Upload a local file (picked in the file dialog) into the folder `dir`; returns its path.
#[tauri::command(rename_all = "snake_case")]
pub async fn s3_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    local_path: PathBuf,
    dir: String,
    transfer_id: String,
) -> Result<String> {
    let (bucket, prefix) = in_bucket(&dir)?;
    let name = picked_name(&local_path)?;
    let key = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    let c = client(&state, &account_id).await?;
    c.upload(&bucket, &key, &local_path, reporter(&app, transfer_id))
        .await?;
    Ok(format!("/{bucket}/{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paths() {
        assert_eq!(split("/").unwrap(), None);
        assert_eq!(split("/b").unwrap(), Some(("b".into(), "".into())));
        assert_eq!(
            split("/b/logs/a.txt").unwrap(),
            Some(("b".into(), "logs/a.txt".into()))
        );
        assert!(split("/b/../etc").is_err());
        assert!(object("/b").is_err());
        assert!(in_bucket("/").is_err());
    }
}
