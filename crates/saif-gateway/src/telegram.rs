use crate::guards::{BusyDecision, BusyMessageGuard};
use crate::identity::resolve_identity;
use crate::streaming::{DeliveryAction, StreamDraftController};
use reqwest::Client;
use saif_core::{AgentRunner, TurnOptions};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Native Telegram Bot API polling & 4-invariant draft streaming adapter.
pub struct TelegramAdapter {
    token: String,
    profile_name: String,
    allow_all: bool,
    allowed_users: Vec<String>,
    client: Client,
    guard: BusyMessageGuard,
}

impl TelegramAdapter {
    pub fn new(
        token: String,
        profile_name: String,
        allow_all: bool,
        allowed_users: Vec<String>,
    ) -> Self {
        Self {
            token,
            profile_name,
            allow_all,
            allowed_users,
            client: Client::new(),
            guard: BusyMessageGuard::default(),
        }
    }

    pub fn is_authorized(&self, user_id: &str) -> bool {
        self.allow_all || self.allowed_users.iter().any(|u| u == user_id)
    }

    pub async fn run_polling(&mut self, agent: AgentRunner, cwd: PathBuf) -> Result<(), String> {
        let base = format!("https://api.telegram.org/bot{}", self.token);
        let mut offset: i64 = 0;
        info!(
            "Telegram polling started for profile '{}'",
            self.profile_name
        );

        loop {
            let resp = self
                .client
                .get(format!("{base}/getUpdates"))
                .query(&[("timeout", "25"), ("offset", &offset.to_string())])
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let val: Value = resp.json().await.map_err(|e| e.to_string())?;
            let Some(updates) = val.get("result").and_then(|r| r.as_array()) else {
                continue;
            };
            for upd in updates {
                if let Some(uid) = upd.get("update_id").and_then(|v| v.as_i64()) {
                    offset = uid + 1;
                }
                let Some(msg) = upd.get("message") else {
                    continue;
                };
                let chat_id = msg
                    .pointer("/chat/id")
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                let user_id = msg
                    .pointer("/from/id")
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                if chat_id.is_empty() || text.is_empty() {
                    continue;
                }
                if !self.is_authorized(&user_id) {
                    warn!("Rejected unauthorized Telegram user {user_id}");
                    continue;
                }
                let is_dm = msg.pointer("/chat/type").and_then(|v| v.as_str()) == Some("private");
                let identity = resolve_identity(
                    "telegram",
                    &self.profile_name,
                    None,
                    &chat_id,
                    &user_id,
                    None,
                    is_dm,
                );

                match self.guard.admit(&identity.session_key, text) {
                    BusyDecision::Queued { .. } => continue,
                    BusyDecision::ControlBypass(cmd) => {
                        let _ = self
                            .send_or_edit(
                                &base,
                                &chat_id,
                                DeliveryAction::SendMessage {
                                    text: format!("Control acknowledged: {cmd:?}"),
                                },
                            )
                            .await;
                        continue;
                    }
                    BusyDecision::ExecuteImmediately => {}
                }

                self.guard.mark_active(&identity.session_key);
                let mut stream = StreamDraftController::new();
                let opts = TurnOptions {
                    session_id: Some(identity.session_key.clone()),
                    source: "telegram".to_string(),
                    cwd: cwd.clone(),
                    ..TurnOptions::default()
                };
                let reply_text = match agent
                    .run_conversation(text, None, opts, None, CancellationToken::new(), None)
                    .await
                {
                    Ok(out) => out.final_response,
                    Err(e) => format!("Error: {e}"),
                };
                let action = stream.finish(&reply_text);
                let _ = self.send_or_edit(&base, &chat_id, action).await;
                let _ = self.guard.mark_idle(&identity.session_key);
            }
        }
    }

    async fn send_or_edit(
        &self,
        base: &str,
        chat_id: &str,
        action: DeliveryAction,
    ) -> Result<(), String> {
        match action {
            DeliveryAction::SendMessage { text } | DeliveryAction::InterimSend { text } => {
                let _ = self
                    .client
                    .post(format!("{base}/sendMessage"))
                    .json(&json!({ "chat_id": chat_id, "text": text }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
            }
            DeliveryAction::EditMessage { message_id, text }
            | DeliveryAction::UpdateStream {
                message_id,
                frame: text,
            } => {
                let _ = self
                    .client
                    .post(format!("{base}/editMessageText"))
                    .json(&json!({
                        "chat_id": chat_id,
                        "message_id": message_id.parse::<i64>().unwrap_or(0),
                        "text": text
                    }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
