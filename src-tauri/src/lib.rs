mod fs_tree;
mod projects;
mod store;

use std::sync::atomic::Ordering;
use tauri::{Manager, RunEvent, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(projects::Quitting::default())
        .manage(fs_tree::Watchers::default())
        .setup(|app| {
            let dir = app.path().home_dir()?.join("Library/Application Support/CodeArt");
            app.manage(store::Store::open(&dir.join("codeart.db"))?);
            projects::rebuild_menu(app.handle())?;
            projects::restore_windows(app.handle())?;
            Ok(())
        })
        .on_menu_event(|app, event| projects::on_menu_event(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                projects::on_window_destroyed(window.app_handle(), window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            projects::project_open_folder,
            projects::project_current,
            projects::project_rename,
            projects::project_add_folder,
            projects::project_relocate,
            projects::project_missing_folders,
            projects::project_pick_and_open,
            projects::project_branch,
            projects::settings_get,
            projects::settings_set,
            fs_tree::fs_list,
            fs_tree::fs_create,
            fs_tree::fs_rename,
            fs_tree::fs_trash,
            fs_tree::fs_read,
            fs_tree::fs_write,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { .. } = event {
            app.state::<projects::Quitting>().0.store(true, Ordering::SeqCst);
        }
    });
}
