use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    Stop,
    New,
    Queue,
    Status,
    Approve,
    Deny,
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
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusyDecision {
    ExecuteImmediately,
    ControlBypass(ControlCommand),
    Queued { position: usize },
}

/// Two-stage busy guard (`gateway/AGENTS.md`):
/// Control/approval commands bypass both active-session queue and turn interruption.
#[derive(Debug, Default)]
pub struct BusyMessageGuard {
    active_sessions: HashSet<String>,
    pending_queues: HashMap<String, Vec<String>>,
}

impl BusyMessageGuard {
    pub fn mark_active(&mut self, session_key: &str) {
        self.active_sessions.insert(session_key.to_string());
    }

    pub fn mark_idle(&mut self, session_key: &str) -> Vec<String> {
        self.active_sessions.remove(session_key);
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
