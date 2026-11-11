//! SQL backups opened for reading, with no server: the user picks a `.sql` (or `.sql.gz`)
//! file, it is indexed once ([`aht_db::backup`]), then the page asks for pages of a table's
//! rows or saves a table. The page holds a token, never the file's path.

use crate::db::{transfer_reporter, Running};
use crate::state::AppState;
use aht_core::{Error, Result};
use aht_db::backup::{self, BackupIndex, BackupPage, Format, Source};
use aht_db::transfer::Progress;
use aht_db::DbKind;
use serde::Serialize;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::oneshot;

/// A backup open for reading.
pub struct OpenBackup {
    path: PathBuf,
    /// gzip-compressed: read from the start to reach an offset.
    gz: bool,
    index: BackupIndex,
}

struct FileSource<'a> {
    path: &'a PathBuf,
    gz: bool,
}

impl Source for FileSource<'_> {
    fn open_at(&mut self, offset: u64) -> io::Result<Box<dyn BufRead + '_>> {
        let mut f = File::open(self.path)?;
        if self.gz {
            let mut r = BufReader::with_capacity(
                1 << 20,
                flate2::read::MultiGzDecoder::new(BufReader::new(f)),
            );
            io::copy(&mut (&mut r).take(offset), &mut io::sink())?;
            return Ok(Box::new(r));
        }
        f.seek(SeekFrom::Start(offset))?;
        Ok(Box::new(BufReader::with_capacity(1 << 20, f)))
    }
}

fn is_gzip(path: &PathBuf) -> io::Result<bool> {
    let mut magic = [0u8; 2];
    let n = File::open(path)?.read(&mut magic)?;
    Ok(n == 2 && magic == [0x1f, 0x8b])
}

fn backup(state: &AppState, token: &str) -> Result<Arc<OpenBackup>> {
    state
        .db
        .backups
        .lock()
        .unwrap()
        .get(token)
        .cloned()
        .ok_or_else(|| Error::NotFound("the backup (open it again)".into()))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(e.to_string()))?
}

#[derive(Serialize)]
pub struct BackupOpened {
    pub token: String,
    pub name: String,
    pub size: u64,
    pub compressed: bool,
    pub index: BackupIndex,
}

fn parse_kind(kind: Option<&str>) -> Result<Option<DbKind>> {
    match kind {
        None | Some("") | Some("auto") => Ok(None),
        Some("mysql") => Ok(Some(DbKind::Mysql)),
        Some("postgres") => Ok(Some(DbKind::Postgres)),
        Some(other) => Err(Error::Invalid(format!("unknown dialect {other}"))),
    }
}

/// Index `path` (in a blocking thread, stoppable with `db_transfer_cancel`).
async fn index_file(
    app: &AppHandle,
    state: &AppState,
    path: PathBuf,
    gz: bool,
    kind: Option<DbKind>,
    transfer_id: &str,
) -> Result<BackupIndex> {
    let running = Running::new(&state.db, "backup", transfer_id);
    let stop = running.stop.clone();
    let mut report = transfer_reporter(app, transfer_id);
    let index = blocking(move || {
        let mut src = FileSource { path: &path, gz };
        let mut input = src.open_at(0)?;
        backup::index(
            &mut input,
            kind,
            &mut |bytes| {
                report(&Progress {
                    bytes,
                    ..Progress::default()
                })
            },
            &stop,
        )
    })
    .await;
    drop(running);
    index
}

/// Pick an SQL backup and index it (progress: `db://transfer` with the bytes read). `kind`:
/// "mysql" or "postgres" when the user says so; otherwise the file's first lines decide.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_backup_open(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: Option<String>,
    transfer_id: String,
) -> Result<Option<BackupOpened>> {
    use tauri_plugin_dialog::DialogExt;
    let kind = parse_kind(kind.as_deref())?;
    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .add_filter("SQL", &["sql", "gz", "txt", "dump"])
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| Error::Other("the file dialog closed".into()))?
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| Error::Other(e.to_string()))?;
    let name = crate::sftp::picked_name(&path)?;
    let size = std::fs::metadata(&path)?.len();
    let gz = is_gzip(&path)?;
    let mut magic = [0u8; 5];
    let n = File::open(&path)?.read(&mut magic)?;
    if n == 5 && &magic == b"PGDMP" {
        return Err(Error::Invalid(
            "this is a pg_dump archive (custom format), not SQL: turn it into SQL with \
             `pg_restore -f backup.sql` and open that"
                .into(),
        ));
    }
    let index = index_file(&app, &state, path.clone(), gz, kind, &transfer_id).await?;
    let token = uuid::Uuid::new_v4().to_string();
    state.db.backups.lock().unwrap().insert(
        token.clone(),
        Arc::new(OpenBackup {
            path,
            gz,
            index: index.clone(),
        }),
    );
    Ok(Some(BackupOpened {
        token,
        name,
        size,
        compressed: gz,
        index,
    }))
}

