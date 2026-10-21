use crate::engine::{benchmark, compress, extract, file_ops, hasher, list, windows_fs};
use crate::error::AppError;
use crate::models::{
    ArchiveEntry, BenchmarkResult, CompressOptions, DriveItem, ExtractOptions, FileItem,
    HashResult, ProgressPayload,
};
use crate::task_manager::TaskManager;
use std::process::Command;
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Channel;

#[tauri::command]
pub fn get_drives() -> Result<Vec<DriveItem>, AppError> {
    windows_fs::get_logical_drives()
}

#[tauri::command]
pub fn list_directory(path: String) -> Result<Vec<FileItem>, AppError> {
    windows_fs::list_directory_entries(&path)
}

#[tauri::command]
pub fn list_archive(path: String, password: Option<String>) -> Result<Vec<ArchiveEntry>, AppError> {
    list::list_archive_contents(&path, password)
}

#[tauri::command]
pub async fn compress_files(
    options: CompressOptions,
    on_progress: Channel<ProgressPayload>,
    task_manager: State<'_, TaskManager>,
) -> Result<(), AppError> {
    let task_id = options.task_id.clone();
    let cancel_flag = task_manager.register(&task_id);
    let tm = Arc::new(task_manager.inner().clone());
    let tm_cleanup = Arc::clone(&tm);
    let task_id_cleanup = task_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let res = compress::compress_archive(options, cancel_flag, &on_progress);
        tm_cleanup.unregister(&task_id_cleanup);
        res
    })
    .await
    .map_err(|e| AppError::new(format!("Task execution failed: {}", e)))?
}

#[tauri::command]
pub async fn extract_archive_files(
    options: ExtractOptions,
    on_progress: Channel<ProgressPayload>,
    task_manager: State<'_, TaskManager>,
) -> Result<(), AppError> {
    let task_id = options.task_id.clone();
    let cancel_flag = task_manager.register(&task_id);
    let tm = Arc::new(task_manager.inner().clone());
    let tm_cleanup = Arc::clone(&tm);
    let task_id_cleanup = task_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let res = extract::extract_archive(options, cancel_flag, &on_progress);
        tm_cleanup.unregister(&task_id_cleanup);
        res
    })
    .await
    .map_err(|e| AppError::new(format!("Task execution failed: {}", e)))?
}

#[tauri::command]
pub async fn calculate_hashes(
    path: String,
    task_id: String,
    on_progress: Channel<ProgressPayload>,
    task_manager: State<'_, TaskManager>,
) -> Result<HashResult, AppError> {
    let cancel_flag = task_manager.register(&task_id);
    let tm = Arc::new(task_manager.inner().clone());
    let tm_cleanup = Arc::clone(&tm);
    let task_id_cleanup = task_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let res = hasher::calculate_file_hashes(&path, &task_id, cancel_flag, &on_progress);
        tm_cleanup.unregister(&task_id_cleanup);
        res
    })
    .await
    .map_err(|e| AppError::new(format!("Hashing task failed: {}", e)))?
}

#[tauri::command]
pub async fn run_benchmark(
    threads: usize,
    task_id: String,
    on_progress: Channel<ProgressPayload>,
    task_manager: State<'_, TaskManager>,
) -> Result<BenchmarkResult, AppError> {
    let cancel_flag = task_manager.register(&task_id);
    let tm = Arc::new(task_manager.inner().clone());
    let tm_cleanup = Arc::clone(&tm);
    let task_id_cleanup = task_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let res = benchmark::run_cpu_benchmark(threads, &task_id, cancel_flag, &on_progress);
        tm_cleanup.unregister(&task_id_cleanup);
        res
    })
    .await
    .map_err(|e| AppError::new(format!("Benchmark task failed: {}", e)))?
}

#[tauri::command]
pub fn cancel_task(
    task_id: String,
    task_manager: State<'_, TaskManager>,
) -> Result<bool, AppError> {
    Ok(task_manager.cancel(&task_id))
}

#[tauri::command]
pub fn delete_to_recycle_bin(paths: Vec<String>) -> Result<(), AppError> {
    windows_fs::move_to_recycle_bin(&paths)
}

