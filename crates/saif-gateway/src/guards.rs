use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    Stop,
    New,
    Queue,
    Status,
    Approve,
    Deny,
    Steer,
}

impl ControlCommand {
    pub fn parse(text: &str) -> Option<Self> {
        let cmd = text
            .split_whitespace()
            .next()?
            .split('@')
            .next()?
            .to_ascii_lowercase();
        match cmd.as_str() {
            "/stop" => Some(Self::Stop),
            "/new" | "/reset" => Some(Self::New),
            "/queue" => Some(Self::Queue),
            "/status" => Some(Self::Status),
            "/approve" => Some(Self::Approve),
            "/deny" => Some(Self::Deny),
            "/steer" => Some(Self::Steer),
            _ => None,
        }
    }
}

pub fn extract_steer_text(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let first = trimmed.split_whitespace().next()?;
    let cmd = first.split('@').next()?.to_ascii_lowercase();
    if cmd != "/steer" {
        return None;
    }
    let rest = trimmed[first.len()..].trim();
    Some(rest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusyDecision {
    ExecuteImmediately,
    ControlBypass(ControlCommand),
    Queued { position: usize },
}

#[derive(Debug, Clone)]
pub struct ActiveTurnHandle {
    pub cancel: CancellationToken,
    pub steer_tx: Option<mpsc::UnboundedSender<String>>,
}

/// Two-stage busy guard (`gateway/AGENTS.md`):
/// Control/approval commands bypass both active-session queue and turn interruption.
#[derive(Debug, Default)]
pub struct BusyMessageGuard {
    active_sessions: HashSet<String>,
    pending_queues: HashMap<String, Vec<String>>,
    active_turns: HashMap<String, ActiveTurnHandle>,
    session_generations: HashMap<String, u64>,
}

impl BusyMessageGuard {
    pub fn effective_session_id(&self, session_key: &str) -> String {
        let gen = self
            .session_generations
            .get(session_key)
            .copied()
            .unwrap_or(0);
        if gen == 0 {
            session_key.to_string()
        } else {
            format!("{session_key}#gen{gen}")
        }
    }

    pub fn is_active(&self, session_key: &str) -> bool {
        self.active_sessions.contains(session_key)
    }

    pub fn queue_len(&self, session_key: &str) -> usize {
        self.pending_queues
            .get(session_key)
            .map(Vec::len)
            .unwrap_or(0)
    }

    pub fn mark_active(&mut self, session_key: &str) {
        self.active_sessions.insert(session_key.to_string());
    }

    pub fn register_turn(
        &mut self,
        session_key: &str,
        cancel: CancellationToken,
        steer_tx: Option<mpsc::UnboundedSender<String>>,
    ) {
        self.active_sessions.insert(session_key.to_string());
        self.active_turns.insert(
            session_key.to_string(),
            ActiveTurnHandle { cancel, steer_tx },
        );
    }

    pub fn cancel_active(&mut self, session_key: &str) -> bool {
        if let Some(handle) = self.active_turns.remove(session_key) {
            handle.cancel.cancel();
            self.active_sessions.remove(session_key);
            true
        } else {
            false
        }
    }

    pub fn send_steer(&self, session_key: &str, text: String) -> bool {
        if let Some(handle) = self.active_turns.get(session_key) {
            if let Some(tx) = &handle.steer_tx {
                return tx.send(text).is_ok();
            }
        }
        false
    }

    pub fn reset_session(&mut self, session_key: &str) -> u64 {
        let _ = self.cancel_active(session_key);
        self.pending_queues.remove(session_key);
        let gen = self
            .session_generations
            .entry(session_key.to_string())
            .or_insert(0);
        *gen += 1;
        *gen
    }

    pub fn pop_next_queued(&mut self, session_key: &str) -> Option<String> {
        let queue = self.pending_queues.get_mut(session_key)?;
        if queue.is_empty() {
            return None;
        }
        let next = queue.remove(0);
        if queue.is_empty() {
            self.pending_queues.remove(session_key);
        }
        Some(next)
    }

    pub fn mark_idle(&mut self, session_key: &str) -> Vec<String> {
        self.active_sessions.remove(session_key);
        self.active_turns.remove(session_key);
        self.pending_queues.remove(session_key).unwrap_or_default()
    }

    pub fn admit(&mut self, session_key: &str, text: &str) -> BusyDecision {
        if let Some(ctrl) = ControlCommand::parse(text) {
            return BusyDecision::ControlBypass(ctrl);
        }
        if self.active_sessions.contains(session_key) {
            let q = self
                .pending_queues
                .entry(session_key.to_string())
                .or_default();
            q.push(text.to_string());
            return BusyDecision::Queued { position: q.len() };
        }
        BusyDecision::ExecuteImmediately
    }
}
