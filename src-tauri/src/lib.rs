mod commands;
mod engine;
mod error;
mod models;
mod task_manager;

use task_manager::TaskManager;

#[cfg(test)]
mod engine_tests;

/// For the installer: adds (`true`) or removes (`false`) the Explorer menu.
/// Returns the process exit code. Never touches a Store (MSIX) install,
/// whose menu is declared by its package.
pub fn register_shell(add: bool) -> i32 {
    if engine::packaging::is_packaged() {
        return 0;
    }
    let result = if add {
        engine::context_menu::register_shell_context_menu()
    } else {
        engine::context_menu::unregister_shell_context_menu()
    };
    if result.is_ok() { 0 } else { 1 }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Files opened from inside archives last session are no longer needed.
    let _ = std::fs::remove_dir_all(commands::open_temp_root());
    let task_manager = TaskManager::new();

    tauri::Builder::default()
        .manage(task_manager)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_drives,
            commands::list_directory,
            commands::list_archive,
            commands::compress_files,
            commands::extract_archive_files,
            commands::calculate_hashes,
            commands::run_benchmark,
            commands::cancel_task,
            commands::delete_to_recycle_bin,
            commands::get_app_version,
            commands::get_nine_z_manifest,
            commands::test_archive,
            commands::get_launch_action,
            commands::register_shell_menu,
            commands::is_packaged,
            commands::show_in_folder,
            commands::open_path,
            commands::rename_item,
            commands::path_exists,
            commands::archive_properties,
            commands::show_properties,
            commands::transfer_items,
            commands::get_known_folders,
            commands::get_start_folder,
            commands::get_favorites,
            commands::set_favorites,
            commands::get_settings,
            commands::set_settings,
            commands::open_archive_entry,
            commands::extract_for_drag,
            commands::start_drag_out,
        ])
        .run(tauri::generate_context!())
        .expect("error while running 9-zip");
}