#[tauri::command]
pub fn get_launch_action() -> Result<Option<crate::models::LaunchAction>, AppError> {
    // `9-zip.exe <action> <path>...` from Explorer menus, or `9-zip.exe <path>`
    // from a file association.
    let args: Vec<String> = std::env::args().skip(1).collect();
    Ok(match args.as_slice() {
        [] => None,
        [path] if std::path::Path::new(path).exists() => Some(crate::models::LaunchAction {
            action: "open".to_string(),
            target_path: path.clone(),
            target_paths: vec![path.clone()],
        }),
        [_] => None,
        [action, paths @ ..] => Some(crate::models::LaunchAction {
            action: action.clone(),
            target_path: paths[0].clone(),
            target_paths: paths.to_vec(),
        }),
    })
}

#[tauri::command]
pub fn is_packaged() -> bool {
    crate::engine::packaging::is_packaged()
}

#[tauri::command]
pub fn register_shell_menu() -> Result<(), AppError> {
    if crate::engine::packaging::is_packaged() {
        return Err(AppError::new(
            "9-zip was installed from the Microsoft Store, so its Explorer menu is already set up by Windows.",
        ));
    }
    crate::engine::context_menu::register_shell_context_menu()
}

#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> Result<String, AppError> {
    // tauri.conf.json is the single source of the version number.
    Ok(app.package_info().version.to_string())
}

#[tauri::command]
pub fn get_nine_z_manifest(
    path: String,
) -> Result<crate::engine::nine_z_format::NineZManifest, AppError> {
    crate::engine::nine_z_format::read_manifest_nine_z(std::path::Path::new(&path))
}

#[tauri::command]
pub async fn test_archive(
    path: String,
    password: Option<String>,
    task_id: String,
    on_progress: Channel<ProgressPayload>,
    task_manager: State<'_, TaskManager>,
) -> Result<crate::engine::test::TestReport, AppError> {
    let cancel_flag = task_manager.register(&task_id);
    let tm = Arc::new(task_manager.inner().clone());

    tauri::async_runtime::spawn_blocking(move || {
        let res = crate::engine::test::test_archive(
            &path,
            password.as_deref(),
            &task_id,
            &cancel_flag,
            &on_progress,
        );
        tm.unregister(&task_id);
        res
    })
    .await
    .map_err(|e| AppError::new(format!("Test task failed: {}", e)))?
}

#[tauri::command]
pub fn open_path(path: String, verb: Option<String>) -> Result<(), AppError> {
    file_ops::open_with_shell(&path, verb.as_deref().unwrap_or("open"))
}

#[tauri::command]
pub fn show_properties(window: tauri::WebviewWindow, path: String) -> Result<(), AppError> {
    let owner = window
        .hwnd()
        .map_err(|e| AppError::new(format!("Cannot find the 9-zip window: {}", e)))?;
    file_ops::show_properties(owner.0, &path)
}

#[tauri::command]
pub async fn archive_properties(
    path: String,
    password: Option<String>,
) -> Result<crate::engine::properties::ArchiveInfo, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::engine::properties::archive_properties(&path, password)
    })
    .await
    .map_err(|e| AppError::new(format!("Properties task failed: {}", e)))?
}

#[tauri::command]
pub fn path_exists(path: String) -> bool {
    std::path::Path::new(&path).exists()
}

#[tauri::command]
pub fn rename_item(path: String, new_name: String) -> Result<String, AppError> {
    file_ops::rename(&path, &new_name)
}

#[tauri::command]
pub async fn transfer_items(
    paths: Vec<String>,
    destination: String,
    move_items: bool,
) -> Result<file_ops::TransferReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        file_ops::transfer(&paths, &destination, move_items)
    })
    .await
    .map_err(|e| AppError::new(format!("Copy task failed: {}", e)))?
}

#[tauri::command]
pub fn get_known_folders() -> file_ops::KnownFolders {
    file_ops::known_folders()
}

fn favorites_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppError> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::new(format!("Cannot find the app data folder: {}", e)))?;
    Ok(dir.join("favorites.json"))
}

