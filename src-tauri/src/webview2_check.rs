//! Startup check for Microsoft Edge WebView2, which draws the 9-zip window.
//! It is part of Windows 11 and is installed on almost every Windows 10 PC.
//! When it is missing, a plain Windows message explains what to install.

use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    IDOK, MB_ICONWARNING, MB_OKCANCEL, MessageBoxW, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, PCWSTR};

const DOWNLOAD_PAGE: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// True when a WebView2 runtime is installed.
pub fn webview2_available() -> bool {
    let mut version = windows_core::PWSTR::null();
    let found = unsafe {
        GetAvailableCoreWebView2BrowserVersionString(windows_core::PCWSTR::null(), &mut version)
    };
    if found.is_err() || version.is_null() {
        return false;
    }
    // The text is allocated by WebView2; take_pwstr reads and frees it.
    !webview2_com::take_pwstr(version).trim().is_empty()
}

/// Tells the user WebView2 is missing and offers to open its download page.
pub fn explain_missing_webview2() {
    let text = HSTRING::from(
        "9-zip needs Microsoft Edge WebView2, a free part of Windows.\n\n\
         Click OK to open the download page. After installing it, start 9-zip again.",
    );
    let title = HSTRING::from("9-zip");
    let choice = unsafe { MessageBoxW(None, &text, &title, MB_OKCANCEL | MB_ICONWARNING) };
    if choice == IDOK {
        unsafe {
            ShellExecuteW(
                None,
                &HSTRING::from("open"),
                &HSTRING::from(DOWNLOAD_PAGE),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }
}
