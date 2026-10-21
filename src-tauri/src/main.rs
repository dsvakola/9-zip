// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Run by the installer: add or remove the Explorer menu, then exit.
    match std::env::args().nth(1).as_deref() {
        Some("--register-shell") => std::process::exit(nine_z_lib::register_shell(true)),
        Some("--unregister-shell") => std::process::exit(nine_z_lib::register_shell(false)),
        _ => nine_z_lib::run(),
    }
}
