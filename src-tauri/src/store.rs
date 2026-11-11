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
";

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
        let conn = Connection::open(path).map_err(err)?;
        conn.execute_batch(SCHEMA).map_err(err)?;
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
               cwd = excluded.cwd, last_activity = excluded.last_activity, turn_count = excluded.turn_count",
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
    pub fn reset_working_conversations(&self) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET status = 'waiting' WHERE status = 'working'", [])
            .map_err(err)?;
        Ok(())
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

    pub fn set_conversation_archived(&self, session_id: &str, archived: bool) -> Result<()> {
        self.conn()
            .execute("UPDATE conversations SET archived = ?2 WHERE session_id = ?1", params![session_id, archived])
            .map_err(err)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn exec_for_test(&self, sql: &str, arg: &str) {
        self.conn().execute(sql, [arg]).unwrap();
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
    fn reopening_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let p = Project { id: "a".into(), name: "A".into(), folders: vec!["/x".into()] };
        Store::open(&path).unwrap().upsert_project(&p).unwrap();
        assert_eq!(Store::open(&path).unwrap().get_project("a").unwrap(), Some(p));
    }
}
