//! Linux versions of the few actions that call Windows directly on Windows:
//! the list of places (instead of drive letters), the Trash (instead of the
//! Recycle Bin), opening files with their usual program, and the user's
//! Desktop, Documents and Downloads folders.

use crate::error::AppError;
use crate::models::DriveItem;
use std::ffi::CString;
use std::path::Path;

use super::file_ops::KnownFolders;

/// Total and free bytes of the file system that holds `path`.
// The field types differ between 32-bit and 64-bit Linux, so the casts stay.
#[allow(clippy::unnecessary_cast)]
fn space(path: &str) -> (u64, u64) {
    let Ok(c_path) = CString::new(path) else {
        return (0, 0);
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is a valid C string and `stats` is written by statvfs.
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return (0, 0);
    }
    // SAFETY: statvfs returned 0, so it filled `stats`.
    let stats = unsafe { stats.assume_init() };
    let block = stats.f_frsize as u64;
    (stats.f_blocks as u64 * block, stats.f_bavail as u64 * block)
}

fn place(path: &str, label: &str, kind: &str) -> DriveItem {
    let (total_bytes, free_bytes) = space(path);
    DriveItem {
        letter: path.to_string(),
        label: label.to_string(),
        total_bytes,
        free_bytes,
        drive_type: kind.to_string(),
    }
}

/// The places shown where Windows shows drive letters: the whole file
/// system, the user's home folder, and USB drives and other disks mounted
/// under /media, /run/media or /mnt.
pub fn get_logical_drives() -> Result<Vec<DriveItem>, AppError> {
    let mut places = vec![place("/", "File System", "Fixed")];
    if let Some(home) = dirs::home_dir() {
        places.push(place(&home.to_string_lossy(), "Home", "Fixed"));
    }

    let user = std::env::var("USER").unwrap_or_default();
    let mut roots = vec!["/mnt".to_string()];
    if !user.is_empty() {
        roots.insert(0, format!("/media/{}", user));
        roots.insert(1, format!("/run/media/{}", user));
    }
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut found: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .collect();
        found.sort_by_key(|e| e.file_name());
        for entry in found {
            let name = entry.file_name().to_string_lossy().to_string();
            places.push(place(&entry.path().to_string_lossy(), &name, "Removable"));
        }
    }
    Ok(places)
}

/// Moves files and folders to the desktop's Trash, from where the user can
/// restore them.
pub fn move_to_recycle_bin(paths: &[String]) -> Result<(), AppError> {
    if paths.is_empty() {
        return Ok(());
    }
    trash::delete_all(paths).map_err(|e| AppError::new(format!("Cannot move to the Trash: {}", e)))
}

/// Opens a file with the program the desktop associates with it. Linux has
/// no separate "edit" action, so `verb` is not used.
pub fn open_with_shell(path: &str, _verb: &str) -> Result<(), AppError> {
    if !Path::new(path).exists() {
        return Err(AppError::new(format!(
            "Cannot open {}: it no longer exists.",
            path
        )));
    }
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| {
        AppError::new(format!(
            "Cannot open {}: no program is set up for this file type ({}).",
            path, e
        ))
    })
}

/// The user's Desktop, Documents and Downloads folders, as the desktop
/// names them (they can be translated, such as "Bureau" in French).
pub fn known_folders() -> KnownFolders {
    let text = |p: Option<std::path::PathBuf>| p.map(|p| p.to_string_lossy().to_string());
    KnownFolders {
        desktop: text(dirs::desktop_dir()),
        documents: text(dirs::document_dir()),
        downloads: text(dirs::download_dir()),
    }
}

/// Linux has no Windows-style shortcuts that carry a start folder.
pub fn shortcut_folder() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_start_with_the_file_system() {
        let places = get_logical_drives().unwrap();
        assert_eq!(places[0].letter, "/");
        assert!(places[0].total_bytes > 0);
    }

    #[test]
    fn trash_accepts_an_empty_list() {
        assert!(move_to_recycle_bin(&[]).is_ok());
    }
}
