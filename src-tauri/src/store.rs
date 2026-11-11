use rusqlite::{params, Connection, OptionalExtension};
use crate::claude::Conversation;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type Result<T> = std::result::Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// folders[0] = cartella principale
    pub folders: Vec<String>,
}

/// Clonabile: le copie condividono la stessa connessione (il motore la usa dai suoi thread).
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  folders_json TEXT NOT NULL,
  last_opened INTEGER NOT NULL DEFAULT 0,
  was_open INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS conversations (
  session_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  cwd TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  prev_title TEXT,
  title_changed_at INTEGER,
  title_locked INTEGER NOT NULL DEFAULT 0,
  summary TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'idle',
  owned INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  last_activity INTEGER NOT NULL DEFAULT 0,
  turn_count INTEGER NOT NULL DEFAULT 0,
  model TEXT,
  permission_mode TEXT NOT NULL DEFAULT '',
  context_used_pct REAL,
  title_turn INTEGER,
  seen_turn INTEGER
);
CREATE INDEX IF NOT EXISTS idx_conversations_project ON conversations(project_id);
CREATE TABLE IF NOT EXISTS trusted_folders (
  path TEXT PRIMARY KEY
);
";

/// Migrazioni dello schema, in ordine: la voce `i` porta il database alla versione `i + 1` (`PRAGMA user_version`).
/// La versione 1 è lo schema della tappa 1; un database creato prima del versionamento (versione 0, tabelle già
/// presenti) la riceve senza cambiamenti (`IF NOT EXISTS`). Uno schema nuovo si aggiunge in coda, mai modificando
/// le voci esistenti.
const MIGRATIONS: &[&str] = &[SCHEMA];

/// Applica le migrazioni mancanti, ciascuna in una transazione insieme al nuovo `user_version`. Un database di una
/// versione più recente (app tornata indietro) resta com'è.
fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).map_err(err)?.max(0) as usize;
    for (i, sql) in migrations.iter().enumerate().skip(current) {
        let tx = conn.transaction().map_err(err)?;
        tx.execute_batch(sql).map_err(err)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1)).map_err(err)?;
        tx.commit().map_err(err)?;
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const CONV_SELECT: &str = "SELECT session_id, project_id, cwd, title, prev_title, title_changed_at, title_locked, summary,
  status, owned, archived, last_activity, turn_count, model, permission_mode, context_used_pct FROM conversations";

