pub mod domain;
pub mod kanban;
pub mod preview;
pub mod schema;
pub mod sqlite;

pub use domain::{
    AdmissionIntent, AdmissionStatus, MessageRecord, PersistenceError, PersistenceStore, SearchHit,
    SessionRecord, SessionSummary, ToolCallRecord, UsageTotals,
};
pub use kanban::{
    KanbanComment, KanbanStore, KanbanTask, TaskStatus, KANBAN_TERMINAL_PROVIDER_EXIT_CODE,
};
pub use preview::shape_preview;
pub use schema::{
    FTS_STORAGE_VERSION, FTS_TOOL_CONTENT_PREFIX_CHARS, MAX_FTS5_QUERY_CHARS, SCHEMA_VERSION,
};
pub use sqlite::{sanitize_fts5_query, SqliteStore};
