use crate::streaming::DeliveryAction;
use saif_config::SecretScope;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct PlatformSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub secret_env_vars: &'static [&'static str],
    pub max_chars: usize,
    pub supports_edit: bool,
    pub webhook_only: bool,
    pub serves_profile_prefix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct InboundPlatformEvent {
    pub platform: String,
    pub chat_id: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    pub text: String,
    pub is_dm: bool,
    pub routed_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboundHttpSpec {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

pub fn builtin_platform_specs() -> Vec<PlatformSpec> {
    vec![
        PlatformSpec {
            id: "telegram",
            display_name: "Telegram",
            secret_env_vars: &["TELEGRAM_BOT_TOKEN"],
            max_chars: 4096,
            supports_edit: true,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "discord",
            display_name: "Discord",
            secret_env_vars: &["DISCORD_BOT_TOKEN"],
            max_chars: 2000,
            supports_edit: true,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "slack",
            display_name: "Slack",
            secret_env_vars: &["SLACK_BOT_TOKEN", "SLACK_SIGNING_SECRET"],
            max_chars: 4000,
            supports_edit: true,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "whatsapp",
            display_name: "WhatsApp Bridge",
            secret_env_vars: &["WHATSAPP_BRIDGE_URL"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "whatsapp_cloud",
            display_name: "WhatsApp Cloud API",
            secret_env_vars: &["WHATSAPP_CLOUD_TOKEN", "WHATSAPP_PHONE_NUMBER_ID"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "signal",
            display_name: "Signal",
            secret_env_vars: &["SIGNAL_CLI_URL"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "matrix",
            display_name: "Matrix",
            secret_env_vars: &["MATRIX_HOMESERVER", "MATRIX_ACCESS_TOKEN"],
            max_chars: 16384,
            supports_edit: true,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "email",
            display_name: "Email (IMAP/SMTP)",
            secret_env_vars: &["EMAIL_IMAP_PASSWORD", "EMAIL_SMTP_PASSWORD"],
            max_chars: 65536,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "feishu",
            display_name: "Feishu / Lark",
            secret_env_vars: &["FEISHU_APP_ID", "FEISHU_APP_SECRET"],
            max_chars: 8192,
            supports_edit: true,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "irc",
            display_name: "IRC",
            secret_env_vars: &["IRC_SERVER", "IRC_NICK"],
            max_chars: 400,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "mattermost",
            display_name: "Mattermost",
            secret_env_vars: &["MATTERMOST_URL", "MATTERMOST_BOT_TOKEN"],
            max_chars: 4000,
            supports_edit: true,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "google_chat",
            display_name: "Google Chat",
            secret_env_vars: &["GOOGLE_CHAT_CREDENTIALS"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "line",
            display_name: "LINE",
            secret_env_vars: &["LINE_CHANNEL_ACCESS_TOKEN", "LINE_CHANNEL_SECRET"],
            max_chars: 5000,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "teams",
            display_name: "Microsoft Teams",
            secret_env_vars: &["TEAMS_APP_ID", "TEAMS_APP_PASSWORD"],
            max_chars: 16384,
            supports_edit: true,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "wecom",
            display_name: "WeCom",
            secret_env_vars: &["WECOM_CORP_ID", "WECOM_SECRET"],
            max_chars: 2048,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "weixin",
            display_name: "Weixin",
            secret_env_vars: &["WEIXIN_TOKEN"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "qqbot",
            display_name: "QQBot",
            secret_env_vars: &["QQBOT_APP_ID", "QQBOT_TOKEN"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "dingtalk",
            display_name: "DingTalk",
            secret_env_vars: &["DINGTALK_APP_KEY", "DINGTALK_APP_SECRET"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "sms",
            display_name: "SMS (Twilio)",
            secret_env_vars: &["TWILIO_ACCOUNT_SID", "TWILIO_AUTH_TOKEN"],
            max_chars: 1600,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "a2a",
            display_name: "Agent2Agent (A2A)",
            secret_env_vars: &["A2A_SHARED_SECRET"],
            max_chars: 32768,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "bluebubbles",
            display_name: "BlueBubbles (iMessage)",
            secret_env_vars: &["BLUEBUBBLES_URL", "BLUEBUBBLES_PASSWORD"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "buzz",
            display_name: "Buzz",
            secret_env_vars: &["BUZZ_API_KEY"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "homeassistant",
            display_name: "Home Assistant",
            secret_env_vars: &["HOMEASSISTANT_URL", "HOMEASSISTANT_TOKEN"],
            max_chars: 8192,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "ntfy",
            display_name: "ntfy",
            secret_env_vars: &["NTFY_URL", "NTFY_TOKEN"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "photon",
            display_name: "Photon",
            secret_env_vars: &["PHOTON_BRIDGE_URL"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "raft",
            display_name: "Raft",
            secret_env_vars: &["RAFT_TOKEN"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "simplex",
            display_name: "SimpleX",
            secret_env_vars: &["SIMPLEX_WS_URL"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "yuanbao",
            display_name: "Yuanbao",
            secret_env_vars: &["YUANBAO_TOKEN"],
            max_chars: 4096,
            supports_edit: false,
            webhook_only: false,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "webhook",
            display_name: "Generic Webhook",
            secret_env_vars: &["WEBHOOK_SECRET"],
            max_chars: 32768,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
        PlatformSpec {
            id: "api_server",
            display_name: "OpenAI API Server",
            secret_env_vars: &["API_SERVER_KEY"],
            max_chars: 65536,
            supports_edit: false,
            webhook_only: true,
            serves_profile_prefix: true,
        },
    ]
}

pub fn get_platform_spec(id: &str) -> Option<PlatformSpec> {
    let needle = id.trim().to_ascii_lowercase();
    builtin_platform_specs()
        .into_iter()
        .find(|p| p.id == needle)
}

pub fn chunk_for_platform(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 || text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for ch in text.chars() {
        if cur_len >= max_chars {
            chunks.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        cur.push(ch);
        cur_len += 1;
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

fn s_at(v: &Value, ptr: &str) -> Option<String> {
    match v.pointer(ptr) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// Normalize inbound webhook JSON across all 30 supported Hermes platforms into `InboundPlatformEvent`.
pub fn parse_inbound_webhook(
    platform: &str,
    payload: &Value,
) -> Result<Option<InboundPlatformEvent>, String> {
    let p = platform.trim().to_ascii_lowercase();
    if get_platform_spec(&p).is_none() {
        return Err(format!("unsupported platform '{platform}'"));
    }
    let routed_profile = s_at(payload, "/profile");

    let event = match p.as_str() {
        "telegram" => {
            let Some(msg) = payload.get("message") else {
                return Ok(None);
            };
            let chat_id = s_at(msg, "/chat/id").unwrap_or_default();
            let user_id = s_at(msg, "/from/id").unwrap_or_else(|| chat_id.clone());
            let text = s_at(msg, "/text").unwrap_or_default();
            let thread_id = s_at(msg, "/message_thread_id");
            let is_dm = s_at(msg, "/chat/type").as_deref() == Some("private");
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id,
                text,
                is_dm,
                routed_profile,
            }
        }
        "discord" => {
            let chat_id = s_at(payload, "/channel_id").unwrap_or_default();
            let user_id = s_at(payload, "/author/id").unwrap_or_else(|| chat_id.clone());
            let text = s_at(payload, "/content").unwrap_or_default();
            let thread_id = s_at(payload, "/thread_id");
            let is_dm = payload.get("guild_id").map(|g| g.is_null()).unwrap_or(true);
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id,
                text,
                is_dm,
                routed_profile,
            }
        }
        "slack" => {
            let ev = payload.get("event").unwrap_or(payload);
            let chat_id = s_at(ev, "/channel").unwrap_or_default();
            let user_id = s_at(ev, "/user").unwrap_or_else(|| chat_id.clone());
            let text = s_at(ev, "/text").unwrap_or_default();
            let thread_id = s_at(ev, "/thread_ts");
            let is_dm = s_at(ev, "/channel_type").as_deref() == Some("im");
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id,
                text,
                is_dm,
                routed_profile,
            }
        }
        "whatsapp_cloud" => {
            let msg = payload
                .pointer("/entry/0/changes/0/value/messages/0")
                .unwrap_or(payload);
            let user_id = s_at(msg, "/from").unwrap_or_default();
            let text = s_at(msg, "/text/body")
                .or_else(|| s_at(msg, "/text"))
                .unwrap_or_default();
            if user_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id: user_id.clone(),
                user_id,
                thread_id: None,
                text,
                is_dm: true,
                routed_profile,
            }
        }
        "matrix" => {
            let chat_id = s_at(payload, "/room_id").unwrap_or_default();
            let user_id = s_at(payload, "/sender").unwrap_or_else(|| chat_id.clone());
            let text = s_at(payload, "/content/body")
                .or_else(|| s_at(payload, "/text"))
                .unwrap_or_default();
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id: None,
                text,
                is_dm: false,
                routed_profile,
            }
        }
        "a2a" => {
            let text = s_at(payload, "/params/message/parts/0/text")
                .or_else(|| s_at(payload, "/text"))
                .unwrap_or_default();
            let chat_id = s_at(payload, "/params/contextId")
                .or_else(|| s_at(payload, "/params/sessionId"))
                .or_else(|| s_at(payload, "/id"))
                .unwrap_or_else(|| "a2a-default".to_string());
            let user_id = s_at(payload, "/params/sender").unwrap_or_else(|| "a2a-peer".to_string());
            if text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id: None,
                text,
                is_dm: true,
                routed_profile,
            }
        }
        "line" => {
            let ev = payload.pointer("/events/0").unwrap_or(payload);
            let user_id = s_at(ev, "/source/userId").unwrap_or_default();
            let chat_id = s_at(ev, "/source/groupId")
                .or_else(|| s_at(ev, "/source/roomId"))
                .unwrap_or_else(|| user_id.clone());
            let text = s_at(ev, "/message/text").unwrap_or_default();
            let is_dm = s_at(ev, "/source/type").as_deref() == Some("user");
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id: None,
                text,
                is_dm,
                routed_profile,
            }
        }
        "google_chat" => {
            let chat_id = s_at(payload, "/space/name").unwrap_or_default();
            let user_id = s_at(payload, "/user/name").unwrap_or_else(|| chat_id.clone());
            let text = s_at(payload, "/message/text").unwrap_or_default();
            let is_dm = s_at(payload, "/space/type").as_deref() == Some("DM");
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id: s_at(payload, "/message/thread/name"),
                text,
                is_dm,
                routed_profile,
            }
        }
        "feishu" => {
            let chat_id = s_at(payload, "/event/message/chat_id")
                .or_else(|| s_at(payload, "/chat_id"))
                .unwrap_or_default();
            let user_id =
                s_at(payload, "/event/sender/sender_id/open_id").unwrap_or_else(|| chat_id.clone());
            let text = s_at(payload, "/event/message/content")
                .or_else(|| s_at(payload, "/text"))
                .unwrap_or_default();
            if chat_id.is_empty() || text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id: None,
                text,
                is_dm: true,
                routed_profile,
            }
        }
        _ => {
            let chat_id = s_at(payload, "/chat_id")
                .or_else(|| s_at(payload, "/channel_id"))
                .or_else(|| s_at(payload, "/conversation/id"))
                .or_else(|| s_at(payload, "/topic"))
                .unwrap_or_else(|| "default".to_string());
            let user_id = s_at(payload, "/user_id")
                .or_else(|| s_at(payload, "/from/id"))
                .or_else(|| s_at(payload, "/sender"))
                .unwrap_or_else(|| chat_id.clone());
            let text = s_at(payload, "/text")
                .or_else(|| s_at(payload, "/prompt"))
                .or_else(|| s_at(payload, "/message"))
                .or_else(|| s_at(payload, "/content"))
                .unwrap_or_default();
            if text.is_empty() {
                return Ok(None);
            }
            InboundPlatformEvent {
                platform: p,
                chat_id,
                user_id,
                thread_id: s_at(payload, "/thread_id"),
                text,
                is_dm: payload
                    .get("is_dm")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                routed_profile,
            }
        }
    };

    Ok(Some(event))
}

pub fn build_outbound_request(
    platform: &str,
    secrets: &SecretScope,
    chat_id: &str,
    action: &DeliveryAction,
) -> Result<OutboundHttpSpec, String> {
    let text = match action {
        DeliveryAction::SendMessage { text }
        | DeliveryAction::InterimSend { text }
        | DeliveryAction::EditMessage { text, .. }
        | DeliveryAction::UpdateStream { frame: text, .. } => text.clone(),
    };
    let mut headers = BTreeMap::new();
    match platform.trim().to_ascii_lowercase().as_str() {
        "telegram" => {
            let tok = secrets
                .require_secret("TELEGRAM_BOT_TOKEN")
                .map_err(|e| e.to_string())?;
            Ok(OutboundHttpSpec {
                url: format!("https://api.telegram.org/bot{tok}/sendMessage"),
                headers,
                body: json!({ "chat_id": chat_id, "text": text }),
            })
        }
        "discord" => {
            let tok = secrets
                .require_secret("DISCORD_BOT_TOKEN")
                .map_err(|e| e.to_string())?;
            headers.insert("Authorization".into(), format!("Bot {tok}"));
            Ok(OutboundHttpSpec {
                url: format!("https://discord.com/api/v10/channels/{chat_id}/messages"),
                headers,
                body: json!({ "content": text }),
            })
        }
        "slack" => {
            let tok = secrets
                .require_secret("SLACK_BOT_TOKEN")
                .map_err(|e| e.to_string())?;
            headers.insert("Authorization".into(), format!("Bearer {tok}"));
            Ok(OutboundHttpSpec {
                url: "https://slack.com/api/chat.postMessage".to_string(),
                headers,
                body: json!({ "channel": chat_id, "text": text }),
            })
        }
        "mattermost" => {
            let base = secrets
                .require_secret("MATTERMOST_URL")
                .map_err(|e| e.to_string())?;
            let tok = secrets
                .require_secret("MATTERMOST_BOT_TOKEN")
                .map_err(|e| e.to_string())?;
            headers.insert("Authorization".into(), format!("Bearer {tok}"));
            Ok(OutboundHttpSpec {
                url: format!("{}/api/v4/posts", base.trim_end_matches('/')),
                headers,
                body: json!({ "channel_id": chat_id, "message": text }),
            })
        }
        other => Ok(OutboundHttpSpec {
            url: format!("https://localhost/webhooks/outbound/{other}/{chat_id}"),
            headers,
            body: json!({ "chat_id": chat_id, "text": text }),
        }),
    }
}
