mod autosync;
mod cmd_misc;
mod cmd_vault;
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
    let builder = tauri::Builder::default();
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
            cmd_vault::list_secrets,
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
            terminal::open_session,
            terminal::list_local_shells,
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
