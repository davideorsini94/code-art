use tauri::Manager;
use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let window = app.get_webview_window("main").expect("finestra main assente");
            apply_vibrancy(&window, NSVisualEffectMaterial::HudWindow, None, None)
                .expect("vibrancy supportata solo su macOS");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
