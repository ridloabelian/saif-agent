use crate::guards::{BusyDecision, BusyMessageGuard};
use crate::identity::{resolve_identity, RoutingIdentity};
use crate::platforms::{
    builtin_platform_specs, chunk_for_platform, get_platform_spec, parse_inbound_webhook,
};
use crate::streaming::{DeliveryAction, StreamDraftController};
use saif_config::{list_live_profiles, ProfileScope, SaifConfig};
use saif_core::{AgentRunner, TurnOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ProfileGatewaySlot {
    pub profile: Arc<ProfileScope>,
    pub config: Arc<SaifConfig>,
    pub agent: AgentRunner,
    pub guard: Arc<Mutex<BusyMessageGuard>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiplexOutcome {
    pub accepted: bool,
    pub queued_position: Option<usize>,
    pub identity: RoutingIdentity,
    pub session_id: String,
    pub chunks: Vec<String>,
    pub delivery_action_kind: String,
}

/// Hosts every profile inside one gateway process with fail-closed credential checks (`gateway/run.py`).
#[derive(Clone)]
pub struct GatewayMultiplexer {
    slots: BTreeMap<String, ProfileGatewaySlot>,
    default_profile: String,
}

impl GatewayMultiplexer {
    pub fn new(default_profile: impl Into<String>) -> Self {
        Self {
            slots: BTreeMap::new(),
            default_profile: default_profile.into(),
        }
    }

    pub fn add_slot(
        &mut self,
        profile: Arc<ProfileScope>,
        config: Arc<SaifConfig>,
        agent: AgentRunner,
    ) {
        let name = profile.profile_name.clone();
        self.slots.insert(
            name,
            ProfileGatewaySlot {
                profile,
                config,
                agent,
                guard: Arc::new(Mutex::new(BusyMessageGuard::default())),
            },
        );
    }

    pub fn slot_names(&self) -> Vec<String> {
        self.slots.keys().cloned().collect()
    }

    /// Detect shared platform bot credentials across distinct profiles (`_adapter_credential_fingerprint`).
    pub fn preflight_credential_conflicts(&self) -> Result<(), String> {
        let mut seen: HashMap<(String, String), String> = HashMap::new();
        for (prof_name, slot) in &self.slots {
            for spec in builtin_platform_specs() {
                for env_var in spec.secret_env_vars {
                    if let Some(secret) = slot.profile.secrets.get_secret(env_var) {
                        let fp = hex::encode(Sha256::digest(secret.as_bytes()));
                        let key = (format!("{}:{env_var}", spec.id), fp);
                        if let Some(other_prof) = seen.get(&key) {
                            if other_prof != prof_name {
                                return Err(format!(
                                    "Multiplex credential conflict: profiles '{other_prof}' and '{prof_name}' share identical {env_var} for platform '{}'",
                                    spec.id
                                ));
                            }
                        } else {
                            seen.insert(key, prof_name.clone());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn handle_inbound_event(
        &self,
        platform: &str,
        payload: &Value,
        cwd: &Path,
    ) -> Result<MultiplexOutcome, String> {
        let Some(ev) = parse_inbound_webhook(platform, payload)? else {
            return Err("ignored empty or non-text platform webhook event".to_string());
        };
        let target_profile = ev
            .routed_profile
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or(&self.default_profile);
        let slot = self
            .slots
            .get(target_profile)
            .ok_or_else(|| format!("no gateway slot registered for profile '{target_profile}'"))?
            .clone();

        let identity = resolve_identity(
            &ev.platform,
            &slot.profile.profile_name,
            ev.routed_profile.as_deref(),
            &ev.chat_id,
            &ev.user_id,
            ev.thread_id.as_deref(),
            ev.is_dm,
        );

        let decision = {
            let mut guard = slot
                .guard
                .lock()
                .map_err(|_| "busy guard lock poisoned".to_string())?;
            let d = guard.admit(&identity.session_key, &ev.text);
            if matches!(d, BusyDecision::ExecuteImmediately) {
                guard.mark_active(&identity.session_key);
            }
            d
        };

        match decision {
            BusyDecision::Queued { position } => {
                return Ok(MultiplexOutcome {
                    accepted: true,
                    queued_position: Some(position),
                    identity,
                    session_id: String::new(),
                    chunks: Vec::new(),
                    delivery_action_kind: "queued".to_string(),
                });
            }
            BusyDecision::ControlBypass(cmd) => {
                return Ok(MultiplexOutcome {
                    accepted: true,
                    queued_position: None,
                    identity,
                    session_id: String::new(),
                    chunks: vec![format!("Control acknowledged: {cmd:?}")],
                    delivery_action_kind: "control_bypass".to_string(),
                });
            }
            BusyDecision::ExecuteImmediately => {}
        }

        let scope = Arc::clone(&slot.profile);
        let agent = slot.agent.clone();
        let session_key = identity.session_key.clone();
        let prompt_text = ev.text.clone();
        let platform_name = ev.platform.clone();
        let cwd_buf = cwd.to_path_buf();

        let turn_result = ProfileScope::run_scoped(scope, async move {
            let opts = TurnOptions {
                session_id: Some(session_key),
                source: platform_name,
                cwd: cwd_buf,
                ..TurnOptions::default()
            };
            agent
                .run_conversation(
                    &prompt_text,
                    None,
                    opts,
                    None,
                    CancellationToken::new(),
                    None,
                )
                .await
        })
        .await;

        if let Ok(mut guard) = slot.guard.lock() {
            let _ = guard.mark_idle(&identity.session_key);
        }

        let outcome = turn_result.map_err(|e| e.to_string())?;
        let mut draft = StreamDraftController::new();
        let action = draft.finish(&outcome.final_response);
        let action_kind = match action {
            DeliveryAction::SendMessage { .. } => "send_message",
            DeliveryAction::EditMessage { .. } => "edit_message",
            DeliveryAction::InterimSend { .. } => "interim_send",
            DeliveryAction::UpdateStream { .. } => "update_stream",
        }
        .to_string();

        let max_chars = get_platform_spec(&ev.platform)
            .map(|s| s.max_chars)
            .unwrap_or(4096);
        let chunks = chunk_for_platform(&outcome.final_response, max_chars);

        Ok(MultiplexOutcome {
            accepted: true,
            queued_position: None,
            identity,
            session_id: outcome.session_id,
            chunks,
            delivery_action_kind: action_kind,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationManifest {
    pub status: String,
    pub saif_root: String,
    pub converged_profiles: Vec<String>,
}

/// Converge legacy per-profile gateways onto the single multiplexed host gateway (`gateway_migrate.py`).
pub fn converge_multiplex_migration(saif_root: &Path) -> Result<MigrationManifest, String> {
    fs::create_dir_all(saif_root).map_err(|e| e.to_string())?;
    let mut converged_profiles = vec!["default".to_string()];
    for (name, _) in list_live_profiles(saif_root) {
        converged_profiles.push(name);
    }
    let manifest = MigrationManifest {
        status: "converged".to_string(),
        saif_root: saif_root.display().to_string(),
        converged_profiles,
    };
    let manifest_path = saif_root.join("gateway_migration.json");
    let body = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(manifest_path, body).map_err(|e| e.to_string())?;
    Ok(manifest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Systemd,
    Launchd,
    WindowsTask,
}

impl ServiceKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "systemd" => Ok(Self::Systemd),
            "launchd" => Ok(Self::Launchd),
            "windows" | "windows_task" => Ok(Self::WindowsTask),
            other => Err(format!("unsupported service kind '{other}'")),
        }
    }
}

pub fn render_service_unit(
    kind: ServiceKind,
    saif_bin: &Path,
    saif_root: &Path,
    listen: SocketAddr,
) -> String {
    let bin = saif_bin.display();
    let root = saif_root.display();
    match kind {
        ServiceKind::Systemd => format!(
            "[Unit]\nDescription=Saif Agent Multiplexed Gateway\nAfter=network-online.target\n\n\
             [Service]\nType=simple\nEnvironment=SAIF_HOME={root}\n\
             ExecStart={bin} gateway run --listen {listen}\nRestart=always\nRestartSec=5\n\n\
             [Install]\nWantedBy=default.target\n"
        ),
        ServiceKind::Launchd => format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n<key>Label</key><string>id.co.saif.gateway</string>\n\
             <key>ProgramArguments</key><array><string>{bin}</string><string>gateway</string><string>run</string><string>--listen</string><string>{listen}</string></array>\n\
             <key>EnvironmentVariables</key><dict><key>SAIF_HOME</key><string>{root}</string></dict>\n\
             <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\n</dict></plist>\n"
        ),
        ServiceKind::WindowsTask => format!(
            "schtasks /Create /TN \"SaifAgentGateway\" /SC ONLOGON /RL HIGHEST /TR \"\\\"{bin}\\\" gateway run --listen {listen}\" /F\n"
        ),
    }
}
