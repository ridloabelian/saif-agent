use crate::domain::{
    MessageRecord, PersistenceError, PersistenceStore, SearchHit, SessionRecord, SessionSummary,
    ToolCallRecord, UsageTotals,
};
use crate::preview::shape_preview;
use crate::schema::{
    FTS_SQL, FTS_STORAGE_VERSION, MAX_FTS5_QUERY_CHARS, SCHEMA_SQL, SCHEMA_VERSION,
};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FTS_OPERATORS: &[&str] = &["AND", "OR", "NOT"];

fn now_epoch_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Sanitize arbitrary user search queries into safe FTS5 MATCH expressions.
pub fn sanitize_fts5_query(raw_query: &str) -> String {
    let bounded: String = raw_query.chars().take(MAX_FTS5_QUERY_CHARS).collect();
    let tokens: Vec<String> = bounded
        .split_whitespace()
        .filter(|tok| !tok.is_empty())
        .map(|tok| {
            let upper = tok.to_ascii_uppercase();
            if FTS_OPERATORS.contains(&upper.as_str()) {
                upper
            } else {
                let escaped = tok.replace('"', "\"\"");
                format!("\"{escaped}\"")
            }
        })
        .collect();
    tokens.join(" ")
}

#[derive(Clone)]
pub struct SqliteStore {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open(db_path: &Path) -> Result<Self, PersistenceError> {
        Self::open_with_journal_mode(db_path, "wal")
    }

    pub fn open_with_journal_mode(
        db_path: &Path,
        journal_mode: &str,
    ) -> Result<Self, PersistenceError> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        let mode = if journal_mode.eq_ignore_ascii_case("delete") {
            "DELETE"
        } else {
            "WAL"
        };
        conn.execute_batch(&format!(
            "PRAGMA foreign_keys = ON;\nPRAGMA journal_mode = {mode};\nPRAGMA synchronous = NORMAL;"
        ))?;
        conn.execute_batch(SCHEMA_SQL)?;
        conn.execute_batch(FTS_SQL)?;

        let ver: Option<i64> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        if ver.is_none() {
            conn.execute(
                "INSERT INTO schema_version(version) VALUES (?1)",
                params![SCHEMA_VERSION],
            )?;
        }
        conn.execute(
            "INSERT OR IGNORE INTO state_meta(key, value) VALUES ('fts_storage_version', ?1)",
            params![FTS_STORAGE_VERSION.to_string()],
        )?;

        Ok(Self {
            db_path: db_path.to_path_buf(),
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn count_sessions(&self) -> Result<i64, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        Ok(n)
    }

    pub fn deactivate_session_messages_from_id(
        &self,
        session_id: &str,
        from_message_id: i64,
    ) -> Result<usize, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let changed = conn.execute(
            "UPDATE messages SET active = 0, compacted = 1 WHERE session_id = ?1 AND id <= ?2 AND active = 1",
            params![session_id, from_message_id],
        )?;
        Ok(changed)
    }

