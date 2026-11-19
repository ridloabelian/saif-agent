//! Shared JSON-RPC 2.0 wire contracts (`tui_gateway/contracts/` parity).
//! Zero native OS dependencies so both native backend crates and `wasm32-unknown-unknown`
//! Rust web UI crates compile against the exact same wire types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl RpcRequest {
    pub fn new(id: impl Into<Value>, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSubmitParams {
    pub session_id: String,
    pub prompt: String,
    #[serde(default)]
    pub steer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCreateParams {
    pub profile: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandCatalogItem {
    pub name: String,
    pub description: String,
    pub category: String,
    pub aliases: Vec<String>,
    pub args_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusSnapshot {
    pub version: String,
    pub profile: String,
    pub model: String,
    pub provider: String,
    pub session_count: i64,
    pub live_sessions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayEvent {
    GatewayReady {
        version: String,
        profile: String,
        skin_name: String,
    },
    MessageDelta {
        session_id: String,
        delta: String,
    },
    ThinkingDelta {
        session_id: String,
        delta: String,
    },
    MessageComplete {
        session_id: String,
        content: String,
    },
    ToolStart {
        session_id: String,
        name: String,
        arguments: Value,
    },
    ToolComplete {
        session_id: String,
        name: String,
        ok: bool,
        preview: String,
    },
    RequestCancel {
        request_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerRequest {
    pub jsonrpc: String,
    pub id: String, // e.g., "srq-1"
    pub method: String,
    pub params: Value,
}

pub fn default_command_catalog() -> Vec<CommandCatalogItem> {
    vec![
        CommandCatalogItem {
            name: "help".into(),
            description: "Show available slash commands".into(),
            category: "Info".into(),
            aliases: vec!["h".into(), "?".into()],
            args_hint: None,
        },
        CommandCatalogItem {
            name: "new".into(),
            description: "Start a fresh conversation session".into(),
            category: "Session".into(),
            aliases: vec!["reset".into()],
            args_hint: None,
        },
        CommandCatalogItem {
            name: "model".into(),
            description: "Switch active LLM model".into(),
            category: "Configuration".into(),
            aliases: vec!["m".into()],
            args_hint: Some("[provider/model]".into()),
        },
        CommandCatalogItem {
            name: "skin".into(),
            description: "Switch UI theme (default, ares, mono, slate)".into(),
            category: "Configuration".into(),
            aliases: vec![],
            args_hint: Some("[name]".into()),
        },
        CommandCatalogItem {
            name: "compress".into(),
            description: "Compact older context turns cleanly".into(),
            category: "Session".into(),
            aliases: vec!["compact".into()],
            args_hint: None,
        },
        CommandCatalogItem {
            name: "sessions".into(),
            description: "List recent persistent sessions".into(),
            category: "Session".into(),
            aliases: vec!["ls".into()],
            args_hint: None,
        },
        CommandCatalogItem {
            name: "status".into(),
            description: "Display runtime, token usage, and profile status".into(),
            category: "Info".into(),
            aliases: vec![],
            args_hint: None,
        },
        CommandCatalogItem {
            name: "clear".into(),
            description: "Clear terminal transcript viewport".into(),
            category: "Info".into(),
            aliases: vec![],
            args_hint: None,
        },
        CommandCatalogItem {
            name: "quit".into(),
            description: "Exit interactive session".into(),
            category: "Exit".into(),
            aliases: vec!["exit".into(), "q".into()],
            args_hint: None,
        },
    ]
}
