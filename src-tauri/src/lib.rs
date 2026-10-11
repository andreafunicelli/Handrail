mod autosync;
mod cmd_misc;
mod cmd_vault;
mod s3;
mod sftp;
mod state;
mod terminal;

use tauri::Manager;

/// The single source of the version is the workspace `Cargo.toml` (`[workspace.package]`);
/// release builds stamp it to `<major>.<minor>.<build number>`.
#[tauri::command(rename_all = "snake_case")]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Relaunch after the updater installed a new version (macOS, Linux; the Windows installer
/// closes and restarts the app itself).
#[tauri::command(rename_all = "snake_case")]
fn restart_app(app: tauri::AppHandle) {
    app.restart()
}

/// Open an `https` link in the default browser. The webview ignores `target="_blank"` links, so
/// the UI sends external links here. Only plain https URLs are accepted; the URL goes to the OS
/// opener as a single argument, never through a shell.
#[tauri::command(rename_all = "snake_case")]
fn open_url(url: String) -> aht_core::Result<()> {
    let url = external_url(&url).map_err(aht_core::Error::Invalid)?;
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(windows)]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    let mut child = cmd
        .arg(url.as_str())
        .spawn()
        .map_err(|e| aht_core::Error::Other(format!("could not open the browser: {e}")))?;
    // Reap the opener (explorer.exe exits non-zero even on success, so its status is ignored).
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn external_url(s: &str) -> Result<tauri::Url, String> {
    let url = tauri::Url::parse(s.trim()).map_err(|e| format!("not a valid URL: {e}"))?;
    if url.scheme() != "https" {
        return Err("only https links can be opened".into());
    }
    if url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("unsupported link".into());
    }
    Ok(url)
}

/// App data directory of builds released as "AIHubTerm" (bundle identifier `dev.aihubterm.app`).
const LEGACY_DATA_DIR_NAME: &str = "dev.aihubterm.app";

/// Moves the vault of an AIHubTerm install to Handrail's data directory, once: only when the
/// new directory does not exist yet. Both live in the same per-user parent directory.
fn migrate_legacy_data_dir(dir: &std::path::Path) {
    let Some(legacy) = dir.parent().map(|p| p.join(LEGACY_DATA_DIR_NAME)) else {
        return;
    };
    if dir.exists() || !legacy.is_dir() {
        return;
    }
    match std::fs::rename(&legacy, dir) {
        Ok(()) => {
            tracing::info!(from = %legacy.display(), to = %dir.display(), "moved data directory")
        }
        Err(e) => tracing::warn!(error = %e, "could not move the AIHubTerm data directory"),
    }
}

pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .setup(|app| {
            // HANDRAIL_DATA_DIR isolates test runs from the user's real vault.
            let dir = match std::env::var_os("HANDRAIL_DATA_DIR") {
                Some(d) => std::path::PathBuf::from(d),
                None => {
                    let dir = app.path().app_data_dir()?;
                    migrate_legacy_data_dir(&dir);
                    dir
                }
            };
            let state = state::AppState::new(dir).map_err(|e| e.to_string())?;
            state.terminal.attach(app.handle().clone());
            app.manage(state);
            autosync::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            restart_app,
            open_url,
            cmd_vault::vault_status,
            cmd_vault::vault_create,
            cmd_vault::vault_unlock,
            cmd_vault::vault_lock,
            cmd_vault::save_emergency_kit,
            cmd_vault::list_connections,
            cmd_vault::save_connection,
            cmd_vault::delete_connection,
            cmd_vault::list_groups,
            cmd_vault::save_group,
            cmd_vault::delete_group,
            cmd_vault::save_secret,
            cmd_vault::list_snippets,
            cmd_vault::save_snippet,
            cmd_vault::delete_snippet,
            cmd_vault::list_s3_accounts,
            cmd_vault::save_s3_account,
            cmd_vault::delete_s3_account,
            cmd_vault::list_providers,
            cmd_vault::save_provider,
            cmd_vault::delete_provider,
            cmd_vault::list_harness_profiles,
            cmd_vault::save_harness_profile,
            cmd_vault::delete_harness_profile,
            cmd_vault::get_setting,
            cmd_vault::set_setting,
            cmd_vault::vault_snapshots,
            cmd_vault::vault_snapshot_create,
            cmd_vault::vault_snapshot_restore,
            cmd_vault::vault_conflicts,
            cmd_vault::vault_conflict_restore,
            cmd_vault::vault_conflicts_clear,
            cmd_misc::import_detect,
            cmd_misc::import_preview,
            cmd_misc::import_apply,
            cmd_misc::detect_harnesses,
            cmd_misc::list_harness_extensions,
            cmd_misc::list_models,
            cmd_misc::sync_status,
            cmd_misc::sync_configure_github_start,
            cmd_misc::sync_configure_github_poll,
            cmd_misc::sync_configure_folder,
            cmd_misc::sync_join,
            cmd_misc::sync_now,
            cmd_misc::sync_set_auto,
            cmd_misc::sync_disconnect,
            cmd_misc::sync_folder_presets,
            terminal::approval_decide,
            terminal::host_key_decision,
            terminal::auth_prompt_answer,
            sftp::sftp_home,
            sftp::sftp_list,
            sftp::sftp_mkdir,
            sftp::sftp_rename,
            sftp::sftp_remove,
            sftp::sftp_download,
            sftp::sftp_upload,
            sftp::sftp_sudo,
            sftp::sftp_root_off,
            sftp::sftp_read_text,
            sftp::sftp_write_text,
            terminal::edit_proposal_decide,
            s3::s3_home,
            s3::s3_list,
            s3::s3_mkdir,
            s3::s3_rename,
            s3::s3_remove,
            s3::s3_download,
            s3::s3_upload,
            terminal::open_session,
            terminal::open_local_session,
            terminal::session_write,
            terminal::session_resize,
            terminal::close_session,
            terminal::list_sessions,
            terminal::set_approval_mode,
            terminal::agent_send,
            terminal::agent_cancel,
            terminal::agent_reset,
            terminal::harness_open_native,
            terminal::harness_login,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Handrail");
}

#[cfg(test)]
mod tests {
    use super::external_url;

    #[test]
    fn only_plain_https_links_open() {
        assert_eq!(
            external_url(" https://github.com/login/device ")
                .unwrap()
                .as_str(),
            "https://github.com/login/device"
        );
        for bad in [
            "http://github.com/login/device",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://user:pw@github.com/",
            "-a Calculator",
            "",
        ] {
            assert!(external_url(bad).is_err(), "{bad}");
        }
    }
}
