//! Right-click menus for Linux file managers, like the Explorer menu on
//! Windows. Each file manager adds menu items its own way:
//!
//! - Files (Nautilus, Ubuntu and Fedora): scripts in "Scripts > 9-zip".
//! - Nemo (Linux Mint): one ".nemo_action" file per item.
//! - Dolphin (KDE): a "service menu" with a 9-zip submenu.
//!
//! The files go into the user's own folders under ~/.local/share, so no
//! administrator rights are needed. Removing the menu deletes only the
//! files 9-zip wrote, all named "9-zip...".

use crate::error::AppError;
use std::fs;
use std::path::{Path, PathBuf};

/// One menu item: file name part, label, 9-zip command, archives only.
struct Item {
    id: &'static str,
    label: &'static str,
    action: &'static str,
    archives_only: bool,
}

const ITEMS: [Item; 9] = [
    Item {
        id: "01-open",
        label: "Open archive",
        action: "open",
        archives_only: true,
    },
    Item {
        id: "02-extract-here",
        label: "Extract Here",
        action: "extract_here",
        archives_only: true,
    },
    Item {
        id: "03-extract-to",
        label: "Extract to folder",
        action: "extract_to",
        archives_only: true,
    },
    Item {
        id: "04-test",
        label: "Test archive",
        action: "test",
        archives_only: true,
    },
    Item {
        id: "05-add",
        label: "Add to archive...",
        action: "add",
        archives_only: false,
    },
    Item {
        id: "06-quick-9z",
        label: "Add to .9z archive (self-healing)",
        action: "quick_9z",
        archives_only: false,
    },
    Item {
        id: "07-quick-zip",
        label: "Add to .zip",
        action: "quick_zip",
        archives_only: false,
    },
    Item {
        id: "08-quick-7z",
        label: "Add to .7z",
        action: "quick_7z",
        archives_only: false,
    },
    Item {
        id: "09-hash",
        label: "CRC SHA Checksum",
        action: "hash",
        archives_only: false,
    },
];

/// File name endings that 9-zip opens as archives (as in system_fs.rs).
const ARCHIVE_EXTENSIONS: &str =
    "9z;zip;7z;tar;gz;tgz;bz2;tbz2;xz;txz;zst;tzst;lz4;br;rar;iso;cab;wim;";

/// The same archive types as MIME types, for Dolphin.
const ARCHIVE_MIME_TYPES: &str = "application/x-9z;application/zip;application/x-7z-compressed;\
application/x-tar;application/gzip;application/x-compressed-tar;application/x-bzip2;\
application/x-bzip-compressed-tar;application/x-xz;application/x-xz-compressed-tar;\
application/zstd;application/x-zstd-compressed-tar;application/x-lz4;application/x-brotli;\
application/vnd.rar;application/x-cd-image;";

/// The program the menu runs: the AppImage file itself when 9-zip runs as
/// an AppImage (its inner path changes every start), otherwise 9-zip.
fn program() -> Result<String, AppError> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(appimage.to_string_lossy().to_string());
    }
    let exe = std::env::current_exe()
        .map_err(|e| AppError::new(format!("Cannot find the 9-zip program: {}", e)))?;
    Ok(exe.to_string_lossy().to_string())
}

