use crate::store::{CronError, CronJob, CronStore};
use chrono::{DateTime, Utc};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TickOutcome {
    pub due_jobs: Vec<CronJob>,
    pub skipped_due_to_lock: bool,
}

/// Per-home scheduler with exclusive `.tick.lock` file guard and 600s inactivity timeout policy.
pub struct CronScheduler {
    store: CronStore,
    inactivity_timeout: Duration,
}

impl CronScheduler {
    pub fn new(store: CronStore, inactivity_timeout_secs: u64) -> Self {
        Self {
            store,
            inactivity_timeout: Duration::from_secs(inactivity_timeout_secs),
        }
    }

    pub fn inactivity_timeout(&self) -> Duration {
        self.inactivity_timeout
    }

    fn tick_lock_path(&self) -> PathBuf {
        self.store.cron_dir().join(".tick.lock")
    }

    /// Collect due jobs atomically under a per-home `.tick.lock` file, advancing `next_run_at`
    /// before returning them for dispatch.
    pub fn tick_due_jobs(&self) -> Result<TickOutcome, CronError> {
        fs::create_dir_all(self.store.cron_dir())?;
        let lock_path = self.tick_lock_path();
        let Ok(mut lock_file) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        else {
            return Ok(TickOutcome {
                due_jobs: Vec::new(),
                skipped_due_to_lock: true,
            });
        };
        let _ = writeln!(lock_file, "{}", std::process::id());
        let res = self.collect_and_advance();
        let _ = fs::remove_file(&lock_path);
        res
    }

    fn collect_and_advance(&self) -> Result<TickOutcome, CronError> {
        let now = Utc::now();
        let jobs = self.store.load_jobs()?;
        let mut due = Vec::new();
        for job in jobs {
            if !job.enabled {
                continue;
            }
            let Some(next_str) = job.next_run_at.as_deref() else {
                continue;
            };
            let Ok(next_dt) = DateTime::parse_from_rfc3339(next_str) else {
                continue;
            };
            if next_dt.with_timezone(&Utc) <= now {
                if let Some(advanced) = self.store.advance_before_dispatch(&job.id)? {
                    due.push(advanced);
                }
            }
        }
        Ok(TickOutcome {
            due_jobs: due,
            skipped_due_to_lock: false,
        })
    }
}
