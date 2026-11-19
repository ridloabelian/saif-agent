use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("invalid state transition: {0}")]
    InvalidTransition(String),
    #[error("storage unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub source: String,
    pub created_source: Option<String>,
    pub session_key: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub system_prompt_hash: Option<String>,
    pub parent_session_id: Option<String>,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub end_reason: Option<String>,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub title_source: Option<String>,
    pub last_activity_at: Option<f64>,
    pub profile_name: Option<String>,
    pub transport_profile: Option<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub source: String,
    pub model: Option<String>,
    pub title: Option<String>,
    pub preview: String,
    pub message_count: i64,
    pub started_at: f64,
    pub last_activity_at: Option<f64>,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Option<Vec<ToolCallRecord>>,
    pub tool_name: Option<String>,
    pub timestamp: f64,
    pub finish_reason: Option<String>,
    pub reasoning: Option<String>,
    pub active: bool,
    pub compacted: bool,
    pub display_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub session_id: String,
    pub role: String,
    pub snippet: String,
    pub timestamp: f64,
    pub tool_name: Option<String>,
    pub source: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub api_calls: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionIntent {
    Queue,
    Steer,
    Redirect,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionStatus {
    Queued,
    Started,
    Unknown,
    Terminal,
}

#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait PersistenceStore: Send + Sync {
    async fn create_session(
        &self,
        session_id: &str,
        source: &str,
        model: Option<&str>,
        system_prompt: Option<&str>,
        cwd: Option<&str>,
        profile_name: Option<&str>,
    ) -> Result<String, PersistenceError>;

    async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionRecord>, PersistenceError>;

    async fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: Option<&str>,
        tool_name: Option<&str>,
        tool_call_id: Option<&str>,
        tool_calls: Option<&[ToolCallRecord]>,
        reasoning: Option<&str>,
        finish_reason: Option<&str>,
        display_kind: Option<&str>,
    ) -> Result<i64, PersistenceError>;

    async fn load_active_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<MessageRecord>, PersistenceError>;

    async fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>, PersistenceError>;

    async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, PersistenceError>;

    async fn record_usage(
        &self,
        session_id: &str,
        model: &str,
        usage: &UsageTotals,
    ) -> Result<(), PersistenceError>;
}
