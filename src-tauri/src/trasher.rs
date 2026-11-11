//! Sposta un percorso nel Cestino di macOS. Su macOS usa `NSFileManager` e non Finder/AppleScript (che chiederebbe
//! il permesso di Automazione per Finder e può fallire). Punto unico, riusabile da altri comandi.
use std::path::Path;

pub fn move_to_trash(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path).map_err(|e| e.to_string())
    }
}
