use serde::{Deserialize, Serialize};

/// Canonical per-inbound-event routing identity (`gateway/session_identity.py` parity).
/// Always resolved FIRST at ingress before deriving session keys or loading secrets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingIdentity {
    pub platform: String,
    pub transport_profile: String,
    pub runtime_profile: String,
    pub chat_id: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    pub is_dm: bool,
    pub session_key: String,
}

pub fn resolve_identity(
    platform: &str,
    transport_profile: &str,
    routed_profile: Option<&str>,
    chat_id: &str,
    user_id: &str,
    thread_id: Option<&str>,
    is_dm: bool,
) -> RoutingIdentity {
    let runtime_profile = routed_profile
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(transport_profile)
        .to_string();

    let scope_ns = if runtime_profile == "default" {
        "agent:main".to_string()
    } else {
        format!("agent:{runtime_profile}")
    };

    let session_key = match thread_id.filter(|t| !t.is_empty()) {
        Some(tid) => format!("{scope_ns}:{platform}:{chat_id}:{tid}"),
        None => format!("{scope_ns}:{platform}:{chat_id}"),
    };

    RoutingIdentity {
        platform: platform.to_string(),
        transport_profile: transport_profile.to_string(),
        runtime_profile,
        chat_id: chat_id.to_string(),
        user_id: user_id.to_string(),
        thread_id: thread_id.map(ToString::to_string),
        is_dm,
        session_key,
    }
}
