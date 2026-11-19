use crate::registry::ToolError;
use async_trait::async_trait;
use saif_platform::{terminate_tree_parent_first, ProcessIdentity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::{Child, Command};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[async_trait]
pub trait ExecutionEnvironment: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError>;
}

#[derive(Default, Clone)]
pub struct LocalEnv;

#[async_trait]
impl ExecutionEnvironment for LocalEnv {
    fn kind(&self) -> &'static str {
        "local"
    }

    async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError> {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", command]);
            c
        };
        cmd.current_dir(cwd)
            .env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("failed spawning command: {e}")))?;
        let pid = child.id();

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => Ok(ExecResult {
                exit_code: out.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(ToolError::Execution(format!("process wait failed: {e}"))),
            Err(_) => {
                if let Some(p) = pid {
                    let _ = terminate_tree_parent_first(p, Duration::from_millis(500));
                }
                Ok(ExecResult {
                    exit_code: 124,
                    stdout: String::new(),
                    stderr: format!("command timed out after {}s", timeout.as_secs()),
                    timed_out: true,
                })
            }
        }
    }
}

#[derive(Clone)]
pub struct DockerEnv {
    pub image: String,
    pub container_name: Option<String>,
}

#[async_trait]
impl ExecutionEnvironment for DockerEnv {
    fn kind(&self) -> &'static str {
        "docker"
    }

    async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError> {
        let mut cmd = Command::new("docker");
        if let Some(container) = &self.container_name {
            cmd.args(["exec", "-i", container, "sh", "-c", command]);
        } else {
            cmd.args([
                "run",
                "--rm",
                "-i",
                "-v",
                &format!("{}:/workspace", cwd.display()),
                "-w",
                "/workspace",
                &self.image,
                "sh",
                "-c",
                command,
            ]);
        }
        cmd.env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("docker spawn failed: {e}")))?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => Ok(ExecResult {
                exit_code: out.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(ToolError::Execution(format!("docker wait failed: {e}"))),
            Err(_) => Ok(ExecResult {
                exit_code: 124,
                stdout: String::new(),
                stderr: "docker command timed out".to_string(),
                timed_out: true,
            }),
        }
    }
}

#[derive(Clone)]
pub struct SshEnv {
    pub target: String,
    pub port: Option<u16>,
}

#[async_trait]
impl ExecutionEnvironment for SshEnv {
    fn kind(&self) -> &'static str {
        "ssh"
    }

    async fn execute(
        &self,
        command: &str,
        _cwd: &Path,
        _env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError> {
        let mut cmd = Command::new("ssh");
        if let Some(p) = self.port {
            cmd.args(["-p", &p.to_string()]);
        }
        cmd.arg(&self.target)
            .arg(command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("ssh spawn failed: {e}")))?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => Ok(ExecResult {
                exit_code: out.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(ToolError::Execution(format!("ssh wait failed: {e}"))),
            Err(_) => Ok(ExecResult {
                exit_code: 124,
                stdout: String::new(),
                stderr: "ssh command timed out".to_string(),
                timed_out: true,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessEntry {
    pub id: String,
    pub command: String,
    pub owner_task_id: Option<String>,
    pub identity: Option<ProcessIdentity>,
    pub started_at: u64,
    pub exited: bool,
    pub exit_code: Option<i32>,
    pub output_tail: String,
}

struct TrackedProcess {
    entry: ProcessEntry,
    child: Option<Child>,
}

/// Process registry for background commands (`process_manage`: list/poll/kill/handoff).
#[derive(Clone, Default)]
pub struct ProcessRegistry {
    inner: Arc<Mutex<HashMap<String, TrackedProcess>>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn_background(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        owner_task_id: Option<String>,
    ) -> Result<ProcessEntry, ToolError> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command])
            .current_dir(cwd)
            .env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("background spawn failed: {e}")))?;
        let identity = child.id().and_then(ProcessIdentity::probe);
        let id = format!("proc-{}", &Uuid::new_v4().to_string()[..8]);
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = ProcessEntry {
            id: id.clone(),
            command: command.to_string(),
            owner_task_id,
            identity,
            started_at,
            exited: false,
            exit_code: None,
            output_tail: String::new(),
        };
        if let Ok(mut map) = self.inner.lock() {
            map.insert(
                id,
                TrackedProcess {
                    entry: entry.clone(),
                    child: Some(child),
                },
            );
        }
        Ok(entry)
    }

    pub fn list(&self) -> Vec<ProcessEntry> {
        let Ok(mut map) = self.inner.lock() else {
            return Vec::new();
        };
        for proc in map.values_mut() {
            if let Some(child) = proc.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    proc.entry.exited = true;
                    proc.entry.exit_code = status.code();
                }
            }
        }
        map.values().map(|p| p.entry.clone()).collect()
    }

    pub fn transfer_ownership(&self, id: &str, new_owner_task_id: Option<String>) -> bool {
        let Ok(mut map) = self.inner.lock() else {
            return false;
        };
        if let Some(proc) = map.get_mut(id) {
            proc.entry.owner_task_id = new_owner_task_id;
            return true;
        }
        false
    }

    pub fn kill(&self, id: &str, grace: Duration) -> Result<bool, ToolError> {
        let pid = {
            let Ok(mut map) = self.inner.lock() else {
                return Ok(false);
            };
            let Some(proc) = map.get_mut(id) else {
                return Ok(false);
            };
            proc.entry.exited = true;
            proc.entry.identity.map(|i| i.pid)
        };
        if let Some(p) = pid {
            let _ = terminate_tree_parent_first(p, grace);
        }
        Ok(true)
    }
}