    pub fn mark_messages_compacted(
        &self,
        session_id: &str,
        message_ids: &[i64],
    ) -> Result<(), PersistenceError> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let tx = conn.transaction()?;
        for &id in message_ids {
            tx.execute(
                "UPDATE messages SET active = 0, compacted = 1 WHERE session_id = ?1 AND id = ?2",
                params![session_id, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: &str,
        title_source: &str,
    ) -> Result<(), PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET title = ?1, title_source = ?2 WHERE id = ?3",
            params![title, title_source, session_id],
        )?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
#[async_trait]
impl PersistenceStore for SqliteStore {
    async fn create_session(
        &self,
        session_id: &str,
        source: &str,
        model: Option<&str>,
        system_prompt: Option<&str>,
        cwd: Option<&str>,
        profile_name: Option<&str>,
    ) -> Result<String, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let now = now_epoch_secs();
        let prompt_hash = if let Some(prompt) = system_prompt {
            let mut hasher = Sha256::new();
            hasher.update(prompt.as_bytes());
            let hash = hex::encode(hasher.finalize());
            conn.execute(
                "INSERT OR IGNORE INTO system_prompts(hash, prompt) VALUES (?1, ?2)",
                params![hash, prompt],
            )?;
            Some(hash)
        } else {
            None
        };

        conn.execute(
            "INSERT OR IGNORE INTO sessions(id, source, created_source, model, system_prompt, system_prompt_hash, started_at, last_activity_at, cwd, profile_name)
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
            params![session_id, source, model, system_prompt, prompt_hash, now, cwd, profile_name],
        )?;
        Ok(session_id.to_string())
    }

    async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionRecord>, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let row = conn
            .query_row(
                "SELECT id, source, created_source, session_key, model, system_prompt, system_prompt_hash,
                        parent_session_id, started_at, ended_at, end_reason, message_count, tool_call_count,
                        input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                        cwd, title, title_source, last_activity_at, profile_name, transport_profile, archived
                 FROM sessions WHERE id = ?1",
                params![session_id],
                |r| {
                    Ok(SessionRecord {
                        id: r.get(0)?,
                        source: r.get(1)?,
                        created_source: r.get(2)?,
                        session_key: r.get(3)?,
                        model: r.get(4)?,
                        system_prompt: r.get(5)?,
                        system_prompt_hash: r.get(6)?,
                        parent_session_id: r.get(7)?,
                        started_at: r.get(8)?,
                        ended_at: r.get(9)?,
                        end_reason: r.get(10)?,
                        message_count: r.get::<_, Option<i64>>(11)?.unwrap_or(0),
                        tool_call_count: r.get::<_, Option<i64>>(12)?.unwrap_or(0),
                        input_tokens: r.get::<_, Option<i64>>(13)?.unwrap_or(0),
                        output_tokens: r.get::<_, Option<i64>>(14)?.unwrap_or(0),
                        cache_read_tokens: r.get::<_, Option<i64>>(15)?.unwrap_or(0),
                        cache_write_tokens: r.get::<_, Option<i64>>(16)?.unwrap_or(0),
                        reasoning_tokens: r.get::<_, Option<i64>>(17)?.unwrap_or(0),
                        cwd: r.get(18)?,
                        title: r.get(19)?,
                        title_source: r.get(20)?,
                        last_activity_at: r.get(21)?,
                        profile_name: r.get(22)?,
                        transport_profile: r.get(23)?,
                        archived: r.get::<_, i64>(24).unwrap_or(0) != 0,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

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
    ) -> Result<i64, PersistenceError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let now = now_epoch_secs();
        let tool_calls_json = tool_calls.map(serde_json::to_string).transpose()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO messages(session_id, role, content, tool_name, tool_call_id, tool_calls, timestamp, finish_reason, reasoning, display_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                session_id,
                role,
                content,
                tool_name,
                tool_call_id,
                tool_calls_json,
                now,
                finish_reason,
                reasoning,
                display_kind
            ],
        )?;
        let row_id = tx.last_insert_rowid();
        let added_tool_calls = tool_calls.map(|t| t.len() as i64).unwrap_or(0);
        tx.execute(
            "UPDATE sessions
             SET message_count = COALESCE(message_count, 0) + 1,
                 tool_call_count = COALESCE(tool_call_count, 0) + ?1,
                 last_activity_at = ?2
             WHERE id = ?3",
            params![added_tool_calls, now, session_id],
        )?;
        tx.commit()?;
        Ok(row_id)
    }

    async fn load_active_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<MessageRecord>, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, tool_call_id, tool_calls, tool_name, timestamp,
                    finish_reason, reasoning, active, compacted, display_kind
             FROM messages
             WHERE session_id = ?1 AND active = 1
             ORDER BY timestamp ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            let tc_raw: Option<String> = r.get(5)?;
            let tool_calls =
                tc_raw.and_then(|s| serde_json::from_str::<Vec<ToolCallRecord>>(&s).ok());
            Ok(MessageRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                content: r.get(3)?,
                tool_call_id: r.get(4)?,
                tool_calls,
                tool_name: r.get(6)?,
                timestamp: r.get(7)?,
                finish_reason: r.get(8)?,
                reasoning: r.get(9)?,
                active: r.get::<_, i64>(10)? != 0,
                compacted: r.get::<_, i64>(11)? != 0,
                display_kind: r.get(12)?,
            })
        })?;
        let mut out = Vec::new();
        for item in rows {
            out.push(item?);
        }
        Ok(out)
    }

    async fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>, PersistenceError> {
        let match_expr = sanitize_fts5_query(query);
        if match_expr.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.session_id, m.role,
                    snippet(messages_fts, 0, '[', ']', '...', 18) AS snip,
                    m.timestamp, m.tool_name, s.source, s.model
             FROM messages_fts
             JOIN messages m ON m.id = messages_fts.rowid
             JOIN sessions s ON s.id = m.session_id
             WHERE messages_fts MATCH ?1
               AND (m.active = 1 OR m.compacted = 1)
               AND COALESCE(m.display_kind, '') <> 'hidden'
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![match_expr, limit as i64], |r| {
            Ok(SearchHit {
                message_id: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                snippet: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                timestamp: r.get(4)?,
                tool_name: r.get(5)?,
                source: r.get(6)?,
                model: r.get(7)?,
            })
        })?;
        let mut hits = Vec::new();
        for item in rows {
            hits.push(item?);
        }
        Ok(hits)
    }

    async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.source, s.model, s.title,
                    COALESCE((SELECT m.content FROM messages m
                              WHERE m.session_id = s.id AND m.role = 'user' AND m.content IS NOT NULL
                                AND COALESCE(m.display_kind, '') <> 'hidden'
                              ORDER BY m.timestamp ASC, m.id ASC LIMIT 1), '') AS first_user,
                    COALESCE(s.message_count, 0), s.started_at, s.last_activity_at, s.archived
             FROM sessions s
             WHERE s.hidden = 0
             ORDER BY COALESCE(s.last_activity_at, s.started_at) DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let first_user: String = r.get(4)?;
            Ok(SessionSummary {
                id: r.get(0)?,
                source: r.get(1)?,
                model: r.get(2)?,
                title: r.get(3)?,
                preview: shape_preview(&first_user),
                message_count: r.get(5)?,
                started_at: r.get(6)?,
                last_activity_at: r.get(7)?,
                archived: r.get::<_, i64>(8).unwrap_or(0) != 0,
            })
        })?;
        let mut out = Vec::new();
        for item in rows {
            out.push(item?);
        }
        Ok(out)
    }

    async fn record_usage(
        &self,
        session_id: &str,
        model: &str,
        usage: &UsageTotals,
    ) -> Result<(), PersistenceError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let now = now_epoch_secs();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sessions
             SET input_tokens = COALESCE(input_tokens, 0) + ?1,
                 output_tokens = COALESCE(output_tokens, 0) + ?2,
                 cache_read_tokens = COALESCE(cache_read_tokens, 0) + ?3,
                 cache_write_tokens = COALESCE(cache_write_tokens, 0) + ?4,
                 reasoning_tokens = COALESCE(reasoning_tokens, 0) + ?5,
                 api_call_count = COALESCE(api_call_count, 0) + ?6
             WHERE id = ?7",
            params![
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
                usage.reasoning_tokens,
                usage.api_calls,
                session_id
            ],
        )?;
        tx.execute(
            "INSERT INTO session_model_usage(
                session_id, model, billing_provider, billing_base_url, billing_mode, task,
                api_call_count, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                first_seen, last_seen
             ) VALUES (?1, ?2, '', '', '', '', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(session_id, model, billing_provider, billing_base_url, billing_mode, task) DO UPDATE SET
                api_call_count = api_call_count + excluded.api_call_count,
                input_tokens = input_tokens + excluded.input_tokens,
                output_tokens = output_tokens + excluded.output_tokens,
                cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                reasoning_tokens = reasoning_tokens + excluded.reasoning_tokens,
                last_seen = excluded.last_seen",
            params![
                session_id,
                model,
                usage.api_calls,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
                usage.reasoning_tokens,
                now
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}
