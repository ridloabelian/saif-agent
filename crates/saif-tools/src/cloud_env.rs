use crate::env::{ExecResult, ExecutionEnvironment};
use crate::registry::ToolError;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TRANSIENT_VERCEL_CODES: &[u16] = &[408, 425, 429, 500, 502, 503, 504];

/// Profile-scoped JSON snapshot ledger (`modal_snapshots.json` / `vercel_sandbox_snapshots.json`).
#[derive(Debug, Clone)]
pub struct SnapshotLedger {
    path: PathBuf,
}

impl SnapshotLedger {
    pub fn modal(saif_home: &Path) -> Self {
        Self {
            path: saif_home.join("modal_snapshots.json"),
        }
    }

    pub fn vercel(saif_home: &Path) -> Self {
        Self {
            path: saif_home.join("vercel_sandbox_snapshots.json"),
        }
    }

    fn load_map(&self) -> BTreeMap<String, String> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save_map(&self, map: &BTreeMap<String, String>) -> Result<(), ToolError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolError::Execution(e.to_string()))?;
        }
        let body =
            serde_json::to_string_pretty(map).map_err(|e| ToolError::Execution(e.to_string()))?;
        let tmp = self
            .path
            .with_extension(format!("json.tmp.{}", std::process::id()));
        let mut f = fs::File::create(&tmp).map_err(|e| ToolError::Execution(e.to_string()))?;
        f.write_all(body.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        fs::rename(tmp, &self.path).map_err(|e| ToolError::Execution(e.to_string()))?;
        Ok(())
    }

    /// Namespaced `direct:<task_id>` wins over legacy `<task_id>` (`tools/environments/modal.py`).
    pub fn get_restore_candidate(&self, task_id: &str) -> Option<(String, bool)> {
        let map = self.load_map();
        if let Some(id) = map
            .get(&format!("direct:{task_id}"))
            .filter(|s| !s.is_empty())
        {
            return Some((id.clone(), false));
        }
        if let Some(id) = map.get(task_id).filter(|s| !s.is_empty()) {
            return Some((id.clone(), true));
        }
        None
    }

    pub fn store_direct_snapshot(&self, task_id: &str, snapshot_id: &str) -> Result<(), ToolError> {
        let mut map = self.load_map();
        map.insert(format!("direct:{task_id}"), snapshot_id.to_string());
        map.remove(task_id);
        self.save_map(&map)
    }

    pub fn delete_direct_snapshot(&self, task_id: &str) -> Result<(), ToolError> {
        let mut map = self.load_map();
        map.remove(&format!("direct:{task_id}"));
        map.remove(task_id);
        self.save_map(&map)
    }
}

#[derive(Clone)]
pub struct ModalEnv {
    pub api_base_url: String,
    pub token_id: Option<String>,
    pub token_secret: Option<String>,
    pub image: String,
    pub task_id: String,
    pub saif_home: PathBuf,
    client: Client,
}

