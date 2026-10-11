//! Copying between the file browsers' remote file systems: server to server over SFTP, an S3
//! object to a server, and a server's file to S3. The bytes stream through this computer
//! without touching its disk.

use crate::sftp::{client as sftp, reporter};
use crate::state::AppState;
use aht_core::{Error, Result, SessionId};
use aht_ssh::sftp::{join, EXISTS};
use serde::Deserialize;
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

/// One end of a copy.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    /// An SSH session's files, as root when `root` (through sudo, see `sftp_sudo`).
    Sftp {
        session_id: SessionId,
        #[serde(default)]
        root: bool,
    },
    /// An S3 account; paths are `/bucket/key`.
    S3 { account_id: String },
}

/// The last part of `path`: the name the copy gets.
fn name_of(path: &str) -> Result<String> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid(format!("cannot copy {path}")))
}

/// Copy `path` (a file; between servers a folder too) from `from` into the folder `to_dir` of
/// `to`. Resolves with the new path. Rejects with `busy`/`exists` when the name is taken,
/// unless `overwrite` (files are replaced, folders merged).
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn copy_between(
    app: AppHandle,
    state: State<'_, AppState>,
    from: Place,
    path: String,
    to: Place,
    to_dir: String,
    overwrite: bool,
    transfer_id: String,
) -> Result<String> {
    let name = name_of(&path)?;
    let progress = reporter(&app, transfer_id);
    match (from, to) {
        (
            Place::Sftp {
                session_id: from_id,
                root: from_root,
            },
            Place::Sftp { session_id, root },
        ) => {
            let dest = join(&to_dir, &name);
            if from_id == session_id && from_root == root && dest == path {
                return Err(Error::Invalid(format!("{name} is already in {to_dir}")));
            }
            let src = sftp(&state, from_id, Some(from_root)).await?;
            let dst = sftp(&state, session_id, Some(root)).await?;
            src.copy_to(&path, &dst, &dest, overwrite, progress).await?;
            Ok(dest)
        }
        (Place::S3 { account_id }, Place::Sftp { session_id, root }) => {
            let (bucket, key) = crate::s3::object(&path)?;
            let dst = sftp(&state, session_id, Some(root)).await?;
            let dest = join(&to_dir, &name);
            if !overwrite && dst.exists(&dest).await {
                return Err(Error::Busy(EXISTS.into()));
            }
            let s3 = crate::s3::client(&state, &account_id).await?;
            let mut file = dst.create(&dest).await?;
            let copied = async {
                s3.download_to(&bucket, &key, &mut file, progress).await?;
                file.shutdown()
                    .await
                    .map_err(|e| Error::Other(format!("SFTP: {e}")))
            }
            .await;
            if let Err(e) = copied {
                // No half-copied file left behind.
                let _ = dst.remove(&dest, false).await;
                return Err(e);
            }
            Ok(dest)
        }
        (Place::Sftp { session_id, root }, Place::S3 { account_id }) => {
            let (bucket, prefix) = crate::s3::in_bucket(&to_dir)?;
            let prefix = prefix.trim_end_matches('/');
            let key = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let s3 = crate::s3::client(&state, &account_id).await?;
            if !overwrite && s3.exists(&bucket, &key).await? {
                return Err(Error::Busy(EXISTS.into()));
            }
            let src = sftp(&state, session_id, Some(root)).await?;
            let (file, size) = src.open_read(&path).await?;
            s3.upload_from(&bucket, &key, file, size, progress).await?;
            Ok(format!("/{bucket}/{key}"))
        }
        (Place::S3 { .. }, Place::S3 { .. }) => Err(Error::Invalid(
            "copying between S3 accounts is not supported: download the file and upload it".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_places() {
        assert_eq!(name_of("/srv/app/").unwrap(), "app");
        assert_eq!(name_of("/bucket/logs/a.gz").unwrap(), "a.gz");
        assert!(name_of("/").is_err());
        assert!(name_of("/srv/..").is_err());
        let p: Place = serde_json::from_value(serde_json::json!({
            "kind": "sftp", "session_id": SessionId::new()
        }))
        .unwrap();
        assert!(matches!(p, Place::Sftp { root: false, .. }));
        let p: Place =
            serde_json::from_value(serde_json::json!({ "kind": "s3", "account_id": "a" })).unwrap();
        assert!(matches!(p, Place::S3 { account_id } if account_id == "a"));
    }
}
