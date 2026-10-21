use crate::error::AppError;
use crate::models::{DriveItem, FileItem};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
};
use windows::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW,
};
use windows::core::PCWSTR;

pub fn get_logical_drives() -> Result<Vec<DriveItem>, AppError> {
    let mut drives = Vec::new();
    let mut buffer = [0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) };
    if len == 0 {
        // Fallback: at least return C:\
        drives.push(DriveItem {
            letter: "C:".to_string(),
            label: "Local Disk (C:)".to_string(),
            total_bytes: 0,
            free_bytes: 0,
            drive_type: "Fixed".to_string(),
        });
        return Ok(drives);
    }

    let mut start = 0;
    for i in 0..len as usize {
        if buffer[i] == 0 {
            if i > start {
                let slice = &buffer[start..i];
                let drive_str = String::from_utf16_lossy(slice);
                let trimmed = drive_str.trim_end_matches('\\');

                let mut wide_drive: Vec<u16> = drive_str.encode_utf16().collect();
                wide_drive.push(0);

                let drive_type_code = unsafe { GetDriveTypeW(PCWSTR(wide_drive.as_ptr())) };
                let drive_type = match drive_type_code {
                    2 => "Removable",
                    3 => "Fixed",
                    4 => "Network",
                    5 => "CD-ROM",
                    6 => "RAM Disk",
                    _ => "Unknown",
                }
                .to_string();

                let mut free_bytes_avail = 0u64;
                let mut total_bytes = 0u64;
                let mut total_free = 0u64;

                let success = unsafe {
                    GetDiskFreeSpaceExW(
                        PCWSTR(wide_drive.as_ptr()),
                        Some(&mut free_bytes_avail),
                        Some(&mut total_bytes),
                        Some(&mut total_free),
                    )
                };

                // Only include drives that are accessible or fixed/removable with storage
                let is_accessible = Path::new(&drive_str).exists();
                if is_accessible
                    && (drive_type_code == 3 || drive_type_code == 2 || total_bytes > 0)
                {
                    drives.push(DriveItem {
                        letter: trimmed.to_string(),
                        label: if success.is_ok() {
                            format!(
                                "{} ({})",
                                if drive_type == "Fixed" {
                                    "Local Disk"
                                } else {
                                    &drive_type
                                },
                                trimmed
                            )
                        } else {
                            trimmed.to_string()
                        },
                        total_bytes: if success.is_ok() { total_bytes } else { 0 },
                        free_bytes: if success.is_ok() { free_bytes_avail } else { 0 },
                        drive_type,
                    });
                }
            }
            start = i + 1;
        }
    }

    // Sort drives: C: first, then alphabetical
    drives.sort_by(|a, b| {
        if a.letter.eq_ignore_ascii_case("C:") {
            std::cmp::Ordering::Less
        } else if b.letter.eq_ignore_ascii_case("C:") {
            std::cmp::Ordering::Greater
        } else {
            a.letter.cmp(&b.letter)
        }
    });

    if drives.is_empty() {
        drives.push(DriveItem {
            letter: "C:".to_string(),
            label: "Local Disk (C:)".to_string(),
            total_bytes: 0,
            free_bytes: 0,
            drive_type: "Fixed".to_string(),
        });
    }

    Ok(drives)
}

pub fn list_directory_entries(dir_path: &str) -> Result<Vec<FileItem>, AppError> {
    let path = Path::new(dir_path);
    if !path.exists() {
        return Err(AppError::new(format!("Path does not exist: {}", dir_path)));
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(path)
        .map_err(|e| AppError::new(format!("Unable to read folder {}: {}", dir_path, e)))?;

    for entry_res in read_dir {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        let is_dir = file_type.is_dir();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let full_path = entry.path().to_string_lossy().to_string();
        let mut size = 0u64;
        let mut modified = None;

        if let Ok(meta) = entry.metadata() {
            size = if is_dir { 0 } else { meta.file_size() };
            modified = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|dur| dur.as_secs());
        }

        let extension = entry
            .path()
            .extension()
            .map(|ext| ext.to_string_lossy().to_string())
            .unwrap_or_default();

        let is_archive = is_archive_file(&file_name);

        entries.push(FileItem {
            name: file_name,
            path: full_path,
            size,
            is_dir,
            is_archive,
            modified,
            extension,
        });
    }

    // Sort: directories first, then alphabetical
    entries.sort_by(|a, b| {
        if a.is_dir && !b.is_dir {
            std::cmp::Ordering::Less
        } else if !a.is_dir && b.is_dir {
            std::cmp::Ordering::Greater
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(entries)
}

fn is_archive_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".9z")
        || lower.ends_with(".zip")
        || lower.ends_with(".7z")
        || lower.ends_with(".tar")
        || lower.ends_with(".gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".bz2")
        || lower.ends_with(".tbz2")
        || lower.ends_with(".xz")
        || lower.ends_with(".txz")
        || lower.ends_with(".zst")
        || lower.ends_with(".tzst")
        || lower.ends_with(".lz4")
        || lower.ends_with(".br")
        || lower.ends_with(".rar")
        || lower.ends_with(".iso")
        || lower.ends_with(".cab")
        || lower.ends_with(".wim")
}

pub fn move_to_recycle_bin(paths: &[String]) -> Result<(), AppError> {
    if paths.is_empty() {
        return Ok(());
    }

    let mut double_null_wide: Vec<u16> = Vec::new();
    for p in paths {
        let pb = PathBuf::from(p);
        let absolute = match fs::canonicalize(&pb) {
            Ok(can) => can,
            Err(_) => pb,
        };
        let abs_str = absolute.to_string_lossy().to_string();
        let clean = abs_str.strip_prefix(r"\\?\").unwrap_or(&abs_str);
        for u in clean.encode_utf16() {
            double_null_wide.push(u);
        }
        double_null_wide.push(0);
    }
    double_null_wide.push(0);

    let mut file_op = SHFILEOPSTRUCTW {
        hwnd: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        wFunc: FO_DELETE,
        pFrom: PCWSTR(double_null_wide.as_ptr()),
        pTo: PCWSTR(std::ptr::null()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT).0 as u16,
        fAnyOperationsAborted: BOOL(0),
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: PCWSTR(std::ptr::null()),
    };

    let result = unsafe { SHFileOperationW(&mut file_op) };
    if result != 0 {
        return Err(AppError::new(format!(
            "Windows Recycle Bin error code: {}",
            result
        )));
    }

    Ok(())
}