impl ModalEnv {
    pub fn new(
        api_base_url: impl Into<String>,
        token_id: Option<String>,
        token_secret: Option<String>,
        image: impl Into<String>,
        task_id: impl Into<String>,
        saif_home: PathBuf,
    ) -> Self {
        Self {
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
            token_id,
            token_secret,
            image: image.into(),
            task_id: task_id.into(),
            saif_home,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ExecutionEnvironment for ModalEnv {
    fn kind(&self) -> &'static str {
        "modal"
    }

    async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError> {
        let ledger = SnapshotLedger::modal(&self.saif_home);
        let snapshot = ledger
            .get_restore_candidate(&self.task_id)
            .map(|(id, _)| id);
        let payload = json!({
            "task_id": self.task_id,
            "image": self.image,
            "snapshot_id": snapshot,
            "command": command,
            "cwd": cwd.display().to_string(),
            "env": env_vars,
            "timeout_secs": timeout.as_secs(),
        });
        let mut req = self
            .client
            .post(format!("{}/v1/sandboxes/exec", self.api_base_url))
            .timeout(timeout.max(Duration::from_secs(5)))
            .json(&payload);
        if let (Some(id), Some(sec)) = (&self.token_id, &self.token_secret) {
            req = req.header("Modal-Key", id).header("Modal-Secret", sec);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("modal request failed: {e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Execution(format!("modal JSON error: {e}")))?;
        if !status.is_success() {
            return Err(ToolError::Execution(format!(
                "modal error {status}: {body}"
            )));
        }
        if let Some(new_snap) = body.get("snapshot_id").and_then(|v| v.as_str()) {
            let _ = ledger.store_direct_snapshot(&self.task_id, new_snap);
        }
        Ok(parse_cloud_exec_result(&body))
    }
}

#[derive(Clone)]
pub struct DaytonaEnv {
    pub api_base_url: String,
    pub api_key: Option<String>,
    pub image: String,
    pub task_id: String,
    pub cpu: u32,
    pub memory_mb: u32,
    pub disk_gib: u32,
    client: Client,
}

impl DaytonaEnv {
    pub fn new(
        api_base_url: impl Into<String>,
        api_key: Option<String>,
        image: impl Into<String>,
        task_id: impl Into<String>,
        disk_mb: u32,
    ) -> Self {
        // Daytona caps sandbox disk allocation to 10 GiB (`tools/environments/daytona.py`).
        let disk_gib = disk_mb.div_ceil(1024).clamp(1, 10);
        Self {
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
            api_key,
            image: image.into(),
            task_id: task_id.into(),
            cpu: 1,
            memory_mb: 5120,
            disk_gib,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ExecutionEnvironment for DaytonaEnv {
    fn kind(&self) -> &'static str {
        "daytona"
    }

    async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError> {
        let payload = json!({
            "sandbox_name": format!("saif-{}", self.task_id),
            "image": self.image,
            "resources": {
                "cpu": self.cpu,
                "memory_gib": self.memory_mb.div_ceil(1024).max(1),
                "disk_gib": self.disk_gib,
            },
            "command": command,
            "cwd": cwd.display().to_string(),
            "env": env_vars,
            "timeout_secs": timeout.as_secs(),
        });
        let mut req = self
            .client
            .post(format!("{}/v1/toolbox/exec", self.api_base_url))
            .timeout(timeout.max(Duration::from_secs(5)))
            .json(&payload);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("daytona request failed: {e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Execution(format!("daytona JSON error: {e}")))?;
        if !status.is_success() {
            return Err(ToolError::Execution(format!(
                "daytona error {status}: {body}"
            )));
        }
        Ok(parse_cloud_exec_result(&body))
    }
}

#[derive(Clone)]
pub struct VercelSandboxEnv {
    pub api_base_url: String,
    pub token: Option<String>,
    pub task_id: String,
    pub saif_home: PathBuf,
    pub telemetry_disabled: bool,
    client: Client,
}

impl VercelSandboxEnv {
    pub fn new(
        api_base_url: impl Into<String>,
        token: Option<String>,
        task_id: impl Into<String>,
        saif_home: PathBuf,
    ) -> Self {
        Self {
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
            token,
            task_id: task_id.into(),
            saif_home,
            telemetry_disabled: true, // Privacy-by-default matching `vercel_sandbox.py`
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ExecutionEnvironment for VercelSandboxEnv {
    fn kind(&self) -> &'static str {
        "vercel_sandbox"
    }

    async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env_vars: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecResult, ToolError> {
        let ledger = SnapshotLedger::vercel(&self.saif_home);
        let snapshot = ledger
            .get_restore_candidate(&self.task_id)
            .map(|(id, _)| id);
        let payload = json!({
            "task_id": self.task_id,
            "snapshot_id": snapshot,
            "command": command,
            "cwd": cwd.display().to_string(),
            "env": env_vars,
            "timeout_secs": timeout.as_secs(),
        });

        let mut last_err = String::new();
        for attempt in 0..3u64 {
            let mut req = self
                .client
                .post(format!("{}/v1/sandboxes/run", self.api_base_url))
                .timeout(timeout.max(Duration::from_secs(5)))
                .header(
                    "x-vercel-telemetry-disabled",
                    if self.telemetry_disabled { "1" } else { "0" },
                )
                .json(&payload);
            if let Some(tok) = &self.token {
                req = req.bearer_auth(tok);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let body: Value = resp
                            .json()
                            .await
                            .map_err(|e| ToolError::Execution(format!("vercel JSON error: {e}")))?;
                        if let Some(new_snap) = body.get("snapshot_id").and_then(|v| v.as_str()) {
                            let _ = ledger.store_direct_snapshot(&self.task_id, new_snap);
                        }
                        return Ok(parse_cloud_exec_result(&body));
                    }
                    let code = status.as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    last_err = format!("vercel error {code}: {body}");
                    if !TRANSIENT_VERCEL_CODES.contains(&code) || attempt == 2 {
                        return Err(ToolError::Execution(last_err));
                    }
                }
                Err(e) => {
                    last_err = format!("vercel request failed: {e}");
                    if attempt == 2 {
                        return Err(ToolError::Execution(last_err));
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50 * (attempt + 1))).await;
        }
        Err(ToolError::Execution(last_err))
    }
}

fn parse_cloud_exec_result(body: &Value) -> ExecResult {
    ExecResult {
        exit_code: body.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        stdout: body
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        stderr: body
            .get("stderr")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        timed_out: body
            .get("timed_out")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}
