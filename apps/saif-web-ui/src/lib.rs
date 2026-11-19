//! Pure-Rust WASM dashboard frontend (`wasm32-unknown-unknown`) sharing
//! `saif-rpc-contract` wire types with `saif-web` and `saif-tui`.

use saif_rpc_contract::{
    default_command_catalog, GatewayEvent, PromptSubmitParams, RpcRequest, StatusSnapshot,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardUiState {
    pub connected: bool,
    pub active_session_id: Option<String>,
    pub profile: String,
    pub skin_name: String,
    pub transcript: Vec<String>,
    pub status: Option<StatusSnapshot>,
}

impl DashboardUiState {
    pub fn apply_event(&mut self, event_json: &str) -> Result<(), String> {
        let ev: GatewayEvent =
            serde_json::from_str(event_json).map_err(|e| format!("invalid event: {e}"))?;
        match ev {
            GatewayEvent::GatewayReady {
                profile, skin_name, ..
            } => {
                self.connected = true;
                self.profile = profile;
                self.skin_name = skin_name;
            }
            GatewayEvent::MessageDelta { delta, .. } => {
                if let Some(last) = self.transcript.last_mut() {
                    last.push_str(&delta);
                } else {
                    self.transcript.push(delta);
                }
            }
            GatewayEvent::MessageComplete { content, .. } => {
                self.transcript.push(content);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn build_prompt_rpc(&self, id: u64, prompt: &str) -> String {
        let req = RpcRequest::new(
            id,
            "prompt.submit",
            serde_json::to_value(PromptSubmitParams {
                session_id: self
                    .active_session_id
                    .clone()
                    .unwrap_or_else(|| "default-web".to_string()),
                prompt: prompt.to_string(),
                steer: false,
            })
            .unwrap_or(serde_json::Value::Null),
        );
        serde_json::to_string(&req).unwrap_or_default()
    }

    pub fn slash_commands_json(&self) -> String {
        serde_json::to_string(&default_command_catalog()).unwrap_or_else(|_| "[]".to_string())
    }
}