fn settings_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppError> {
    Ok(favorites_file(app)?.with_file_name("settings.json"))
}

#[tauri::command]
pub fn get_settings(
    app: tauri::AppHandle,
) -> Result<serde_json::Map<String, serde_json::Value>, AppError> {
    Ok(file_ops::load_settings(&settings_file(&app)?))
}

#[tauri::command]
pub fn set_settings(
    app: tauri::AppHandle,
    changes: serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    file_ops::save_settings(&settings_file(&app)?, changes)
}

#[tauri::command]
pub fn get_favorites(app: tauri::AppHandle) -> Result<Vec<String>, AppError> {
    Ok(file_ops::load_favorites(&favorites_file(&app)?))
}

#[tauri::command]
pub fn set_favorites(app: tauri::AppHandle, favorites: Vec<String>) -> Result<(), AppError> {
    file_ops::save_favorites(&favorites_file(&app)?, &favorites)
}

/// Folder for files opened from inside archives; emptied at every start.
pub fn open_temp_root() -> std::path::PathBuf {
    std::env::temp_dir().join("9-zip-open")
}

/// Extracts some entries of an archive into a new temporary folder and
/// returns where each one landed.
fn extract_to_temp(
    archive_path: String,
    entries: Vec<String>,
    password: Option<String>,
    on_progress: &Channel<ProgressPayload>,
) -> Result<Vec<std::path::PathBuf>, AppError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dest = open_temp_root().join(stamp.to_string());
    let options = ExtractOptions {
        archive_path,
        destination: dest.to_string_lossy().to_string(),
        password,
        overwrite: true,
        task_id: format!("temp_{}", stamp),
        entries: Some(entries.clone()),
    };
    extract::extract_archive(
        options,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        on_progress,
    )?;
    entries
        .iter()
        .map(|entry| extract::safe_join(&dest, entry))
        .collect()
}

/// Like double-clicking a file inside an archive in 7-Zip: extracts that one
/// entry to a temporary folder and opens it with its usual program.
#[tauri::command]
pub async fn open_archive_entry(
    archive_path: String,
    entry: String,
    password: Option<String>,
    on_progress: Channel<ProgressPayload>,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = extract_to_temp(archive_path, vec![entry], password, &on_progress)?;
        file_ops::open_with_shell(&files[0].to_string_lossy(), "open")
    })
    .await
    .map_err(|e| AppError::new(format!("Open task failed: {}", e)))?
}

/// Inside an archive, dragging items out first extracts them to a temporary
/// folder; Explorer then copies them from there.
#[tauri::command]
pub async fn extract_for_drag(
    archive_path: String,
    entries: Vec<String>,
    password: Option<String>,
    on_progress: Channel<ProgressPayload>,
) -> Result<Vec<String>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = extract_to_temp(archive_path, entries, password, &on_progress)?;
        Ok(files
            .into_iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect())
    })
    .await
    .map_err(|e| AppError::new(format!("Extract task failed: {}", e)))?
}

/// Starts a Windows drag-and-drop of files out of 9-zip, so they can be
/// dropped into Explorer or any other program, as in 7-Zip.
#[tauri::command]
pub fn start_drag_out(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
) -> Result<(), AppError> {
    if paths.is_empty() {
        return Ok(());
    }
    app.run_on_main_thread(move || {
        let files = paths.into_iter().map(std::path::PathBuf::from).collect();
        let image = drag::Image::Raw(include_bytes!("../icons/64x64.png").to_vec());
        let _ = drag::start_drag(
            &window,
            drag::DragItem::Files(files),
            image,
            |_, _| {},
            drag::Options::default(),
        );
    })
    .map_err(|e| AppError::new(format!("Cannot start dragging: {}", e)))
}

#[tauri::command]
pub fn show_in_folder(path: String) -> Result<(), AppError> {
    Command::new("explorer")
        .args(["/select,", &path])
        .spawn()
        .map_err(|e| AppError::new(format!("Failed to open Explorer: {}", e)))?;
    Ok(())
}
