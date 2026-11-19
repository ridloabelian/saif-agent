#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryAction {
    UpdateStream { message_id: String, frame: String },
    InterimSend { text: String },
    EditMessage { message_id: String, text: String },
    SendMessage { text: String },
}

/// Enforces all 4 streaming delivery invariants (`gateway/AGENTS.md`):
/// 1. Draft frames are strictly prefix-stable (`frame[n]` starts with `frame[n-1]`).
/// 2. Consumer explicitly seals final via `finish(final_text)`.
/// 3. Interim sends carry `is_interim=true` so they do NOT seal the cumulative stream.
/// 4. Final reconciliation prefers `EditMessage` on an existing `message_id` before `SendMessage`.
#[derive(Debug, Clone, Default)]
pub struct StreamDraftController {
    cumulative_frame: String,
    active_message_id: Option<String>,
    sealed: bool,
}

impl StreamDraftController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_message_id(&mut self, message_id: impl Into<String>) {
        self.active_message_id = Some(message_id.into());
    }

    pub fn cumulative_frame(&self) -> &str {
        &self.cumulative_frame
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Push a delta, enforcing Invariant 1 (prefix-stability).
    pub fn push_delta(&mut self, delta: &str) -> Option<DeliveryAction> {
        if self.sealed || delta.is_empty() {
            return None;
        }
        let next = format!("{}{}", self.cumulative_frame, delta);
        debug_assert!(next.starts_with(&self.cumulative_frame));
        self.cumulative_frame = next;
        self.active_message_id
            .as_ref()
            .map(|id| DeliveryAction::UpdateStream {
                message_id: id.clone(),
                frame: self.cumulative_frame.clone(),
            })
    }

    /// Push a full snapshot; rejects/fixes any non-prefix mutation so stacked copies never occur.
    pub fn push_snapshot(&mut self, candidate: &str) -> Result<Option<DeliveryAction>, String> {
        if self.sealed {
            return Ok(None);
        }
        if !candidate.starts_with(&self.cumulative_frame) {
            return Err(
                "Invariant 1 violated: draft snapshot is not a prefix extension of prior frame"
                    .to_string(),
            );
        }
        self.cumulative_frame = candidate.to_string();
        Ok(self
            .active_message_id
            .as_ref()
            .map(|id| DeliveryAction::UpdateStream {
                message_id: id.clone(),
                frame: self.cumulative_frame.clone(),
            }))
    }

    /// Send an interim message without sealing the active stream (Invariant 3).
    pub fn send_interim(&self, text: &str) -> DeliveryAction {
        DeliveryAction::InterimSend {
            text: text.to_string(),
        }
    }

    /// Explicitly seal and reconcile the final response (Invariants 2 & 4).
    pub fn finish(&mut self, final_text: &str) -> DeliveryAction {
        self.sealed = true;
        if let Some(id) = &self.active_message_id {
            DeliveryAction::EditMessage {
                message_id: id.clone(),
                text: final_text.to_string(),
            }
        } else {
            DeliveryAction::SendMessage {
                text: final_text.to_string(),
            }
        }
    }
}
