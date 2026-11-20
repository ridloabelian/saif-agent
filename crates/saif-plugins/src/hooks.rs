use crate::host::PluginHostClient;
use crate::manifest::DiscoveredPlugin;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PreToolCall,
    PostToolCall,
    PreLlmCall,
    PostLlmCall,
    OnSessionStart,
    OnSessionEnd,
    OnSessionDelete,
    PreAuxiliaryCall,
    PostAuxiliaryCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPayload {
    pub event: HookEvent,
    pub session_id: Option<String>,
    pub profile: String,
    pub data: Value,
}

#[async_trait]
pub trait HookSubscriber: Send + Sync {
    async fn on_hook(&self, payload: &HookPayload);
}

#[derive(Clone, Default)]
pub struct HookRegistry {
    subscribers: Arc<RwLock<Vec<Arc<dyn HookSubscriber>>>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, sub: Arc<dyn HookSubscriber>) {
        if let Ok(mut guard) = self.subscribers.write() {
            guard.push(sub);
        }
    }

    pub async fn emit(&self, payload: HookPayload) {
        let snapshot: Vec<Arc<dyn HookSubscriber>> = self
            .subscribers
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        for sub in snapshot {
            sub.on_hook(&payload).await;
        }
    }
}

/// Runtime hook bridge connecting discovered plugins (in-process telemetry or `isolation: host` child RPC).
pub struct PluginHookBridge {
    plugins: Vec<DiscoveredPlugin>,
    isolation_host: bool,
    child_env: BTreeMap<String, String>,
    log_path: PathBuf,
}

impl PluginHookBridge {
    pub fn new(
        plugins: Vec<DiscoveredPlugin>,
        isolation_host: bool,
        child_env: BTreeMap<String, String>,
        log_path: PathBuf,
    ) -> Self {
        Self {
            plugins,
            isolation_host,
            child_env,
            log_path,
        }
    }
}

#[async_trait]
impl HookSubscriber for PluginHookBridge {
    async fn on_hook(&self, payload: &HookPayload) {
        let enabled: Vec<&DiscoveredPlugin> = self.plugins.iter().filter(|p| p.enabled).collect();
        if enabled.is_empty() {
            return;
        }
        if let Some(parent) = self.log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let names: Vec<&str> = enabled.iter().map(|p| p.manifest.name.as_str()).collect();
            let row = json!({
                "event": payload.event,
                "session_id": payload.session_id,
                "profile": payload.profile,
                "plugins": names,
                "data": payload.data
            });
            let _ = writeln!(file, "{row}");
        }

        if self.isolation_host {
            let payload_val = serde_json::to_value(payload).unwrap_or(Value::Null);
            for plugin in enabled {
                let Some(entry) = plugin
                    .manifest
                    .entry
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                else {
                    continue;
                };
                let candidate = plugin.root_dir.join(entry);
                let prog = if candidate.is_file() {
                    candidate.to_string_lossy().into_owned()
                } else {
                    entry.to_string()
                };
                if let Ok(mut client) = PluginHostClient::spawn(&prog, &[], &self.child_env) {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        client.call("hook.emit", payload_val.clone()),
                    )
                    .await;
                    let _ = client.shutdown().await;
                }
            }
        }
    }
}
