use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Stdio;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Error)]
pub enum PluginHostError {
    #[error("plugin host I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin host JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plugin host protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePluginRpc {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// Out-of-process plugin boundary (`plugins.isolation: host` parity).
/// Runs third-party plugins in a child process with scrubbed profile env over NDJSON stdio.
pub struct PluginHostClient {
    child: Child,
    next_id: u64,
}

impl PluginHostClient {
    pub fn spawn(
        program: &str,
        args: &[String],
        child_env: &BTreeMap<String, String>,
    ) -> Result<Self, PluginHostError> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .env_clear()
            .envs(child_env)
            .env("SAIF_PLUGIN_HOST_PROCESS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let child = cmd.spawn()?;
        Ok(Self { child, next_id: 1 })
    }

    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, PluginHostError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = RemotePluginRpc {
            id,
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&req)? + "\n";
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| PluginHostError::Protocol("plugin host stdin closed".to_string()))?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;

        let stdout =
            self.child.stdout.as_mut().ok_or_else(|| {
                PluginHostError::Protocol("plugin host stdout closed".to_string())
            })?;
        let mut reader = BufReader::new(stdout);
        let mut resp_line = String::new();
        reader.read_line(&mut resp_line).await?;
        let val: Value = serde_json::from_str(resp_line.trim())?;
        Ok(val.get("result").cloned().unwrap_or(val))
    }

    pub async fn shutdown(&mut self) -> Result<(), PluginHostError> {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        Ok(())
    }
}
