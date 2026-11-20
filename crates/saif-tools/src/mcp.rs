use crate::registry::{ToolContext, ToolError, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use saif_providers::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Per-profile MCP trust ledger (`tools/mcp_tool_registration.py::_record_scope_trust` parity).
#[derive(Debug, Clone)]
pub struct McpTrustStore {
    trust_file: PathBuf,
}

impl McpTrustStore {
    pub fn new(home_dir: &Path) -> Self {
        Self {
            trust_file: home_dir.join("mcp_trust.json"),
        }
    }

    pub fn is_trusted(&self, server_name: &str) -> bool {
        let Ok(raw) = fs::read_to_string(&self.trust_file) else {
            return false;
        };
        let Ok(set) = serde_json::from_str::<HashSet<String>>(&raw) else {
            return false;
        };
        set.contains(server_name)
    }

    pub fn grant_trust(&self, server_name: &str) -> Result<(), ToolError> {
        let mut set: HashSet<String> = fs::read_to_string(&self.trust_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        set.insert(server_name.to_string());
        if let Some(parent) = self.trust_file.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolError::Execution(e.to_string()))?;
        }
        let encoded =
            serde_json::to_string_pretty(&set).map_err(|e| ToolError::Execution(e.to_string()))?;
        fs::write(&self.trust_file, encoded).map_err(|e| ToolError::Execution(e.to_string()))
    }
}

pub struct McpClient {
    child: Child,
    next_id: u64,
}

impl McpClient {
    pub async fn connect_stdio(
        cfg: &McpServerConfig,
        scrubbed_env: &BTreeMap<String, String>,
    ) -> Result<Self, ToolError> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .env_clear()
            .envs(scrubbed_env)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("MCP spawn '{}': {e}", cfg.name)))?;
        let mut client = Self { child, next_id: 1 };
        let _ = client
            .rpc(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "saif-agent", "version": "0.1.0" }
                }),
            )
            .await?;
        Ok(client)
    }

    pub async fn list_tools(&mut self) -> Result<Value, ToolError> {
        self.rpc("tools/list", json!({})).await
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        self.rpc(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let line = serde_json::to_string(&payload)
            .map_err(|e| ToolError::Execution(e.to_string()))?
            + "\n";
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| ToolError::Execution("MCP stdin closed".to_string()))?;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?;

        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| ToolError::Execution("MCP stdout closed".to_string()))?;
        let mut reader = BufReader::new(stdout);
        let mut resp_line = String::new();
        reader
            .read_line(&mut resp_line)
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        let resp: Value = serde_json::from_str(resp_line.trim())
            .map_err(|e| ToolError::Execution(format!("MCP invalid JSON: {e}")))?;
        if let Some(err) = resp.get("error") {
            return Err(ToolError::Execution(format!("MCP error: {err}")));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn close(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

struct McpRemoteTool {
    registered_name: String,
    remote_name: String,
    description: String,
    parameters: Value,
    server: McpServerConfig,
    scrubbed_env: BTreeMap<String, String>,
}

#[async_trait]
impl ToolHandler for McpRemoteTool {
    fn name(&self) -> &str {
        &self.registered_name
    }

    fn toolset(&self) -> &str {
        "mcp"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.registered_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, ToolError> {
        let mut client = McpClient::connect_stdio(&self.server, &self.scrubbed_env).await?;
        let res = client.call_tool(&self.remote_name, args).await;
        client.close().await;
        let val = res?;
        serde_json::to_string(&val).map_err(|e| ToolError::Execution(e.to_string()))
    }
}

/// Discover and register tools from trusted MCP servers (`tools/mcp_tool_registration.py` parity).
pub async fn register_trusted_mcp_tools(
    registry: &ToolRegistry,
    servers: &[McpServerConfig],
    trust: &McpTrustStore,
    scrubbed_env: &BTreeMap<String, String>,
) -> usize {
    let mut count = 0usize;
    for srv in servers {
        if !trust.is_trusted(&srv.name) || srv.command.trim().is_empty() {
            continue;
        }
        let Ok(Ok(mut client)) = tokio::time::timeout(
            Duration::from_secs(10),
            McpClient::connect_stdio(srv, scrubbed_env),
        )
        .await
        else {
            continue;
        };
        if let Ok(Ok(list_res)) =
            tokio::time::timeout(Duration::from_secs(10), client.list_tools()).await
        {
            if let Some(tools) = list_res.get("tools").and_then(|v| v.as_array()) {
                for tool in tools {
                    let Some(remote_name) = tool.get("name").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let description = tool
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Trusted MCP tool")
                        .to_string();
                    let parameters = tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object" }));
                    let registered_name = format!("mcp__{}__{}", srv.name, remote_name);
                    registry.register(Arc::new(McpRemoteTool {
                        registered_name,
                        remote_name: remote_name.to_string(),
                        description,
                        parameters,
                        server: srv.clone(),
                        scrubbed_env: scrubbed_env.clone(),
                    }));
                    count += 1;
                }
            }
        }
        client.close().await;
    }
    count
}
