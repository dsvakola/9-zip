//! Knows whether 9-zip runs as a store package: MSIX (Microsoft Store) on
//! Windows, Flatpak or Snap on Linux.

#[cfg(windows)]
use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
#[cfg(windows)]
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
#[cfg(windows)]
use windows::core::PWSTR;

/// True when installed from an MSIX package. The package then declares the
/// file types and Explorer verbs itself, so 9-zip must not write them to the
/// registry.
#[cfg(windows)]
pub fn is_packaged() -> bool {
    let mut length = 0u32;
    // With no buffer this only reports whether a package identity exists.
    let result = unsafe { GetCurrentPackageFullName(&mut length, PWSTR::null()) };
    result != APPMODEL_ERROR_NO_PACKAGE
}

/// True when installed from Flathub (Flatpak) or the Snap Store. The store
/// package then declares the file types itself.
#[cfg(not(windows))]
pub fn is_packaged() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some()
}

#[cfg(test)]
mod tests {
    #[test]
    fn tests_do_not_run_inside_a_package() {
        assert!(!super::is_packaged());
    }
}
