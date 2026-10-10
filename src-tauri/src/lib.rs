mod cmd_misc;
mod cmd_vault;
mod state;
mod terminal;

use tauri::Manager;

#[tauri::command(rename_all = "snake_case")]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // AIHUBTERM_DATA_DIR isolates test runs from the user's real vault.
            let dir = match std::env::var_os("AIHUBTERM_DATA_DIR") {
                Some(d) => std::path::PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            let state = state::AppState::new(dir).map_err(|e| e.to_string())?;
            state.terminal.attach(app.handle().clone());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            cmd_vault::vault_status,
            cmd_vault::vault_create,
            cmd_vault::vault_unlock,
            cmd_vault::vault_lock,
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
        .expect("error while running AIHubTerm");
}
