//! Explorer integration for the installer (NSIS) version of 9-zip.
//!
//! The installer runs `9-zip.exe --register-shell` after installing and
//! `9-zip.exe --unregister-shell` before uninstalling, so nothing is left
//! behind. The Store (MSIX) version declares its menu in the package instead
//! and never calls these.

use crate::error::AppError;
use std::env;
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use winreg::RegKey;
use winreg::enums::*;

const MENU_PARENTS: [&str; 3] = ["*", "Directory", "Directory\\Background"];
const PROG_ID: &str = "9zip.Archive";

/// Tells Explorer to reload file icons and menus.
fn notify_explorer() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Removes everything `register_shell_context_menu` wrote, including the
/// entries of builds released under the old name "9z".
pub fn unregister_shell_context_menu() -> Result<(), AppError> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_ALL_ACCESS)
        .map_err(|e| AppError::new(format!("Cannot open HKCU\\Software\\Classes: {}", e)))?;
    for parent in MENU_PARENTS {
        let _ = classes.delete_subkey_all(format!("{}\\shell\\9-zip", parent));
        let _ = classes.delete_subkey_all(format!("{}\\shell\\9z", parent));
    }
    // Only release .9z if it still points at 9-zip.
    if let Ok(ext) = classes.open_subkey(".9z") {
        let owner: String = ext.get_value("").unwrap_or_default();
        if owner == PROG_ID || owner == "9z_Archive" {
            let _ = classes.delete_subkey_all(".9z");
        }
    }
    let _ = classes.delete_subkey_all(PROG_ID);
    let _ = classes.delete_subkey_all("9z_Archive");
    // Left by "Open with" choices of early test builds (nine_z.exe).
    let _ = classes.delete_subkey_all("Applications\\nine_z.exe");
    let _ = classes.delete_subkey_all("9z_auto_file");
    if let Ok(progids) = hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.9z\\OpenWithProgids",
        KEY_ALL_ACCESS,
    ) {
        for old in ["9z_auto_file", "9z_Archive", PROG_ID] {
            let _ = progids.delete_value(old);
        }
    }
    notify_explorer();
    Ok(())
}

pub fn register_shell_context_menu() -> Result<(), AppError> {
    let current_exe = env::current_exe()
        .map_err(|e| AppError::new(format!("Cannot determine exe path: {}", e)))?;
    let exe_path_str = current_exe.to_string_lossy().to_string();
    let quoted_exe = format!("\"{}\"", exe_path_str);
    let icon_str = format!("\"{}\",0", exe_path_str);

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_ALL_ACCESS)
        .map_err(|e| AppError::new(format!("Cannot open HKCU\\Software\\Classes: {}", e)))?;

    // 1. Context menu for Files (*)
    register_menu_for_key(&classes, "*", &quoted_exe, &icon_str, false)?;

    // 2. Context menu for Folders (Directory)
    register_menu_for_key(&classes, "Directory", &quoted_exe, &icon_str, true)?;

    // 3. Context menu for Folder Backgrounds
    register_menu_for_key(
        &classes,
        "Directory\\Background",
        &quoted_exe,
        &icon_str,
        true,
    )?;

    // 4. File Associations for .9z
    register_file_association(
        &classes,
        ".9z",
        PROG_ID,
        "9-zip Archive",
        &quoted_exe,
        &icon_str,
    )?;

    notify_explorer();
    Ok(())
}

fn register_menu_for_key(
    classes: &RegKey,
    parent_sub: &str,
    quoted_exe: &str,
    icon_str: &str,
    is_directory: bool,
) -> Result<(), AppError> {
    // Remove the menu left by builds released under the old name "9z".
    let _ = classes.delete_subkey_all(format!("{}\\shell\\9z", parent_sub));

    let (key_9z, _) = classes
        .create_subkey(format!("{}\\shell\\9-zip", parent_sub))
        .map_err(|e| AppError::new(format!("Registry create failed: {}", e)))?;

    key_9z.set_value("MUIVerb", &"9-zip")?;
    key_9z.set_value("Icon", &icon_str)?;
    key_9z.set_value("SubCommands", &"")?;

    let (shell_sub, _) = key_9z.create_subkey("shell")?;

    if !is_directory {
        add_sub_item(
            &shell_sub,
            "01_open",
            "Open archive",
            &format!("{} open \"%1\"", quoted_exe),
        )?;
        add_sub_item(
            &shell_sub,
            "02_extract",
            "Extract files...",
            &format!("{} extract \"%1\"", quoted_exe),
        )?;
        add_sub_item(
            &shell_sub,
            "03_extract_here",
            "Extract Here",
            &format!("{} extract_here \"%1\"", quoted_exe),
        )?;
        add_sub_item(
            &shell_sub,
            "04_extract_to",
            "Extract to folder",
            &format!("{} extract_to \"%1\"", quoted_exe),
        )?;
        add_sub_item(
            &shell_sub,
            "05_test",
            "Test archive",
            &format!("{} test \"%1\"", quoted_exe),
        )?;
    }

    add_sub_item(
        &shell_sub,
        "06_add",
        "Add to archive...",
        &format!("{} add \"%1\"", quoted_exe),
    )?;
    add_sub_item(
        &shell_sub,
        "07_quick_9z",
        "Add to .9z archive (self-healing)",
        &format!("{} quick_9z \"%1\"", quoted_exe),
    )?;
    add_sub_item(
        &shell_sub,
        "08_quick_zip",
        "Add to .zip",
        &format!("{} quick_zip \"%1\"", quoted_exe),
    )?;
    add_sub_item(
        &shell_sub,
        "09_quick_7z",
        "Add to .7z",
        &format!("{} quick_7z \"%1\"", quoted_exe),
    )?;

    if !is_directory {
        add_sub_item(
            &shell_sub,
            "10_hash",
            "CRC SHA Checksum",
            &format!("{} hash \"%1\"", quoted_exe),
        )?;
    }

    Ok(())
}

fn add_sub_item(
    shell_key: &RegKey,
    name: &str,
    label: &str,
    command_str: &str,
) -> Result<(), AppError> {
    let (item_key, _) = shell_key.create_subkey(name)?;
    item_key.set_value("MUIVerb", &label)?;
    let (cmd_key, _) = item_key.create_subkey("command")?;
    cmd_key.set_value("", &command_str)?;
    Ok(())
}

fn register_file_association(
    classes: &RegKey,
    ext: &str,
    prog_id: &str,
    description: &str,
    quoted_exe: &str,
    icon_str: &str,
) -> Result<(), AppError> {
    let (ext_key, _) = classes.create_subkey(ext)?;
    ext_key.set_value("", &prog_id)?;

    let (prog_key, _) = classes.create_subkey(prog_id)?;
    prog_key.set_value("", &description)?;

    let (icon_key, _) = prog_key.create_subkey("DefaultIcon")?;
    icon_key.set_value("", &icon_str)?;

    let (shell_key, _) = prog_key.create_subkey("shell\\open\\command")?;
    shell_key.set_value("", &format!("{} \"%1\"", quoted_exe))?;

    Ok(())
}
