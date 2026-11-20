use crate::schedule::{compute_next_run, parse_schedule, CronSchedule};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

fn default_true() -> bool {
    true
}

#[derive(Debug, Error)]
pub enum CronError {
    #[error("cron job not found: {0}")]
    NotFound(String),
    #[error("schedule error: {0}")]
    Schedule(#[from] crate::schedule::ScheduleError),
    #[error("invalid cron job config: {0}")]
    InvalidJob(String),
    #[error("cron store locked by another process")]
    Locked,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    pub schedule: CronSchedule,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub repeat_remaining: Option<i64>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub no_agent: bool,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub next_run_at: Option<String>,
    #[serde(default)]
    pub pending_slot: Option<String>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub deliver_targets: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct JobsFile {
    jobs: Vec<CronJob>,
}

#[derive(Clone)]
pub struct CronStore {
    cron_dir: PathBuf,
    jobs_file: PathBuf,
    output_dir: PathBuf,
}

impl CronStore {
    pub fn new(cron_dir: PathBuf) -> Self {
        let jobs_file = cron_dir.join("jobs.json");
        let output_dir = cron_dir.join("output");
        Self {
            cron_dir,
            jobs_file,
            output_dir,
        }
    }

    pub fn cron_dir(&self) -> &Path {
        &self.cron_dir
    }

    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub fn load_jobs(&self) -> Result<Vec<CronJob>, CronError> {
        if !self.jobs_file.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.jobs_file)?;
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        if raw.trim_start().starts_with('[') {
            let jobs: Vec<CronJob> = serde_json::from_str(&raw)?;
            return Ok(jobs);
        }
        let parsed: JobsFile = serde_json::from_str(&raw)?;
        Ok(parsed.jobs)
    }

    pub fn save_jobs(&self, jobs: &[CronJob]) -> Result<(), CronError> {
        fs::create_dir_all(&self.cron_dir)?;
        let payload = serde_json::to_string_pretty(&JobsFile {
            jobs: jobs.to_vec(),
        })?;
        let tmp = self
            .cron_dir
            .join(format!(".jobs.tmp.{}", std::process::id()));
        fs::write(&tmp, payload)?;
        fs::rename(tmp, &self.jobs_file)?;
        Ok(())
    }

    /// Reject inline command strings passed as `script` (must be a file inside `<home>/scripts/`).
    pub fn validate_script_path(home_dir: &Path, script: &str) -> Result<PathBuf, CronError> {
        let candidate = Path::new(script);
        if script.contains('\n')
            || script.contains(';')
            || script.contains("&&")
            || !candidate.is_relative() && !candidate.starts_with(home_dir.join("scripts"))
        {
            return Err(CronError::InvalidJob(
                "cron script must be a file path under $SAIF_HOME/scripts/, not a shell command"
                    .to_string(),
            ));
        }
        let resolved = if candidate.is_relative() {
            home_dir.join("scripts").join(candidate)
        } else {
            candidate.to_path_buf()
        };
        Ok(resolved)
    }

    pub fn create_job(
        &self,
        name: &str,
        schedule_str: &str,
        prompt: &str,
        repeat_remaining: Option<i64>,
        workdir: Option<&str>,
    ) -> Result<CronJob, CronError> {
        let schedule = parse_schedule(schedule_str)?;
        let now = Utc::now();
        let next_run_at = compute_next_run(&schedule, now, None);
        let job = CronJob {
            id: format!("job-{}", &Uuid::new_v4().to_string()[..8]),
            name: name.to_string(),
            prompt: prompt.to_string(),
            schedule,
            enabled: true,
            repeat_remaining,
            script: None,
            no_agent: false,
            workdir: workdir.map(ToString::to_string),
            model: None,
            provider: None,
            created_at: now.to_rfc3339(),
            last_run_at: None,
            next_run_at,
            pending_slot: None,
            last_status: None,
            last_error: None,
            deliver_targets: Vec::new(),
            extra: BTreeMap::new(),
        };
        let mut jobs = self.load_jobs()?;
        jobs.push(job.clone());
        self.save_jobs(&jobs)?;
        Ok(job)
    }

    pub fn set_enabled(&self, job_id: &str, enabled: bool) -> Result<CronJob, CronError> {
        let mut jobs = self.load_jobs()?;
        let job = jobs
            .iter_mut()
            .find(|j| j.id == job_id || j.name == job_id)
            .ok_or_else(|| CronError::NotFound(job_id.to_string()))?;
        job.enabled = enabled;
        let updated = job.clone();
        self.save_jobs(&jobs)?;
        Ok(updated)
    }

    pub fn remove_job(&self, job_id: &str) -> Result<(), CronError> {
        let mut jobs = self.load_jobs()?;
        let before = jobs.len();
        jobs.retain(|j| j.id != job_id && j.name != job_id);
        if jobs.len() == before {
            return Err(CronError::NotFound(job_id.to_string()));
        }
        self.save_jobs(&jobs)
    }

    /// At-most-once invariant (`cron/AGENTS.md`): advance `next_run_at` AND stamp `pending_slot`
    /// BEFORE dispatching the job so a crash mid-run never double-fires the same occurrence.
    pub fn advance_before_dispatch(&self, job_id: &str) -> Result<Option<CronJob>, CronError> {
        let mut jobs = self.load_jobs()?;
        let now = Utc::now();
        let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) else {
            return Err(CronError::NotFound(job_id.to_string()));
        };
        if !job.enabled {
            return Ok(None);
        }
        let Some(due_slot) = job.next_run_at.clone() else {
            return Ok(None);
        };
        job.pending_slot = Some(due_slot.clone());
        job.last_run_at = Some(now.to_rfc3339());
        if let Some(rem) = job.repeat_remaining {
            if rem <= 1 {
                job.repeat_remaining = Some(0);
                job.enabled = false;
                job.next_run_at = None;
            } else {
                job.repeat_remaining = Some(rem - 1);
                job.next_run_at = compute_next_run(&job.schedule, now, Some(&due_slot));
            }
        } else {
            job.next_run_at = compute_next_run(&job.schedule, now, Some(&due_slot));
        }
        if matches!(job.schedule, CronSchedule::Once { .. }) {
            job.enabled = false;
            job.next_run_at = None;
        }
        let snapshot = job.clone();
        self.save_jobs(&jobs)?;
        Ok(Some(snapshot))
    }

    pub fn record_completion(
        &self,
        job_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), CronError> {
        let mut jobs = self.load_jobs()?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
            job.pending_slot = None;
            job.last_status = Some(status.to_string());
            job.last_error = error.map(ToString::to_string);
            self.save_jobs(&jobs)?;
        }
        Ok(())
    }
}
