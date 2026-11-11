//! Integrazione con Claude Code: tipi condivisi con l'interfaccia e lettura delle sessioni su disco.
use serde::{Deserialize, Serialize};

pub mod commands;
pub mod delete;
pub mod engine;
pub mod locate;
pub mod protocol;
pub mod sessions;
pub mod state;
pub mod titles;
pub mod trasher;

/// Stato di una conversazione mostrato nell'interfaccia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConvStatus {
    Working,
    Waiting,
    #[default]
    Idle,
}

impl ConvStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ConvStatus::Working => "working",
            ConvStatus::Waiting => "waiting",
            ConvStatus::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub session_id: String,
    pub project_id: String,
    pub cwd: String,
    pub title: String,
    pub prev_title: Option<String>,
    pub title_changed_at: Option<i64>,
    pub title_locked: bool,
    pub summary: String,
    /// 'working' | 'waiting' | 'idle'
    pub status: String,
    pub owned: bool,
    pub archived: bool,
    pub last_activity: i64,
    pub turn_count: i64,
    pub model: Option<String>,
    pub permission_mode: String,
    pub context_used_pct: Option<f64>,
    /// Calcolato (non salvato): sessione non pilotata da noi, scritta da meno di 60 s.
    pub maybe_open_elsewhere: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ChatItem {
    #[serde(rename_all = "camelCase")]
    User { id: String, text: String, images: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Assistant { id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Thinking { id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Tool {
        id: String,
        name: String,
        input: serde_json::Value,
        result: Option<String>,
        ok: Option<bool>,
        parent_id: Option<String>,
        file_path: Option<String>,
    },
    /// Prodotto dal motore in streaming (task successivi); la lettura da disco non lo genera.
    #[allow(dead_code)]
    #[serde(rename_all = "camelCase")]
    Ask {
        id: String,
        request_id: String,
        /// 'permission' | 'question' | 'plan'
        ask_type: String,
        prompt: String,
        options: Vec<String>,
        answered: bool,
    },
    #[serde(rename_all = "camelCase")]
    Raw { id: String, label: String, json: String },
}
