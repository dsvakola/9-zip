//! The "9-zip" submenu in the Windows 11 Explorer context menu.
//!
//! Windows 11 shows menu items from Store (MSIX) apps only through COM
//! objects that implement `IExplorerCommand`. This DLL provides one: a
//! "9-zip" entry with 7-Zip's usual commands. Each command starts
//! `9-zip.exe` from the same folder with an action and the selected paths,
//! for example `9-zip.exe extract_here "C:\file.zip"`.

#![allow(non_snake_case)]

use std::cell::Cell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{
    BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER,
    HINSTANCE, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECF_HASSUBCOMMANDS, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand,
    IEnumExplorerCommand_Impl, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SHStrDupW,
    SIGDN_FILESYSPATH,
};
use windows::core::{GUID, HRESULT, HSTRING, Interface, PWSTR, Result, implement};

/// Also written in the MSIX manifest (msix/AppxManifest.template.xml).
pub const CLSID_NINEZIP_MENU: GUID = GUID::from_u128(0xbc129e3b_1070_4766_9e57_27e5fa3f08d9);

static MODULE: AtomicIsize = AtomicIsize::new(0);

const ARCHIVE_EXTENSIONS: [&str; 12] = [
    ".9z", ".7z", ".zip", ".tar", ".gz", ".tgz", ".bz2", ".xz", ".zst", ".lz4", ".br", ".tbz2",
];

fn is_archive(path: &str) -> bool {
    let lower = path.to_lowercase();
    ARCHIVE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Root,
    AddDialog,
    Add9z,
    AddZip,
    ExtractHere,
    ExtractTo,
    Test,
}

const SUBCOMMANDS: [Kind; 6] = [
    Kind::AddDialog,
    Kind::Add9z,
    Kind::AddZip,
    Kind::ExtractHere,
    Kind::ExtractTo,
    Kind::Test,
];

impl Kind {
    fn action(self) -> &'static str {
        match self {
            Kind::Root => "",
            Kind::AddDialog => "add",
            Kind::Add9z => "quick_9z",
            Kind::AddZip => "quick_zip",
            Kind::ExtractHere => "extract_here",
            Kind::ExtractTo => "extract_to",
            Kind::Test => "test",
        }
    }

    fn needs_archives(self) -> bool {
        matches!(self, Kind::ExtractHere | Kind::ExtractTo | Kind::Test)
    }
}

/// Name for "Add to <name>.9z": the item's own name for one item, otherwise
/// the folder they are in (as 7-Zip does).
fn archive_base_name(paths: &[String]) -> String {
    let first = Path::new(&paths[0]);
    let name = if paths.len() > 1 {
        first.parent().and_then(Path::file_name)
    } else if first.is_dir() {
        first.file_name()
    } else {
        first.file_stem()
    };
    name.map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "archive".to_string())
}

/// Folder name for "Extract to": the archive name without its extensions.
fn extract_folder_name(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let lower = name.to_lowercase();
    for ext in [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tar.lz4", ".tar.br",
    ] {
        if lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].to_string(),
        _ => name,
    }
}

/// The menu text for a command and the current selection.
pub fn title_for(kind: Kind, paths: &[String]) -> String {
    match kind {
        Kind::Root => "9-zip".to_string(),
        Kind::AddDialog => "Add to archive...".to_string(),
        Kind::Add9z if !paths.is_empty() => format!("Add to \"{}.9z\"", archive_base_name(paths)),
        Kind::AddZip if !paths.is_empty() => format!("Add to \"{}.zip\"", archive_base_name(paths)),
        Kind::Add9z => "Add to .9z archive".to_string(),
        Kind::AddZip => "Add to .zip archive".to_string(),
        Kind::ExtractHere => "Extract here".to_string(),
        Kind::ExtractTo if paths.len() == 1 => {
            format!("Extract to \"{}\\\"", extract_folder_name(&paths[0]))
        }
        Kind::ExtractTo => "Extract each to its own folder".to_string(),
        Kind::Test => "Test archive".to_string(),
    }
}

/// Whether a command shows for this selection.
pub fn visible_for(kind: Kind, paths: &[String]) -> bool {
    !paths.is_empty() && (!kind.needs_archives() || paths.iter().all(|p| is_archive(p)))
}

fn selected_paths(items: Option<&IShellItemArray>) -> Vec<String> {
    let Some(items) = items else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    unsafe {
        let count = items.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(item) = items.GetItemAt(i) else {
                continue;
            };
            let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
                continue;
            };
            if let Ok(text) = name.to_string() {
                paths.push(text);
            }
            CoTaskMemFree(Some(name.0 as *const c_void));
        }
    }
    paths
}

/// 9-zip.exe, next to this DLL in the package folder.
fn app_exe() -> PathBuf {
    let mut buffer = [0u16; 1024];
    let module = HMODULE(MODULE.load(Ordering::Relaxed) as *mut c_void);
    let len = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
    let dll = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
    dll.with_file_name("9-zip.exe")
}

fn to_pwstr(text: &str) -> Result<PWSTR> {
    unsafe { SHStrDupW(&HSTRING::from(text)) }
}

#[implement(IExplorerCommand)]
pub struct MenuCommand {
    kind: Kind,
}

impl MenuCommand {
    pub fn new(kind: Kind) -> Self {
        Self { kind }
    }
}

impl IExplorerCommand_Impl for MenuCommand_Impl {
    fn GetTitle(&self, items: Option<&IShellItemArray>) -> Result<PWSTR> {
        to_pwstr(&title_for(self.kind, &selected_paths(items)))
    }

