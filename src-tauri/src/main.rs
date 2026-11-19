// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // rilanciata da `ssh` come SSH_ASKPASS: risponde con la password ed esce, senza avviare Tauri né finestre
    if code_art_lib::ssh::askpass::maybe_run() {
        return;
    }
    code_art_lib::run()
}
