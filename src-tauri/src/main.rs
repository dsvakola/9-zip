// 9-zip - Archive Manager for Windows
// Copyright (C) 2026 Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar
// Licensed under the GNU General Public License v3.0 (GPL-3.0-only).
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Run by the installer: add or remove the Explorer menu, then exit.
    match std::env::args().nth(1).as_deref() {
        Some("--register-shell") => std::process::exit(nine_z_lib::register_shell(true)),
        Some("--unregister-shell") => std::process::exit(nine_z_lib::register_shell(false)),
        _ => {
            // Without WebView2 the window cannot be drawn: explain, then exit.
            if !nine_z_lib::webview2_available() {
                nine_z_lib::explain_missing_webview2();
                return;
            }
            nine_z_lib::run()
        }
    }
}
