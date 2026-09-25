pub mod commands;
pub mod common;
pub mod config;
pub mod constants;
pub mod extract_new;
pub mod gametype;
pub mod helper;
pub mod ini_container;
pub mod plugin;
pub mod utils;
pub mod workspace;
pub mod workspace_access;

use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const SHOW_WINDOW_SHORTCUT_LABEL: &str = "Alt+F";
const APP_SETTINGS_RELATIVE_PATH: &str = "SSMT4GlobalConfigs/settings.json";

fn show_window_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT), Code::KeyF)
}

fn reveal_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("[GlobalShortcut] Main window is unavailable");
        return;
    };

    if let Err(error) = window.show() {
        eprintln!("[GlobalShortcut] Failed to show main window: {error}");
    }
    if let Err(error) = window.unminimize() {
        eprintln!("[GlobalShortcut] Failed to restore main window: {error}");
    }
    if let Err(error) = window.set_focus() {
        eprintln!("[GlobalShortcut] Failed to focus main window: {error}");
    }
}

fn handle_show_window_shortcut<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let focused = app
        .get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false);

    if focused {
        if let Some(window) = app.get_webview_window("main") {
            if let Err(error) = window.minimize() {
                eprintln!("[GlobalShortcut] Failed to minimize main window: {error}");
            }
        }
        return;
    }

    reveal_main_window(app);
}

fn register_show_window_shortcut<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), tauri_plugin_global_shortcut::Error> {
    app.global_shortcut()
        .on_shortcut(show_window_shortcut(), |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                handle_show_window_shortcut(app);
            }
        })
}

fn show_window_shortcut_enabled_on_launch(app: &tauri::App) -> bool {
    let Ok(settings_path) = app.path().resolve(
        APP_SETTINGS_RELATIVE_PATH,
        tauri::path::BaseDirectory::LocalData,
    ) else {
        return true;
    };

    let Ok(raw) = std::fs::read_to_string(settings_path) else {
        return true;
    };

    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|settings| settings.get("showWindowShortcutEnabled")?.as_bool())
        .unwrap_or(true)
}

#[tauri::command]
fn set_show_window_shortcut_enabled(app: tauri::AppHandle, enabled: bool) {
    if enabled {
        if let Err(error) = register_show_window_shortcut(&app) {
            eprintln!("[GlobalShortcut] Failed to register {SHOW_WINDOW_SHORTCUT_LABEL}: {error}");
        }
    } else if let Err(error) = app.global_shortcut().unregister(show_window_shortcut()) {
        eprintln!("[GlobalShortcut] Failed to unregister {SHOW_WINDOW_SHORTCUT_LABEL}: {error}");
    }
}

