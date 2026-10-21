//! Knows whether 9-zip runs as an MSIX package (Microsoft Store install).

use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::core::PWSTR;

/// True when installed from an MSIX package. The package then declares the
/// file types and Explorer verbs itself, so 9-zip must not write them to the
/// registry.
pub fn is_packaged() -> bool {
    let mut length = 0u32;
    // With no buffer this only reports whether a package identity exists.
    let result = unsafe { GetCurrentPackageFullName(&mut length, PWSTR::null()) };
    result != APPMODEL_ERROR_NO_PACKAGE
}

#[cfg(test)]
mod tests {
    #[test]
    fn tests_do_not_run_inside_a_package() {
        assert!(!super::is_packaged());
    }
}
