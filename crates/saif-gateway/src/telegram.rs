use crate::guards::{extract_steer_text, BusyDecision, BusyMessageGuard, ControlCommand};
use crate::identity::resolve_identity;
use crate::platforms::chunk_for_platform;
use crate::streaming::{DeliveryAction, StreamDraftController};
use reqwest::Client;
use saif_core::{AgentEvent, AgentRunner, TurnOptions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Native Telegram Bot API polling & 4-invariant draft streaming adapter.
pub struct TelegramAdapter {
    token: String,
    profile_name: String,
    allow_all: bool,
    allowed_users: Vec<String>,
    api_base_override: Option<String>,
    client: Client,
    guard: Arc<Mutex<BusyMessageGuard>>,
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
            api_base_override: None,
            client: Client::new(),
            guard: Arc::new(Mutex::new(BusyMessageGuard::default())),
        }
    }

    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base_override = Some(base.into().trim_end_matches('/').to_string());
        self
    }

    pub fn is_authorized(&self, user_id: &str) -> bool {
        self.allow_all || self.allowed_users.iter().any(|u| u == user_id)
    }

    fn base_url(&self) -> String {
        self.api_base_override
            .clone()
            .unwrap_or_else(|| format!("https://api.telegram.org/bot{}", self.token))
    }

    pub async fn run_polling(&mut self, agent: AgentRunner, cwd: PathBuf) -> Result<(), String> {
        self.run_polling_with_cancel(agent, cwd, CancellationToken::new())
            .await
    }

    pub async fn run_polling_with_cancel(
        &mut self,
        agent: AgentRunner,
        cwd: PathBuf,
        shutdown: CancellationToken,
    ) -> Result<(), String> {
        let base = self.base_url();
        let mut offset: i64 = 0;
        info!(
            "Telegram polling started for profile '{}'",
            self.profile_name
        );

        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            let resp = tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                res = self
                    .client
                    .get(format!("{base}/getUpdates"))
                    .query(&[("timeout", "25"), ("offset", &offset.to_string())])
                    .send() => res,
            };
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    warn!("Telegram getUpdates error: {e}");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
            };
            let Ok(val) = resp.json::<Value>().await else {
                continue;
            };
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

                let decision = {
                    let Ok(mut g) = self.guard.lock() else {
                        continue;
                    };
                    let d = g.admit(&identity.session_key, text);
                    if matches!(d, BusyDecision::ExecuteImmediately) {
                        g.mark_active(&identity.session_key);
                    }
                    d
                };

                match decision {
                    BusyDecision::Queued { position } => {
                        let _ = send_or_edit_msg(
                            &self.client,
                            &base,
                            &chat_id,
                            DeliveryAction::InterimSend {
                                text: format!("Queued (position {position})..."),
                            },
                        )
                        .await;
                        continue;
                    }
                    BusyDecision::ControlBypass(cmd) => {
                        let reply = self.handle_control(&identity.session_key, cmd, text);
                        let _ = send_or_edit_msg(
                            &self.client,
                            &base,
                            &chat_id,
                            DeliveryAction::SendMessage { text: reply },
                        )
                        .await;
                        continue;
                    }
                    BusyDecision::ExecuteImmediately => {}
                }

                let client_clone = self.client.clone();
                let base_clone = base.clone();
                let chat_clone = chat_id.clone();
                let session_key = identity.session_key.clone();
                let guard_clone = Arc::clone(&self.guard);
                let agent_clone = agent.clone();
                let cwd_clone = cwd.clone();
                let prompt_initial = text.to_string();

                tokio::spawn(async move {
                    let mut next_prompt = Some(prompt_initial);
                    while let Some(prompt_text) = next_prompt {
                        let cancel = CancellationToken::new();
                        let (steer_tx, mut steer_rx) = mpsc::unbounded_channel();
                        let active_sid = {
                            let Ok(mut g) = guard_clone.lock() else {
                                break;
                            };
                            g.register_turn(&session_key, cancel.clone(), Some(steer_tx));
                            g.effective_session_id(&session_key)
                        };

                        run_single_telegram_turn(
                            &client_clone,
                            &base_clone,
                            &chat_clone,
                            &agent_clone,
                            &cwd_clone,
                            &active_sid,
                            &prompt_text,
                            cancel,
                            &mut steer_rx,
                        )
                        .await;

                        next_prompt = {
                            let Ok(mut g) = guard_clone.lock() else {
                                break;
                            };
                            if let Some(q) = g.pop_next_queued(&session_key) {
                                Some(q)
                            } else {
                                g.mark_idle(&session_key);
                                None
                            }
                        };
                    }
                });
            }
        }
    }

    fn handle_control(&self, session_key: &str, cmd: ControlCommand, raw_text: &str) -> String {
        let Ok(mut g) = self.guard.lock() else {
            return "Gateway guard unavailable.".to_string();
        };
        match cmd {
            ControlCommand::Stop => {
                if g.cancel_active(session_key) {
                    "Stopped active turn.".to_string()
                } else {
                    "No active turn running.".to_string()
                }
            }
            ControlCommand::New => {
                let gen = g.reset_session(session_key);
                format!("Started fresh session (generation {gen}).")
            }
            ControlCommand::Steer => {
                let msg = extract_steer_text(raw_text).unwrap_or("");
                if msg.is_empty() {
                    "Usage: /steer <instruction>".to_string()
                } else if g.send_steer(session_key, msg.to_string()) {
                    "Steering active turn.".to_string()
                } else {
                    "No active turn to steer.".to_string()
                }
            }
            ControlCommand::Status => format!(
                "Session: {} | active={} | queued={}",
                g.effective_session_id(session_key),
                g.is_active(session_key),
                g.queue_len(session_key)
            ),
            other => format!("Control acknowledged: {other:?}"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_single_telegram_turn(
    client: &Client,
    base: &str,
    chat_id: &str,
    agent: &AgentRunner,
    cwd: &Path,
    session_id: &str,
    prompt_text: &str,
    cancel: CancellationToken,
    steer_rx: &mut mpsc::UnboundedReceiver<String>,
) {
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let opts = TurnOptions {
        session_id: Some(session_id.to_string()),
        source: "telegram".to_string(),
        cwd: cwd.to_path_buf(),
        ..TurnOptions::default()
    };

    let mut stream = StreamDraftController::new();
    let mut last_flush = Instant::now();
    let mut last_flushed_len = 0usize;
    let mut has_draft_msg = false;

    let reply_text = {
        let turn_fut =
            agent.run_conversation(prompt_text, None, opts, Some(ev_tx), cancel, Some(steer_rx));
        tokio::pin!(turn_fut);

        loop {
            tokio::select! {
                res = &mut turn_fut => {
                    while let Ok(ev) = ev_rx.try_recv() {
                        if let AgentEvent::ContentDelta(d) = ev {
                            let _ = stream.push_delta(&d);
                        }
                    }
                    break match res {
                        Ok(out) => out.final_response,
                        Err(e) => format!("Error: {e}"),
                    };
                }
                maybe_ev = ev_rx.recv() => {
                    if let Some(AgentEvent::ContentDelta(d)) = maybe_ev {
                        let _ = stream.push_delta(&d);
                        let frame_len = stream.cumulative_frame().len();
                        let should_flush = frame_len > 0 && (!has_draft_msg
                            || (frame_len - last_flushed_len >= 24 && last_flush.elapsed() >= Duration::from_millis(250)));
                        if should_flush {
                            let frame = stream.cumulative_frame().to_string();
                            if !has_draft_msg {
                                if let Ok(Some(mid)) = send_or_edit_msg(
                                    client,
                                    base,
                                    chat_id,
                                    DeliveryAction::InterimSend { text: frame },
                                ).await {
                                    stream.set_message_id(mid);
                                    has_draft_msg = true;
                                }
                            } else if let Some(action) = stream.push_snapshot(&frame).ok().flatten() {
                                let _ = send_or_edit_msg(client, base, chat_id, action).await;
                            }
                            last_flush = Instant::now();
                            last_flushed_len = frame_len;
                        }
                    }
                }
            }
        }
    };

    let chunks = chunk_for_platform(&reply_text, 4096);
    if let Some((first, rest)) = chunks.split_first() {
        let action = stream.finish(first);
        let _ = send_or_edit_msg(client, base, chat_id, action).await;
        for extra in rest {
            let _ = send_or_edit_msg(
                client,
                base,
                chat_id,
                DeliveryAction::SendMessage {
                    text: extra.clone(),
                },
            )
            .await;
        }
    } else {
        let action = stream.finish(&reply_text);
        let _ = send_or_edit_msg(client, base, chat_id, action).await;
    }
}

async fn send_or_edit_msg(
    client: &Client,
    base: &str,
    chat_id: &str,
    action: DeliveryAction,
) -> Result<Option<String>, String> {
    match action {
        DeliveryAction::SendMessage { text } | DeliveryAction::InterimSend { text } => {
            let resp = client
                .post(format!("{base}/sendMessage"))
                .json(&json!({ "chat_id": chat_id, "text": text }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let val: Value = resp.json().await.unwrap_or(Value::Null);
            let mid = val.pointer("/result/message_id").and_then(|v| {
                v.as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(ToString::to_string))
            });
            Ok(mid)
        }
        DeliveryAction::EditMessage { message_id, text }
        | DeliveryAction::UpdateStream {
            message_id,
            frame: text,
        } => {
            let _ = client
                .post(format!("{base}/editMessageText"))
                .json(&json!({
                    "chat_id": chat_id,
                    "message_id": message_id.parse::<i64>().unwrap_or(0),
                    "text": text
                }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            Ok(Some(message_id))
        }
    }
}
