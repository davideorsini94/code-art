mod claude;
/// Pubblici per i test di integrazione (`tests/security.rs`): confinamento dei percorsi, store, fiducia.
pub mod fs_tree;
mod lifecycle;
mod projects;
mod pty;
pub mod store;
pub mod trust;

use std::sync::Arc;
use tauri::{Manager, RunEvent, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // misura di avvio, attiva solo con CODEART_TIMING=1: ms dall'inizio di `run`
    let timing = std::env::var_os("CODEART_TIMING").is_some_and(|v| v == "1");
    let t0 = std::time::Instant::now();
    let app = tauri::Builder::default()
        .on_page_load(move |window, payload| {
            if timing && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                eprintln!("[timing] pagina caricata in {} ms ({})", t0.elapsed().as_millis(), window.label());
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(projects::Quitting::default())
        .manage(fs_tree::Watchers::default())
        .manage(pty::Ptys::default())
        .manage(trust::TrustPrompts::default())
        .manage(claude::sessions::SessionWatchers::default())
        .manage(claude::sessions::MetaCache::default())
        .manage(claude::delete::DeletePrompts::default())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let dir = app.path().home_dir()?.join("Library/Application Support/CodeArt");
            let store = store::Store::open(&dir.join("codeart.db"))?;
            let launch = claude::engine::app_launch();
            let life = Arc::new(lifecycle::Lifecycle::default());
            // ogni cambio di stato passa da qui: numero sul Dock e notifiche
            let sink: Arc<dyn claude::engine::EventSink> = Arc::new(lifecycle::LifecycleSink {
                inner: Arc::new(claude::engine::TauriSink(app.handle().clone())),
                app: app.handle().clone(),
                life: life.clone(),
                store: store.clone(),
            });
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
                std::sync::Arc::new(claude::titles::ClaudeRunner::new(launch)),
            );
            engine.set_titles(titles.clone());
            app.manage(engine);
            app.manage(titles);
            app.manage(store.clone());
            app.manage(life.clone());
            lifecycle::register_notification_clicks(app.handle(), life.clone());
            lifecycle::update_badge(app.handle(), &life, &store);
            projects::rebuild_menu(app.handle())?;
            projects::restore_windows(app.handle())?;
            if timing {
                eprintln!("[timing] finestra creata in {} ms", t0.elapsed().as_millis());
            }
            Ok(())
        })
        .on_menu_event(|app, event| projects::on_menu_event(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => lifecycle::on_close_requested(window, api),
                WindowEvent::Focused(true) => lifecycle::on_window_focused(window.app_handle(), window.label()),
                WindowEvent::Destroyed => projects::on_window_destroyed(window.app_handle(), window.label()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            projects::project_current,
            projects::project_rename,
            projects::project_pick_and_add_folder,
            projects::project_pick_relocate,
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
            fs_tree::fs_read_bytes,
            fs_tree::fs_stat,
            fs_tree::fs_write,
            fs_tree::fs_all_files,
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_kill,
            claude::sessions::conv_list,
            claude::sessions::conv_history,
            claude::sessions::conv_search,
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
            claude::commands::conv_delete,
            claude::commands::conv_save_image,
            claude::commands::conv_pending,
            claude::titles::conv_rename,
            claude::titles::conv_unlock_title,
            trust::trust_check,
            trust::trust_accept,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        match event {
            // senza codice = chiusa l'ultima finestra: l'app resta viva (i processi in sottofondo e il Dock);
            // con codice = uscita vera (`app.exit`)
            RunEvent::ExitRequested { code, api, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                } else {
                    projects::mark_quitting(app);
                }
            }
            RunEvent::Reopen { has_visible_windows, .. } => lifecycle::on_reopen(app, has_visible_windows),
            // i processi `claude` non sopravvivono all'app (la chiusura di una finestra invece non li ferma);
            // l'uscita dal Dock o dal sistema arriva qui senza `ExitRequested`
            RunEvent::Exit => {
                projects::mark_quitting(app);
                app.state::<claude::engine::Engine>().stop_all();
            }
            _ => {}
        }
    });
}