    fn GetIcon(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        if self.kind == Kind::Root {
            to_pwstr(&format!("{},0", app_exe().display()))
        } else {
            Err(E_NOTIMPL.into())
        }
    }

    fn GetToolTip(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Err(E_NOTIMPL.into())
    }

    fn GetState(&self, items: Option<&IShellItemArray>, _ok_to_be_slow: BOOL) -> Result<u32> {
        let paths = selected_paths(items);
        let visible = match self.kind {
            Kind::Root => !paths.is_empty(),
            kind => visible_for(kind, &paths),
        };
        Ok(if visible {
            ECS_ENABLED.0 as u32
        } else {
            ECS_HIDDEN.0 as u32
        })
    }

    fn Invoke(&self, items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        let paths = selected_paths(items);
        if self.kind == Kind::Root || paths.is_empty() {
            return Ok(());
        }
        Command::new(app_exe())
            .arg(self.kind.action())
            .args(&paths)
            .spawn()
            .map(|_| ())
            .map_err(|_| E_FAIL.into())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(if self.kind == Kind::Root {
            ECF_HASSUBCOMMANDS.0 as u32
        } else {
            ECF_DEFAULT.0 as u32
        })
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        if self.kind == Kind::Root {
            Ok(CommandList::new(0).into())
        } else {
            Err(E_NOTIMPL.into())
        }
    }
}

#[implement(IEnumExplorerCommand)]
struct CommandList {
    position: Cell<usize>,
}

impl CommandList {
    fn new(position: usize) -> Self {
        Self {
            position: Cell::new(position),
        }
    }
}

impl IEnumExplorerCommand_Impl for CommandList_Impl {
    fn Next(&self, count: u32, out: *mut Option<IExplorerCommand>, fetched: *mut u32) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        let mut given = 0u32;
        while given < count && self.position.get() < SUBCOMMANDS.len() {
            let command: IExplorerCommand =
                MenuCommand::new(SUBCOMMANDS[self.position.get()]).into();
            unsafe { out.add(given as usize).write(Some(command)) };
            self.position.set(self.position.get() + 1);
            given += 1;
        }
        if !fetched.is_null() {
            unsafe { fetched.write(given) };
        }
        if given == count { S_OK } else { S_FALSE }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        self.position
            .set((self.position.get() + count as usize).min(SUBCOMMANDS.len()));
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.position.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        Ok(CommandList::new(self.position.get()).into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Option<&windows_core::IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let command: IExplorerCommand = MenuCommand::new(Kind::Root).into();
        unsafe { command.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(instance.0 as isize, Ordering::Relaxed);
    }
    true.into()
}

/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }
    unsafe {
        *object = std::ptr::null_mut();
        if *clsid != CLSID_NINEZIP_MENU {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, object)
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Explorer keeps shell extensions loaded; staying loaded is always safe.
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn titles_follow_the_selection_like_7zip() {
        let one_file = paths(&["C:\\Work\\report.docx"]);
        assert_eq!(title_for(Kind::Add9z, &one_file), "Add to \"report.9z\"");
        assert_eq!(title_for(Kind::AddZip, &one_file), "Add to \"report.zip\"");
        let archive = paths(&["C:\\Work\\photos.tar.gz"]);
        assert_eq!(
            title_for(Kind::ExtractTo, &archive),
            "Extract to \"photos\\\""
        );
        let many = paths(&["C:\\Windows\\a.txt", "C:\\Windows\\b.txt"]);
        assert_eq!(title_for(Kind::Add9z, &many), "Add to \"Windows.9z\"");
    }

    #[test]
    fn extract_and_test_only_for_archives() {
        let file = paths(&["C:\\notes.txt"]);
        let archive = paths(&["C:\\data.9z"]);
        assert!(visible_for(Kind::Add9z, &file));
        assert!(!visible_for(Kind::ExtractHere, &file));
        assert!(visible_for(Kind::ExtractHere, &archive));
        assert!(visible_for(Kind::Test, &archive));
        assert!(!visible_for(
            Kind::Test,
            &paths(&["C:\\data.9z", "C:\\notes.txt"])
        ));
    }

    #[test]
    fn com_objects_answer_like_explorer_expects() {
        unsafe {
            let mut object: *mut c_void = std::ptr::null_mut();
            let hr = DllGetClassObject(&CLSID_NINEZIP_MENU, &IClassFactory::IID, &mut object);
            assert_eq!(hr, S_OK);
            let factory = IClassFactory::from_raw(object);
            let root: IExplorerCommand = factory.CreateInstance(None).expect("create");
            assert_eq!(root.GetFlags().expect("flags"), ECF_HASSUBCOMMANDS.0 as u32);
            let title = root.GetTitle(None).expect("title");
            assert_eq!(title.to_string().expect("utf16"), "9-zip");
            CoTaskMemFree(Some(title.0 as *const c_void));

            let list = root.EnumSubCommands().expect("enum");
            let mut commands: [Option<IExplorerCommand>; 7] = Default::default();
            let mut fetched = 0u32;
            let hr = list.Next(&mut commands, Some(&mut fetched));
            assert_eq!(hr, S_FALSE, "asked for 7, there are 6");
            assert_eq!(fetched, 6);

            let mut other: *mut c_void = std::ptr::null_mut();
            assert_eq!(
                DllGetClassObject(&GUID::zeroed(), &IClassFactory::IID, &mut other),
                CLASS_E_CLASSNOTAVAILABLE
            );
        }
    }
}
