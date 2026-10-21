//! File manager actions of the main window: open, rename, copy, move,
//! favorites and known folders. Nothing here overwrites an existing file.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{GUID, HSTRING, PCWSTR};

/// Opens a file with the program Windows associates with it. `verb` is
/// "open" (View) or "edit" (Edit); "edit" falls back to "open" when the
/// file type has no editor.
pub fn open_with_shell(path: &str, verb: &str) -> Result<(), AppError> {
    if !Path::new(path).exists() {
        return Err(AppError::new(format!(
            "Cannot open {}: it no longer exists.",
            path
        )));
    }
    let run = |verb: &str| {
        let result = unsafe {
            ShellExecuteW(
                None,
                &HSTRING::from(verb),
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success.
        result.0 as isize > 32
    };
    if run(verb) || (verb != "open" && run("open")) {
        Ok(())
    } else {
        Err(AppError::new(format!(
            "Cannot open {}: Windows has no program for this file type.",
            path
        )))
    }
}

/// Opens the Windows Properties window for a file or folder, as 7-Zip does.
/// `owner` is the 9-zip window handle.
pub fn show_properties(owner: *mut std::ffi::c_void, path: &str) -> Result<(), AppError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{SHOP_FILEPATH, SHObjectProperties};
    let shown = unsafe {
        SHObjectProperties(
            HWND(owner),
            SHOP_FILEPATH,
            &HSTRING::from(path),
            PCWSTR::null(),
        )
    };
    if shown.as_bool() {
        Ok(())
    } else {
        Err(AppError::new(format!(
            "Cannot show the properties of {}",
            path
        )))
    }
}

/// Renames a file or folder in place. Refuses names with path separators
/// and never replaces an existing item.
pub fn rename(path: &str, new_name: &str) -> Result<String, AppError> {
    let new_name = new_name.trim();
    if new_name.is_empty()
        || new_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
        || new_name == "."
        || new_name == ".."
    {
        return Err(AppError::new(
            "A file name cannot be empty or contain any of these characters: \\ / : * ? \" < > |",
        ));
    }
    let from = PathBuf::from(path);
    let target = from
        .parent()
        .ok_or_else(|| AppError::new("Cannot rename a drive."))?
        .join(new_name);
    if target.exists() && !same_path_ignoring_case(&from, &target) {
        return Err(AppError::new(format!(
            "Cannot rename: \"{}\" already exists in this folder.",
            new_name
        )));
    }
    fs::rename(&from, &target)
        .map_err(|e| AppError::new(format!("Cannot rename {}: {}", from.display(), e)))?;
    Ok(target.to_string_lossy().to_string())
}

fn same_path_ignoring_case(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferReport {
    pub done: usize,
    /// Items left alone because something with the same name already exists.
    pub skipped: Vec<String>,
}

/// Copies or moves files and folders into `destination`. Existing items are
/// never overwritten: they are skipped and listed in the report.
pub fn transfer(
    paths: &[String],
    destination: &str,
    move_items: bool,
) -> Result<TransferReport, AppError> {
    let dest = PathBuf::from(destination);
    if !dest.is_dir() {
        return Err(AppError::new(format!("{} is not a folder.", destination)));
    }
    let mut report = TransferReport::default();
    for path in paths {
        let source = PathBuf::from(path);
        let name = source
            .file_name()
            .ok_or_else(|| AppError::new(format!("Cannot copy {}", path)))?;
        let target = dest.join(name);
        if dest.starts_with(&source) {
            return Err(AppError::new(format!(
                "Cannot copy {} into itself.",
                source.display()
            )));
        }
        if target.exists() {
            report.skipped.push(target.to_string_lossy().to_string());
            continue;
        }
        if move_items {
            // Same drive: a rename. Otherwise copy, then remove the original.
            if fs::rename(&source, &target).is_err() {
                let skipped_before = report.skipped.len();
                copy_recursive(&source, &target, &mut report)?;
                // Remove the original only when every part of it was copied.
                if report.skipped.len() == skipped_before {
                    if source.is_dir() {
                        fs::remove_dir_all(&source)?;
                    } else {
                        fs::remove_file(&source)?;
                    }
                }
            }
        } else {
            copy_recursive(&source, &target, &mut report)?;
        }
        report.done += 1;
    }
    Ok(report)
}

fn copy_recursive(from: &Path, to: &Path, report: &mut TransferReport) -> Result<(), AppError> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if target.exists() {
                report.skipped.push(target.to_string_lossy().to_string());
                continue;
            }
            copy_recursive(&entry.path(), &target, report)?;
        }
    } else {
        fs::copy(from, to)
            .map_err(|e| AppError::new(format!("Cannot copy {}: {}", from.display(), e)))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownFolders {
    pub desktop: Option<String>,
    pub documents: Option<String>,
    pub downloads: Option<String>,
}

fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let text = path.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _));
        text
    }
}

/// The user's real Desktop, Documents and Downloads, even when OneDrive or
/// a policy has moved them.
pub fn known_folders() -> KnownFolders {
    KnownFolders {
        desktop: known_folder(&FOLDERID_Desktop),
        documents: known_folder(&FOLDERID_Documents),
        downloads: known_folder(&FOLDERID_Downloads),
    }
}

/// Saved favorite folders, kept as JSON in 9-zip's app data folder.
pub fn load_favorites(file: &Path) -> Vec<String> {
    fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the favorites atomically: a temporary file, then a rename.
pub fn save_favorites(file: &Path, favorites: &[String]) -> Result<(), AppError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = file.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(favorites)?)?;
    fs::rename(&temp, file)?;
    Ok(())
}

