use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, RwLock};

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