fn row_to_conversation(r: &rusqlite::Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        session_id: r.get(0)?,
        project_id: r.get(1)?,
        cwd: r.get(2)?,
        title: r.get(3)?,
        prev_title: r.get(4)?,
        title_changed_at: r.get(5)?,
        title_locked: r.get(6)?,
        summary: r.get(7)?,
        status: r.get(8)?,
        owned: r.get(9)?,
        archived: r.get(10)?,
        last_activity: r.get(11)?,
        turn_count: r.get(12)?,
        model: r.get(13)?,
        permission_mode: r.get(14)?,
        context_used_pct: r.get(15)?,
        maybe_open_elsewhere: false,
    })
}

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<(String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

fn build_project(raw: (String, String, String)) -> Result<Project> {
    let folders: Vec<String> = serde_json::from_str(&raw.2).map_err(err)?;
    Ok(Project { id: raw.0, name: raw.1, folders })
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let mut conn = Connection::open(path).map_err(err)?;
        migrate(&mut conn, MIGRATIONS)?;
        conn.execute(
            "INSERT OR IGNORE INTO settings(key, value) VALUES ('default_permission_mode', 'bypassPermissions')",
            [],
        )
        .map_err(err)?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Inserisce o aggiorna nome e cartelle; non tocca `last_opened` né `was_open` di un progetto esistente.
    pub fn upsert_project(&self, p: &Project) -> Result<()> {
        let folders = serde_json::to_string(&p.folders).map_err(err)?;
        self.conn()
            .execute(
                "INSERT INTO projects(id, name, folders_json, last_opened, was_open) VALUES (?1, ?2, ?3, ?4, 0)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, folders_json = excluded.folders_json",
                params![p.id, p.name, folders, now_ms()],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let raw = self
            .conn()
            .query_row("SELECT id, name, folders_json FROM projects WHERE id = ?1", [id], row_to_project)
            .optional()
            .map_err(err)?;
        raw.map(build_project).transpose()
    }

    /// Più recenti per primi.
    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT id, name, folders_json FROM projects ORDER BY last_opened DESC, name")
            .map_err(err)?;
        let rows = stmt.query_map([], row_to_project).map_err(err)?;
        rows.map(|r| build_project(r.map_err(err)?)).collect()
    }

    /// Imposta il flag "aperto" e, se true, aggiorna `last_opened`.
    pub fn set_open_flag(&self, id: &str, open: bool) -> Result<()> {
        let conn = self.conn();
        if open {
            conn.execute(
                "UPDATE projects SET was_open = 1, last_opened = ?2 WHERE id = ?1",
                params![id, now_ms()],
            )
        } else {
            conn.execute("UPDATE projects SET was_open = 0 WHERE id = ?1", [id])
        }
        .map_err(err)?;
        Ok(())
    }

    pub fn open_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT id, name, folders_json FROM projects WHERE was_open = 1 ORDER BY last_opened")
            .map_err(err)?;
        let rows = stmt.query_map([], row_to_project).map_err(err)?;
        rows.map(|r| build_project(r.map_err(err)?)).collect()
    }

    /// Inserisce una sessione nuova (owned=false, idle, permessi dal default) oppure aggiorna SOLO
    /// cwd, last_activity e turn_count di una già nota: titolo, archiviazione, letto/non letto restano.
    /// Per le conversazioni pilotate da Code Art (`owned = 1`) `turn_count` (e lo stato) li scrive solo il motore,
    /// che conta ogni fine turno (anche i comandi slash): il conteggio dei prompt del file non lo sovrascrive.
    pub fn upsert_synced_conversation(
        &self,
        project_id: &str,
        session_id: &str,
        cwd: &str,
        title: &str,
        last_activity: i64,
        turn_count: i64,
    ) -> Result<()> {
        let conn = self.conn();
        let mode: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'default_permission_mode'", [], |r| r.get(0))
            .optional()
            .map_err(err)?
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO conversations(session_id, project_id, cwd, title, last_activity, turn_count, permission_mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(session_id) DO UPDATE SET
               cwd = excluded.cwd, last_activity = excluded.last_activity,
               turn_count = CASE WHEN conversations.owned = 1 THEN conversations.turn_count ELSE excluded.turn_count END",
            params![session_id, project_id, cwd, title, last_activity, turn_count, mode],
        )
        .map_err(err)?;
        Ok(())
    }

    pub fn get_conversation(&self, session_id: &str) -> Result<Option<Conversation>> {
        self.conn()
            .query_row(&format!("{CONV_SELECT} WHERE session_id = ?1"), [session_id], row_to_conversation)
            .optional()
            .map_err(err)
    }

    /// Più recenti per primi.
    pub fn list_conversations(&self, project_id: &str) -> Result<Vec<Conversation>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!("{CONV_SELECT} WHERE project_id = ?1 ORDER BY last_activity DESC, session_id"))
            .map_err(err)?;
        let rows = stmt.query_map([project_id], row_to_conversation).map_err(err)?;
        rows.map(|r| r.map_err(err)).collect()
    }

    /// Nuova conversazione avviata da Code Art (owned, idle).
    pub fn insert_owned_conversation(
        &self,
        project_id: &str,
        session_id: &str,
        cwd: &str,
        model: Option<&str>,
        permission_mode: &str,
    ) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO conversations(session_id, project_id, cwd, owned, last_activity, model, permission_mode, seen_turn)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, 0)",
                params![session_id, project_id, cwd, now_ms(), model, permission_mode],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Stato della macchina degli stati salvato: (status, turn_count, seen_turn).
    pub fn conversation_state(&self, session_id: &str) -> Result<Option<(String, i64, Option<i64>)>> {
        self.conn()
            .query_row(
                "SELECT status, turn_count, seen_turn FROM conversations WHERE session_id = ?1",
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(err)
    }

    /// Salva lo stato della macchina degli stati; `touch` aggiorna anche `last_activity`.
    pub fn save_conversation_state(&self, session_id: &str, status: &str, turn_count: i64, seen_turn: i64, touch: bool) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE conversations SET status = ?2, turn_count = ?3, seen_turn = ?4,
                   last_activity = CASE WHEN ?5 THEN ?6 ELSE last_activity END
                 WHERE session_id = ?1",
                params![session_id, status, turn_count, seen_turn, touch, now_ms()],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Imposta `owned = 1` e riporta a `waiting` uno stato `working` rimasto da un processo che non c'è più.
    pub fn mark_conversation_owned(&self, session_id: &str) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE conversations SET owned = 1, status = CASE status WHEN 'working' THEN 'waiting' ELSE status END
                 WHERE session_id = ?1",
                [session_id],
            )
            .map_err(err)?;
        Ok(())
    }

    /// All'avvio dell'app nessun processo è vivo: gli stati `working` salvati diventano `waiting`.
    /// Restituisce le sessioni il cui turno è rimasto a metà.
    pub fn reset_working_conversations(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let ids = {
            let mut stmt = conn.prepare("SELECT session_id FROM conversations WHERE status = 'working'").map_err(err)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?
        };
        conn.execute("UPDATE conversations SET status = 'waiting' WHERE status = 'working'", []).map_err(err)?;
        Ok(ids)
    }

    /// Conversazioni `waiting` non archiviate, di tutti i progetti (numero sul Dock).
    pub fn waiting_total(&self) -> Result<usize> {
        let n: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM conversations WHERE status = 'waiting' AND archived = 0", [], |r| r.get(0))
            .map_err(err)?;
        Ok(n.max(0) as usize)
    }

    pub fn set_conversation_model(&self, session_id: &str, model: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET model = ?2 WHERE session_id = ?1", params![session_id, model])
            .map_err(err)?;
        Ok(())
    }

    pub fn set_conversation_permission_mode(&self, session_id: &str, mode: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET permission_mode = ?2 WHERE session_id = ?1", params![session_id, mode])
            .map_err(err)?;
        Ok(())
    }

    pub fn set_conversation_context(&self, session_id: &str, pct: f64) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET context_used_pct = ?2 WHERE session_id = ?1", params![session_id, pct])
            .map_err(err)?;
        Ok(())
    }

    pub fn delete_conversation(&self, session_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM conversations WHERE session_id = ?1", [session_id]).map_err(err)?;
        Ok(())
    }

    pub fn set_conversation_archived(&self, session_id: &str, archived: bool) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET archived = ?2 WHERE session_id = ?1", params![session_id, archived])
            .map_err(err)?;
        Ok(())
    }

    pub fn title_turn(&self, session_id: &str) -> Result<Option<i64>> {
        let v: Option<Option<i64>> = self
            .conn()
            .query_row("SELECT title_turn FROM conversations WHERE session_id = ?1", [session_id], |r| r.get(0))
            .optional()
            .map_err(err)?;
        Ok(v.flatten())
    }

    /// `title_turn` di ogni conversazione del progetto (`None` = mai titolata).
    pub fn title_turns(&self, project_id: &str) -> Result<std::collections::HashMap<String, Option<i64>>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT session_id, title_turn FROM conversations WHERE project_id = ?1").map_err(err)?;
        let rows = stmt.query_map([project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))).map_err(err)?;
        rows.map(|r| r.map_err(err)).collect()
    }

    /// Salva titolo e riassunto generati, solo se nel frattempo la conversazione non è stata bloccata
    /// (titolo scelto dall'utente) né archiviata. Restituisce true se la riga è stata aggiornata.
    pub fn apply_title_result(&self, c: &Conversation, title_turn: i64) -> Result<bool> {
        let n = self
            .conn()
            .execute(
                "UPDATE conversations SET title = ?2, prev_title = ?3, title_changed_at = ?4, summary = ?5, title_turn = ?6
                 WHERE session_id = ?1 AND title_locked = 0 AND archived = 0",
                params![c.session_id, c.title, c.prev_title, c.title_changed_at, c.summary, title_turn],
            )
            .map_err(err)?;
        Ok(n > 0)
    }

    /// Titolo scelto dall'utente: blocca la titolazione automatica.
    pub fn rename_conversation(&self, session_id: &str, title: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET title = ?2, title_locked = 1 WHERE session_id = ?1", params![session_id, title])
            .map_err(err)?;
        Ok(())
    }

    pub fn unlock_conversation_title(&self, session_id: &str) -> Result<()> {
        self.conn().execute("UPDATE conversations SET title_locked = 0 WHERE session_id = ?1", [session_id]).map_err(err)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn exec_for_test(&self, sql: &str, arg: &str) {
        self.conn().execute(sql, [arg]).unwrap();
    }

    pub fn folder_trusted(&self, path: &str) -> Result<bool> {
        self.conn()
            .query_row("SELECT 1 FROM trusted_folders WHERE path = ?1", [path], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
            .map_err(err)
    }

    pub fn trust_folder(&self, path: &str) -> Result<()> {
        self.conn().execute("INSERT OR IGNORE INTO trusted_folders(path) VALUES (?1)", [path]).map_err(err)?;
        Ok(())
    }

    pub fn setting_get(&self, key: &str) -> Result<Option<String>> {
        self.conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(err)
    }

    pub fn setting_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_version(path: &Path) -> i64 {
        Connection::open(path).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn schema_is_versioned_and_unversioned_databases_are_adopted() {
        let dir = tempfile::tempdir().unwrap();
        // database nuovo: versione 1
        let fresh = dir.path().join("nuovo.db");
        Store::open(&fresh).unwrap();
        assert_eq!(user_version(&fresh), 1);
        // database creato prima del versionamento: tabelle e dati restano, versione 1
        let old = dir.path().join("vecchio.db");
        {
            let c = Connection::open(&old).unwrap();
            c.execute_batch(SCHEMA).unwrap();
            c.execute("INSERT INTO settings(key, value) VALUES ('default_permission_mode', 'plan')", []).unwrap();
            assert_eq!(user_version(&old), 0);
        }
        let s = Store::open(&old).unwrap();
        assert_eq!(s.setting_get("default_permission_mode").unwrap().as_deref(), Some("plan"));
        assert_eq!(user_version(&old), 1);
    }

    #[test]
    fn migrations_apply_only_the_missing_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let mut c = Connection::open(&path).unwrap();
        migrate(&mut c, &[SCHEMA]).unwrap();
        // la seconda migrazione fallirebbe se la prima venisse rieseguita senza IF NOT EXISTS
        let v2 = "CREATE TABLE extra (x INTEGER); INSERT INTO extra VALUES (1);";
        migrate(&mut c, &[SCHEMA, v2]).unwrap();
        migrate(&mut c, &[SCHEMA, v2]).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM extra", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "ogni passo una volta sola");
        assert_eq!(user_version(&path), 2);
        // un passo che fallisce non lascia mezza migrazione né sposta la versione
        assert!(migrate(&mut c, &[SCHEMA, v2, "CREATE TABLE y (a); SELECT * FROM nessuna;"]).is_err());
        assert_eq!(user_version(&path), 2);
        let y: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'y'", [], |r| r.get(0)).unwrap();
        assert_eq!(y, 0);
    }

    #[test]
    fn default_permission_mode_seeded_and_settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.setting_get("default_permission_mode").unwrap().as_deref(), Some("bypassPermissions"));
        store.setting_set("default_permission_mode", "plan").unwrap();
        assert_eq!(store.setting_get("default_permission_mode").unwrap().as_deref(), Some("plan"));
        assert_eq!(store.setting_get("nope").unwrap(), None);
    }

    #[test]
    fn sync_does_not_touch_the_turn_count_of_owned_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        store.insert_owned_conversation("p", "own", "/Users/dev/x", None, "default").unwrap();
        store.save_conversation_state("own", "waiting", 5, 4, false).unwrap();
        store.upsert_synced_conversation("p", "own", "/Users/dev/x", "t", 10, 2).unwrap();
        assert_eq!(store.conversation_state("own").unwrap(), Some(("waiting".into(), 5, Some(4))));
        // importata: il conteggio viene dal file
        store.upsert_synced_conversation("p", "imp", "/Users/dev/x", "t", 10, 2).unwrap();
        store.upsert_synced_conversation("p", "imp", "/Users/dev/x", "t", 11, 3).unwrap();
        assert_eq!(store.get_conversation("imp").unwrap().unwrap().turn_count, 3);
    }

    #[test]
    fn reopening_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let p = Project { id: "a".into(), name: "A".into(), folders: vec!["/x".into()] };
        Store::open(&path).unwrap().upsert_project(&p).unwrap();
        assert_eq!(Store::open(&path).unwrap().get_project("a").unwrap(), Some(p));
    }
}