// 我们的 run 函数现在主要负责组装（Wiring）
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal_main_window(app);
        }))
        .setup(|app| {
            // 初始化 ModWatcher 状态
            app.manage(commands::mod_manager::ModWatcher(Mutex::new(None)));
            app.manage(commands::mod_library::ModLibraryWatcher(Mutex::new(None)));

            // 将随包发布的 GameType 同步到用户配置目录，
            // 确保新版本的数据类型 JSON 能覆盖旧版本文件。
            match gametype::sync_bundled_gametype_to_config() {
                Ok(copied) => {
                    println!("[GameType] Synced {copied} bundled GameType file(s) to user config.");
                }
                Err(error) => {
                    eprintln!("[GameType] Failed to sync bundled GameType folder: {error}");
                }
            }

            if show_window_shortcut_enabled_on_launch(app) {
                if let Err(error) = register_show_window_shortcut(app.handle()) {
                    // A shortcut collision must not prevent SSMT from starting.
                    eprintln!(
                        "[GlobalShortcut] Failed to register {SHOW_WINDOW_SHORTCUT_LABEL}: {error}"
                    );
                }
            }

            Ok(())
        })
        // 从各个子模块中注册命令
        .invoke_handler(tauri::generate_handler![
            set_show_window_shortcut_enabled,
            plugin::commands::plugin_registry_snapshot,
            plugin::commands::plugin_ui_routes,
            plugin::commands::plugin_capabilities,
            plugin::commands::set_plugin_enabled,
            plugin::commands::install_plugin_package,
            plugin::commands::get_plugin_setting,
            plugin::commands::set_plugin_setting,
            config::path_manager::ssmt_install_directory,
            commands::ccswitch::import_ccswitch_providers,
            commands::clipboard::clipboard_write_text,
            commands::game_discovery::find_game_executable,
            commands::game_launcher::configure_zzmi_launch_settings,
            commands::game_launcher::configure_wwmi_launch_settings,
            commands::game_launcher::execute_external_program,
            commands::game_launcher::launch_programs,
            commands::game_launcher::file_md5,
            commands::file_locks::query_file_locks,
            commands::file_locks::terminate_file_lock_owners,
            commands::file_locks::rename_mod_with_retry,
            commands::mod_manager::watch_mods,
            commands::mod_manager::unwatch_mods,
            commands::mod_manager::preview_mod_archive,
            commands::mod_manager::mod_install_target_exists,
            commands::mod_manager::install_mod_archive,
            commands::mod_manager::gamebanana_download_and_install_mod,
            commands::mod_manager::cancel_gamebanana_download_and_install_mod,
            commands::mod_manager::nexusmods_download_and_install_mod,
            commands::mod_manager::cancel_nexusmods_download_and_install_mod,
            commands::mod_manager::export_mod_archive,
            commands::mod_manager::scan_directory,
            commands::mod_manager::get_mod_key_list,
            commands::mod_library::mod_library_stream_scan,
            commands::mod_library::mod_library_scan_group,
            commands::mod_library::mod_library_refresh_group,
            commands::mod_library::mod_library_refresh_all,
            commands::mod_library::mod_library_all_mods,
            commands::mod_library::mod_library_search,
            commands::mod_library::watch_mod_library,
            commands::mod_library::unwatch_mod_library,
            commands::mod_library::find_nested_ini_files,
            commands::extract_model::extract_models_new,
            commands::extract_model::full_extract,
            commands::extract_model::analyze_draw_ib_submeshes,
            commands::extract_model::regenerate_draw_ib_component_json,
            commands::vscheck::update_vscheck,
            commands::vscheck::generate_vscheck,
            commands::extract_textures::extract_deduped_textures,
            commands::extract_textures::extract_trianglelist_textures,
            commands::extract_textures::prepare_dds_webgl_preview,
            commands::texture_mod_maker::scan_frame_analysis_textures,
            commands::texture_mod_maker::texture_mod_ffmpeg_status,
            commands::texture_mod_maker::install_texture_mod_ffmpeg,
            commands::texture_mod_maker::texture_mod_media_info,
            commands::texture_mod_maker::list_texture_mod_sequence,
            commands::texture_mod_maker::prepare_texture_mod_preview,
            commands::texture_mod_maker::generate_texture_mod,
            commands::texture_mod_maker::cancel_texture_mod_generation,
            commands::texture_mod_maker::texture_mod_generation_progress,
            commands::compress::extract_zip_archive,
            commands::compress::extract_archive_to_dir,
            commands::compress::create_rar_archive,
            commands::compress::create_mod_archive,
            commands::workspace_merge::workspace_merge_preview,
            commands::workspace_merge::workspace_merge,
            workspace_access::commands::workspace_access_preflight,
            workspace_access::commands::workspace_access_create_archive,
            workspace_access::commands::workspace_access_validate_archive,
            workspace_access::commands::workspace_access_download_archive,
            workspace_access::commands::workspace_access_check_disk_space,
            workspace_access::commands::workspace_access_import_archive,
            workspace_access::commands::workspace_access_import_metadata_skeleton,
            workspace_access::commands::workspace_access_publish,
            workspace_access::commands::workspace_access_create_and_publish,
            workspace_access::commands::workspace_access_cancel_upload,
            workspace_access::commands::workspace_access_fetch_index,
            workspace_access::commands::workspace_access_fetch_metadata,
            workspace_access::commands::workspace_access_download_entry,
            workspace_access::commands::workspace_access_record_download,
            workspace_access::commands::workspace_access_download_and_import_entry,
            commands::recycle_bin::move_file_to_recycle_bin,
            commands::recycle_bin::move_dir_to_recycle_bin,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