/// Quotes a path for an Exec line or a shell script.
fn quoted(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The three folders, under the given ~/.local/share folder.
struct Folders {
    nautilus: PathBuf,
    nemo: PathBuf,
    dolphin: PathBuf,
}

fn folders(data_dir: &Path) -> Folders {
    Folders {
        nautilus: data_dir.join("nautilus/scripts/9-zip"),
        nemo: data_dir.join("nemo/actions"),
        dolphin: data_dir.join("kio/servicemenus"),
    }
}

fn data_dir() -> Result<PathBuf, AppError> {
    dirs::data_dir().ok_or_else(|| AppError::new("Cannot find your home folder."))
}

fn write_executable(path: &Path, text: &str) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

pub fn register_shell_context_menu() -> Result<(), AppError> {
    register_in(&data_dir()?, &program()?)
}

pub fn unregister_shell_context_menu() -> Result<(), AppError> {
    unregister_in(&data_dir()?)
}

fn register_in(data_dir: &Path, program: &str) -> Result<(), AppError> {
    let f = folders(data_dir);
    let exe = quoted(program);

    // Files (Nautilus): one small script per item. Scripts cannot be
    // limited to archives, so every item is shown.
    fs::create_dir_all(&f.nautilus)?;
    for item in &ITEMS {
        let script = format!(
            "#!/bin/sh\n# 9-zip menu item. Removed by Tools > Remove from File Manager Menu.\nIFS='\n'\nexec {} {} $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n",
            exe, item.action
        );
        write_executable(&f.nautilus.join(item.label.trim_end_matches('.')), &script)?;
    }

    // Nemo: one action per item, archive items only on archives.
    fs::create_dir_all(&f.nemo)?;
    for item in &ITEMS {
        let extensions = if item.archives_only {
            ARCHIVE_EXTENSIONS
        } else {
            "any;"
        };
        let action = format!(
            "[Nemo Action]\nName=9-zip: {}\nComment=9-zip archive manager\nExec={} {} %F\nIcon-Name=9-zip\nSelection=notnone\nExtensions={}\nQuote=double\n",
            item.label, exe, item.action, extensions
        );
        fs::write(
            f.nemo.join(format!("9-zip-{}.nemo_action", item.id)),
            action,
        )?;
    }

    // Dolphin: a 9-zip submenu; one file for archives, one for everything.
    fs::create_dir_all(&f.dolphin)?;
    for (file, mime, archives) in [
        ("9-zip-archives.desktop", ARCHIVE_MIME_TYPES, true),
        ("9-zip.desktop", "all/allfiles;inode/directory;", false),
    ] {
        let items: Vec<&Item> = ITEMS
            .iter()
            .filter(|i| i.archives_only == archives)
            .collect();
        let mut text = format!(
            "[Desktop Entry]\nType=Service\nMimeType={}\nActions={}\nX-KDE-Submenu=9-zip\nIcon=9-zip\n",
            mime,
            items.iter().map(|i| i.action).collect::<Vec<_>>().join(";")
        );
        for item in items {
            text.push_str(&format!(
                "\n[Desktop Action {}]\nName={}\nIcon=9-zip\nExec={} {} %F\n",
                item.action, item.label, exe, item.action
            ));
        }
        write_executable(&f.dolphin.join(file), &text)?;
    }
    Ok(())
}

fn unregister_in(data_dir: &Path) -> Result<(), AppError> {
    let f = folders(data_dir);
    if f.nautilus.is_dir() {
        fs::remove_dir_all(&f.nautilus)?;
    }
    for folder in [&f.nemo, &f.dolphin] {
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name().to_string_lossy().starts_with("9-zip") {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_removes_only_its_own_menu_files() {
        let root = std::env::temp_dir().join(format!("9zip-menu-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let f = folders(&root);
        fs::create_dir_all(&f.nemo).unwrap();
        fs::write(f.nemo.join("other-app.nemo_action"), "keep").unwrap();

        register_in(&root, "/opt/My Apps/9-zip.AppImage").unwrap();
        assert_eq!(fs::read_dir(&f.nautilus).unwrap().count(), ITEMS.len());
        let nemo = fs::read_to_string(f.nemo.join("9-zip-02-extract-here.nemo_action")).unwrap();
        assert!(nemo.contains("Exec=\"/opt/My Apps/9-zip.AppImage\" extract_here %F"));
        assert!(nemo.contains("Extensions=9z;zip;"));
        let dolphin = fs::read_to_string(f.dolphin.join("9-zip.desktop")).unwrap();
        assert!(dolphin.contains("Actions=add;quick_9z;quick_zip;quick_7z;hash"));

        unregister_in(&root).unwrap();
        assert!(!f.nautilus.exists());
        assert_eq!(fs::read_dir(&f.nemo).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&f.dolphin).unwrap().count(), 0);
        fs::remove_dir_all(&root).unwrap();
    }
}