/// Window settings (such as column widths), kept as one JSON object in the
/// app data folder.
pub fn load_settings(file: &Path) -> serde_json::Map<String, serde_json::Value> {
    fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Merges `changes` into the saved settings, written atomically.
pub fn save_settings(
    file: &Path,
    changes: serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    let mut settings = load_settings(file);
    settings.extend(changes);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = file.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(&settings)?)?;
    fs::rename(&temp, file)?;
    Ok(())
}

/// The folder of the shortcut that started 9-zip, if a shortcut did, so the
/// window opens there (a 9-zip shortcut on the Desktop shows the Desktop).
/// Start menu and taskbar shortcuts are ignored: their folder means nothing
/// to the user.
pub fn shortcut_folder() -> Option<String> {
    use windows::Win32::System::Threading::{
        GetStartupInfoW, STARTF_TITLEISLINKNAME, STARTUPINFOW,
    };
    let mut info = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    unsafe { GetStartupInfoW(&mut info) };
    if !info.dwFlags.contains(STARTF_TITLEISLINKNAME) || info.lpTitle.is_null() {
        return None;
    }
    let link = unsafe { info.lpTitle.to_string() }.ok()?;
    folder_for_shortcut(&link)
}

fn folder_for_shortcut(link: &str) -> Option<String> {
    let lower = link.to_lowercase();
    let menus = [
        "\\microsoft\\windows\\start menu\\",
        "\\microsoft\\internet explorer\\quick launch\\",
    ];
    if menus.iter().any(|m| lower.contains(m)) {
        return None;
    }
    let folder = Path::new(link).parent()?;
    folder
        .is_dir()
        .then(|| folder.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("9z_fileops_{}", name));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("mkdir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rename_never_replaces() {
        let tmp = TempDir::new("rename");
        fs::write(tmp.0.join("a.txt"), b"a").expect("write");
        fs::write(tmp.0.join("b.txt"), b"b").expect("write");
        let a = tmp.0.join("a.txt").to_string_lossy().to_string();
        assert!(rename(&a, "b.txt").is_err());
        assert!(rename(&a, "bad/name").is_err());
        let renamed = rename(&a, "c.txt").expect("rename");
        assert!(renamed.ends_with("c.txt"));
        assert_eq!(fs::read(tmp.0.join("b.txt")).expect("read"), b"b");
    }

    #[test]
    fn copy_and_move_skip_existing() {
        let tmp = TempDir::new("transfer");
        let src = tmp.0.join("src");
        let dest = tmp.0.join("dest");
        fs::create_dir_all(src.join("folder")).expect("mkdir");
        fs::create_dir_all(&dest).expect("mkdir");
        fs::write(src.join("one.txt"), b"1").expect("write");
        fs::write(src.join("folder").join("two.txt"), b"2").expect("write");
        fs::write(dest.join("one.txt"), b"keep").expect("write");

        let paths = vec![
            src.join("one.txt").to_string_lossy().to_string(),
            src.join("folder").to_string_lossy().to_string(),
        ];
        let report = transfer(&paths, &dest.to_string_lossy(), false).expect("copy");
        assert_eq!(report.done, 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(fs::read(dest.join("one.txt")).expect("read"), b"keep");
        assert_eq!(
            fs::read(dest.join("folder").join("two.txt")).expect("read"),
            b"2"
        );

        let dest2 = tmp.0.join("dest2");
        fs::create_dir_all(&dest2).expect("mkdir");
        let moved = transfer(&paths[1..], &dest2.to_string_lossy(), true).expect("move");
        assert_eq!(moved.done, 1);
        assert!(!src.join("folder").exists());
        assert!(dest2.join("folder").join("two.txt").exists());
    }

    #[test]
    fn refuses_copy_into_itself() {
        let tmp = TempDir::new("itself");
        let folder = tmp.0.join("folder");
        fs::create_dir_all(folder.join("inner")).expect("mkdir");
        let paths = vec![folder.to_string_lossy().to_string()];
        assert!(transfer(&paths, &folder.join("inner").to_string_lossy(), false).is_err());
    }

    #[test]
    fn favorites_round_trip() {
        let tmp = TempDir::new("fav");
        let file = tmp.0.join("app").join("favorites.json");
        assert!(load_favorites(&file).is_empty());
        save_favorites(&file, &["C:\\Work".to_string()]).expect("save");
        assert_eq!(load_favorites(&file), vec!["C:\\Work".to_string()]);
    }

    #[test]
    fn settings_merge_and_survive() {
        let tmp = TempDir::new("settings");
        let file = tmp.0.join("app").join("settings.json");
        let mut first = serde_json::Map::new();
        first.insert("columnWidths".into(), serde_json::json!({"name": 300}));
        save_settings(&file, first).expect("save");
        let mut second = serde_json::Map::new();
        second.insert("other".into(), serde_json::json!(true));
        save_settings(&file, second).expect("save");
        let loaded = load_settings(&file);
        assert_eq!(loaded["columnWidths"]["name"], 300);
        assert_eq!(loaded["other"], true);
    }

    #[test]
    fn start_folder_comes_from_ordinary_shortcuts_only() {
        let tmp = TempDir::new("shortcut");
        let link = tmp.0.join("9-zip.lnk");
        assert_eq!(
            folder_for_shortcut(&link.to_string_lossy()),
            Some(tmp.0.to_string_lossy().to_string())
        );
        assert_eq!(
            folder_for_shortcut(
                "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\9-zip.lnk"
            ),
            None
        );
        assert_eq!(
            folder_for_shortcut(
                "C:\\Users\\A\\AppData\\Roaming\\Microsoft\\Internet Explorer\\Quick Launch\\User Pinned\\TaskBar\\9-zip.lnk"
            ),
            None
        );
    }

    #[test]
    fn finds_the_desktop() {
        assert!(known_folders().desktop.is_some());
    }
}
