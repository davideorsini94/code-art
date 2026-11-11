use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

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

pub struct Store {
    conn: Mutex<Connection>,
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
        Ok(Store { conn: Mutex::new(conn) })
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
