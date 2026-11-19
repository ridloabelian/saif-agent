use crate::domain::PersistenceError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Exit code `78` signals unrecoverable provider/auth failure -> sticky auto-block on attempt 1.
pub const KANBAN_TERMINAL_PROVIDER_EXIT_CODE: i32 = 78;

const KANBAN_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
    id                   TEXT PRIMARY KEY,
    title                TEXT NOT NULL,
    body                 TEXT,
    assignee             TEXT,
    status               TEXT NOT NULL,
    priority             INTEGER DEFAULT 0,
    created_by           TEXT,
    created_at           INTEGER NOT NULL,
    started_at           INTEGER,
    completed_at         INTEGER,
    workspace_kind       TEXT NOT NULL DEFAULT 'scratch',
    workspace_path       TEXT,
    branch_name          TEXT,
    project_id           TEXT,
    claim_lock           TEXT,
    claim_expires        INTEGER,
    tenant               TEXT,
    result               TEXT,
    idempotency_key      TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    worker_pid           INTEGER,
    worker_started_at    INTEGER,
    last_failure_error   TEXT,
    max_runtime_seconds  INTEGER,
    last_heartbeat_at    INTEGER,
    current_run_id       INTEGER
);

CREATE TABLE IF NOT EXISTS task_links (
    parent_id TEXT NOT NULL,
    child_id  TEXT NOT NULL,
    PRIMARY KEY (parent_id, child_id)
);

CREATE TABLE IF NOT EXISTS task_comments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    author     TEXT NOT NULL,
    body       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS task_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload    TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS task_runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     TEXT NOT NULL,
    worker_pid  INTEGER,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    exit_code   INTEGER,
    outcome     TEXT
);

