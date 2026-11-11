mod claude;
mod fs_tree;
mod projects;
mod pty;
mod store;

use tauri::{Manager, RunEvent, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(projects::Quitting::default())
        .manage(fs_tree::Watchers::default())
        .manage(pty::Ptys::default())
        .manage(claude::sessions::SessionWatchers::default())
        .manage(claude::sessions::MetaCache::default())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().home_dir()?.join("Library/Application Support/CodeArt");
            let store = store::Store::open(&dir.join("codeart.db"))?;
            let launch = claude::engine::app_launch();
            let sink: std::sync::Arc<dyn claude::engine::EventSink> = std::sync::Arc::new(claude::engine::TauriSink(app.handle().clone()));
            let engine = claude::engine::Engine::new(
                launch.clone(),
                store.clone(),
                sink.clone(),
                claude::sessions::default_root(),
                claude::engine::image_dir(),
            );
            let titles = claude::titles::TitleQueue::new(
                store.clone(),
                sink,
                claude::sessions::default_root(),
                std::sync::Arc::new(claude::titles::ClaudeRunner { launch }),
            );
            engine.set_titles(titles.clone());
            app.manage(engine);
            app.manage(titles);
            app.manage(store);
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
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_kill,
            claude::sessions::conv_list,
            claude::sessions::conv_history,
            claude::commands::claude_check,
            claude::commands::conv_start,
            claude::commands::conv_resume,
            claude::commands::conv_send,
            claude::commands::conv_answer,
            claude::commands::conv_interrupt,
            claude::commands::conv_set_model,
            claude::commands::conv_set_permission_mode,
            claude::commands::conv_mark_viewed,
            claude::commands::conv_archive,
            claude::commands::conv_running,
            claude::commands::conv_stop,
            claude::titles::conv_rename,
            claude::titles::conv_unlock_title,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        match event {
            RunEvent::ExitRequested { .. } => projects::mark_quitting(app),
            // i processi `claude` non sopravvivono all'app (la chiusura di una finestra invece non li ferma)
            RunEvent::Exit => app.state::<claude::engine::Engine>().stop_all(),
            _ => {}
        }
    });
}