/// Read an open backup again as the other dialect (the file's first lines guessed wrong).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_backup_reindex(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    kind: String,
    transfer_id: String,
) -> Result<BackupIndex> {
    let b = backup(&state, &token)?;
    let kind = parse_kind(Some(&kind))?;
    let index = index_file(&app, &state, b.path.clone(), b.gz, kind, &transfer_id).await?;
    state.db.backups.lock().unwrap().insert(
        token,
        Arc::new(OpenBackup {
            path: b.path.clone(),
            gz: b.gz,
            index: index.clone(),
        }),
    );
    Ok(index)
}

/// A page of a table's rows (with `filter`: only rows holding that text, in `column` or any).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_backup_rows(
    state: State<'_, AppState>,
    token: String,
    table: usize,
    from: u64,
    limit: Option<usize>,
    filter: Option<String>,
    column: Option<String>,
) -> Result<BackupPage> {
    let b = backup(&state, &token)?;
    let limit = limit.unwrap_or(500).clamp(1, 10_000);
    blocking(move || {
        let stop = AtomicBool::new(false);
        let mut src = FileSource {
            path: &b.path,
            gz: b.gz,
        };
        backup::rows(
            &mut src,
            &b.index,
            table,
            from,
            limit,
            filter.as_deref(),
            column.as_deref(),
            &stop,
        )
    })
    .await
}

#[derive(Serialize)]
pub struct BackupSaved {
    pub path: String,
    pub rows: u64,
}

/// Save a table of the backup (all its rows, or the ones holding `filter`, in `column` or any)
/// as CSV, JSON or SQL, to a file picked in a save dialog; `None` when the dialog is cancelled.
/// A stopped or failed save leaves no file.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn db_backup_export(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    table: usize,
    format: Format,
    filter: Option<String>,
    column: Option<String>,
    transfer_id: String,
) -> Result<Option<BackupSaved>> {
    use tauri_plugin_dialog::DialogExt;
    let b = backup(&state, &token)?;
    let t = b
        .index
        .tables
        .get(table)
        .ok_or_else(|| Error::NotFound("table of the backup".into()))?;
    let ext = match format {
        Format::Csv => "csv",
        Format::Json => "json",
        Format::Sql => "sql",
    };
    let stem: String = t
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("{stem}.{ext}"))
        .add_filter(ext.to_uppercase(), &[ext])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| Error::Other("the save dialog closed".into()))?
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| Error::Other(e.to_string()))?;
    let same = |a: &PathBuf, b: &PathBuf| match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    };
    if same(&path, &b.path) {
        return Err(Error::Invalid(
            "pick another file: this one is the backup being read".into(),
        ));
    }
    // Written next to it, then renamed over it: a destination that is a link to the backup
    // replaces the link, never truncates the backup being read.
    let part = path.with_file_name(format!(
        ".{}.part",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export")
    ));
    let running = Running::new(&state.db, "backup", &transfer_id);
    let stop = running.stop.clone();
    let mut report = transfer_reporter(&app, &transfer_id);
    let out_path = part.clone();
    let done = blocking(move || {
        let mut out = std::io::BufWriter::with_capacity(1 << 20, File::create(&out_path)?);
        let mut src = FileSource {
            path: &b.path,
            gz: b.gz,
        };
        backup::export(
            &mut src,
            &b.index,
            table,
            format,
            filter.as_deref(),
            column.as_deref(),
            &mut out,
            &mut |rows| {
                report(&Progress {
                    rows,
                    ..Progress::default()
                })
            },
            &stop,
        )
    })
    .await;
    drop(running);
    match done.and_then(|rows| {
        std::fs::rename(&part, &path)?;
        Ok(rows)
    }) {
        Ok(rows) => Ok(Some(BackupSaved {
            path: path.display().to_string(),
            rows,
        })),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Forget an open backup (its pane closed).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_backup_close(state: State<'_, AppState>, token: String) -> Result<()> {
    state.db.backups.lock().unwrap().remove(&token);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_plain_and_gzip_files_from_an_offset() {
        let dir = std::env::temp_dir().join(format!("aht-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = b"SET NAMES utf8mb4;\nINSERT INTO t VALUES (1);\n";
        let plain = dir.join("b.sql");
        std::fs::write(&plain, text).unwrap();
        let gz = dir.join("b.sql.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(text).unwrap();
        std::fs::write(&gz, enc.finish().unwrap()).unwrap();
        for (path, compressed) in [(&plain, false), (&gz, true)] {
            assert_eq!(is_gzip(path).unwrap(), compressed);
            let mut src = FileSource {
                path,
                gz: compressed,
            };
            let mut rest = String::new();
            src.open_at(19).unwrap().read_to_string(&mut rest).unwrap();
            assert_eq!(rest, "INSERT INTO t VALUES (1);\n");
            let stop = AtomicBool::new(false);
            let mut input = src.open_at(0).unwrap();
            let ix = backup::index(&mut input, None, &mut |_| {}, &stop).unwrap();
            assert_eq!(ix.tables.len(), 1);
            assert_eq!(ix.tables[0].rows, 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