CREATE TABLE IF NOT EXISTS task_attachments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    name       TEXT NOT NULL,
    path_or_url TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS kanban_notify_subs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    profile    TEXT NOT NULL,
    target     TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Ready,
    Running,
    Review,
    Blocked,
    Completed,
    Archived,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Review => "review",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Archived => "archived",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "ready" => Self::Ready,
            "running" => Self::Running,
            "review" => Self::Review,
            "blocked" => Self::Blocked,
            "completed" => Self::Completed,
            "archived" => Self::Archived,
            _ => Self::Ready,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanTask {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub assignee: Option<String>,
    pub status: TaskStatus,
    pub priority: i64,
    pub created_at: i64,
    pub result: Option<String>,
    pub consecutive_failures: i64,
    pub worker_pid: Option<i64>,
    pub worker_started_at: Option<i64>,
    pub last_failure_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanComment {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

fn now_epoch_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Clone)]
pub struct KanbanStore {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl KanbanStore {
    pub fn open(db_path: &Path) -> Result<Self, PersistenceError> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.execute_batch("PRAGMA journal_mode = WAL;\nPRAGMA synchronous = NORMAL;")?;
        conn.execute_batch(KANBAN_SCHEMA_SQL)?;
        Ok(Self {
            db_path: db_path.to_path_buf(),
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn create_task(
        &self,
        title: &str,
        body: Option<&str>,
        assignee: Option<&str>,
        priority: i64,
        parent_ids: &[String],
    ) -> Result<KanbanTask, PersistenceError> {
        let id = format!("task-{}", &Uuid::new_v4().to_string()[..8]);
        let now = now_epoch_i64();
        let initial_status = if parent_ids.is_empty() {
            TaskStatus::Ready
        } else {
            TaskStatus::Blocked
        };
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO tasks(id, title, body, assignee, status, priority, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                title,
                body,
                assignee,
                initial_status.as_str(),
                priority,
                now
            ],
        )?;
        for parent in parent_ids {
            tx.execute(
                "INSERT OR IGNORE INTO task_links(parent_id, child_id) VALUES (?1, ?2)",
                params![parent, id],
            )?;
        }
        tx.execute(
            "INSERT INTO task_events(task_id, event_type, payload, created_at) VALUES (?1, 'created', ?2, ?3)",
            params![id, title, now],
        )?;
        tx.commit()?;
        Ok(KanbanTask {
            id,
            title: title.to_string(),
            body: body.map(ToString::to_string),
            assignee: assignee.map(ToString::to_string),
            status: initial_status,
            priority,
            created_at: now,
            result: None,
            consecutive_failures: 0,
            worker_pid: None,
            worker_started_at: None,
            last_failure_error: None,
        })
    }

    pub fn list_tasks(
        &self,
        status: Option<TaskStatus>,
    ) -> Result<Vec<KanbanTask>, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let sql = if status.is_some() {
            "SELECT id, title, body, assignee, status, priority, created_at, result,
                    consecutive_failures, worker_pid, worker_started_at, last_failure_error
             FROM tasks WHERE status = ?1 ORDER BY priority DESC, created_at ASC"
        } else {
            "SELECT id, title, body, assignee, status, priority, created_at, result,
                    consecutive_failures, worker_pid, worker_started_at, last_failure_error
             FROM tasks ORDER BY priority DESC, created_at ASC"
        };
        let mut stmt = conn.prepare(sql)?;
        let map_row = |r: &rusqlite::Row| -> rusqlite::Result<KanbanTask> {
            let status_s: String = r.get(4)?;
            Ok(KanbanTask {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                assignee: r.get(3)?,
                status: TaskStatus::parse(&status_s),
                priority: r.get(5)?,
                created_at: r.get(6)?,
                result: r.get(7)?,
                consecutive_failures: r.get(8)?,
                worker_pid: r.get(9)?,
                worker_started_at: r.get(10)?,
                last_failure_error: r.get(11)?,
            })
        };
        let mut out = Vec::new();
        if let Some(st) = status {
            let rows = stmt.query_map(params![st.as_str()], map_row)?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let rows = stmt.query_map([], map_row)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn show_task(&self, task_id: &str) -> Result<Option<KanbanTask>, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        conn.query_row(
            "SELECT id, title, body, assignee, status, priority, created_at, result,
                    consecutive_failures, worker_pid, worker_started_at, last_failure_error
             FROM tasks WHERE id = ?1",
            params![task_id],
            |r| {
                let status_s: String = r.get(4)?;
                Ok(KanbanTask {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    body: r.get(2)?,
                    assignee: r.get(3)?,
                    status: TaskStatus::parse(&status_s),
                    priority: r.get(5)?,
                    created_at: r.get(6)?,
                    result: r.get(7)?,
                    consecutive_failures: r.get(8)?,
                    worker_pid: r.get(9)?,
                    worker_started_at: r.get(10)?,
                    last_failure_error: r.get(11)?,
                })
            },
        )
        .optional()
        .map_err(PersistenceError::from)
    }

    /// Atomic CAS claim with `(worker_pid, worker_started_at)` anti-PID-reuse fingerprint.
    pub fn claim_next_ready_task(
        &self,
        assignee: Option<&str>,
        worker_pid: u32,
        worker_started_at: Option<u64>,
    ) -> Result<Option<KanbanTask>, PersistenceError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let tx = conn.transaction()?;
        let candidate_id: Option<String> = if let Some(who) = assignee {
            tx.query_row(
                "SELECT id FROM tasks WHERE status = 'ready' AND assignee = ?1
                 ORDER BY priority DESC, created_at ASC LIMIT 1",
                params![who],
                |r| r.get(0),
            )
            .optional()?
        } else {
            tx.query_row(
                "SELECT id FROM tasks WHERE status = 'ready'
                 ORDER BY priority DESC, created_at ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
        };
        let Some(id) = candidate_id else {
            return Ok(None);
        };
        let now = now_epoch_i64();
        let claim_token = Uuid::new_v4().to_string();
        let updated = tx.execute(
            "UPDATE tasks
             SET status = 'running', started_at = ?1, claim_lock = ?2, worker_pid = ?3, worker_started_at = ?4, last_heartbeat_at = ?1
             WHERE id = ?5 AND status = 'ready'",
            params![
                now,
                claim_token,
                worker_pid as i64,
                worker_started_at.map(|v| v as i64),
                id
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        tx.commit()?;
        drop(conn);
        self.show_task(&id)
    }

    pub fn complete_task(&self, task_id: &str, result: &str) -> Result<(), PersistenceError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let now = now_epoch_i64();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'completed', completed_at = ?1, result = ?2, consecutive_failures = 0
             WHERE id = ?3",
            params![now, result, task_id],
        )?;
        if changed == 0 {
            return Err(PersistenceError::TaskNotFound(task_id.to_string()));
        }
        // Promote downstream tasks whose parents are all completed.
        tx.execute(
            "UPDATE tasks SET status = 'ready'
             WHERE status = 'blocked'
               AND id IN (SELECT child_id FROM task_links WHERE parent_id = ?1)
               AND NOT EXISTS (
                   SELECT 1 FROM task_links tl
                   JOIN tasks p ON p.id = tl.parent_id
                   WHERE tl.child_id = tasks.id AND p.status <> 'completed'
               )",
            params![task_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn record_failure(
        &self,
        task_id: &str,
        error: &str,
        exit_code: Option<i32>,
        failure_limit: i64,
    ) -> Result<TaskStatus, PersistenceError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let tx = conn.transaction()?;
        let prev_failures: i64 = tx
            .query_row(
                "SELECT consecutive_failures FROM tasks WHERE id = ?1",
                params![task_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| PersistenceError::TaskNotFound(task_id.to_string()))?;
        let next_failures = prev_failures + 1;
        let sticky_block =
            exit_code == Some(KANBAN_TERMINAL_PROVIDER_EXIT_CODE) || next_failures >= failure_limit;
        let new_status = if sticky_block {
            TaskStatus::Blocked
        } else {
            TaskStatus::Ready
        };
        tx.execute(
            "UPDATE tasks SET status = ?1, consecutive_failures = ?2, last_failure_error = ?3,
                    worker_pid = NULL, worker_started_at = NULL, claim_lock = NULL
             WHERE id = ?4",
            params![new_status.as_str(), next_failures, error, task_id],
        )?;
        tx.commit()?;
        Ok(new_status)
    }

    pub fn unblock_task(&self, task_id: &str) -> Result<(), PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let changed = conn.execute(
            "UPDATE tasks SET status = 'ready', consecutive_failures = 0 WHERE id = ?1",
            params![task_id],
        )?;
        if changed == 0 {
            return Err(PersistenceError::TaskNotFound(task_id.to_string()));
        }
        Ok(())
    }

    pub fn add_comment(
        &self,
        task_id: &str,
        author: &str,
        body: &str,
    ) -> Result<i64, PersistenceError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let now = now_epoch_i64();
        conn.execute(
            "INSERT INTO task_comments(task_id, author, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, author, body, now],
        )?;
        Ok(conn.last_insert_rowid())
    }
}
