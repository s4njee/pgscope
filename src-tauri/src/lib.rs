pub mod commands;
pub mod db;
pub mod error;
pub mod repl;
pub mod secrets;
pub mod state;
pub mod store;

use tauri::Manager;

use state::AppState;
use store::Paths;

/// Build and run the app: resolve on-disk directories, install the shared
/// [`AppState`], start the connection pinger, and register every `#[tauri::command]`.
///
/// A command missing from the `generate_handler!` list below compiles fine and
/// then fails at runtime when the webview invokes it, so this list is the real
/// surface between the frontend and the backend. Panics if the app directories
/// cannot be created — there is nowhere to store profiles, so there is no app.
///
/// # Arguments
/// None.
///
/// # Returns
/// `()` — returns only when the Tauri event loop exits, i.e. once the last
/// window has closed; panics if the app directories or the generated context
/// cannot be built.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // rustls needs a process-wide crypto provider before any TLS connection.
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        // Restore window size/position across launches.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let paths = Paths::resolve().expect("could not resolve app directories");
            // Populate the saved-queries panel on first run.
            let _ = store::seed_saved_queries(&paths);

            // D7: macOS gets the native traffic lights overlaid on our 42px
            // titlebar (`titleBarStyle: Overlay` in tauri.conf.json — a
            // macOS-only key). Everywhere else that key is ignored and
            // `decorations` defaults to true, which would stack the OS's own
            // title bar on top of the one we draw. Turn decorations off so the
            // custom titlebar and its traffic lights are the only chrome.
            #[cfg(not(target_os = "macos"))]
            {
                use tauri::Manager as _;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_decorations(false);
                }
            }

            app.manage(AppState::new(paths));
            commands::spawn_pinger(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::connect_profile,
            commands::connect_dev_url,
            commands::disconnect,
            commands::ping,
            commands::connection_info,
            commands::schema_tree,
            commands::table_meta,
            commands::fk_graph,
            commands::fetch_page,
            commands::fetch_cell,
            commands::format_row,
            commands::value_predicate,
            commands::cancel_grid,
            commands::split_sql,
            commands::statement_at_cursor,
            commands::run_query,
            commands::explain_query,
            commands::cancel_query,
            commands::repl_open,
            commands::repl_exec,
            commands::repl_complete,
            commands::repl_cancel,
            commands::repl_reset,
            commands::history_list,
            commands::history_append,
            commands::saved_queries,
            commands::save_named_query,
            commands::save_query_at,
            commands::rename_saved_query,
            commands::delete_saved_query,
            commands::create_saved_folder,
            commands::saved_folders,
            commands::rename_saved_folder,
            commands::er_layout_load,
            commands::er_layout_save,
            commands::grid_layout_load,
            commands::grid_layout_save,
            commands::platform_name,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running pgscope");
}
