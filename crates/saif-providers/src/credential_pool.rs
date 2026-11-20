use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const EXHAUSTED_TTL_401_SECONDS: u64 = 5 * 60;
pub const EXHAUSTED_TTL_429_SECONDS: u64 = 60 * 60;
pub const EXHAUSTED_TTL_DEFAULT_SECONDS: u64 = 60 * 60;
pub const EXHAUSTED_TTL_SOLE_CREDENTIAL_SECONDS: u64 = 60;

const TERMINAL_AUTH_REASONS: &[&str] = &[
    "token_invalidated",
    "token_revoked",
    "invalid_token",
    "invalid_grant",
    "unauthorized_client",
    "refresh_token_reused",
];

pub fn is_terminal_auth_reason(reason: &str) -> bool {
    let lower = reason.trim().to_ascii_lowercase();
    TERMINAL_AUTH_REASONS.contains(&lower.as_str())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PoolStrategy {
    #[default]
    FillFirst,
    RoundRobin,
    Random,
    LeastUsed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CredStatus {
    #[default]
    Ok,
    Exhausted,
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PooledCredential {
    pub id: String,
    pub provider: String,
    pub secret: String,
    pub source: String,
    pub status: CredStatus,
    pub cooldown_until: Option<u64>,
    pub failure_reason: Option<String>,
    pub use_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PoolDocument {
    pub strategy: PoolStrategy,
    pub providers: BTreeMap<String, Vec<PooledCredential>>,
    #[serde(default)]
    pub rr_cursors: BTreeMap<String, usize>,
}

/// Persistent same-provider multi-credential failover pool (`agent/credential_pool.py` parity).
#[derive(Debug, Clone)]
pub struct CredentialPool {
    path: PathBuf,
    doc: PoolDocument,
}

impl CredentialPool {
    pub fn open(saif_home: &Path) -> Self {
        let path = saif_home.join("auth_pool.json");
        let doc = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, doc }
    }

    pub fn strategy(&self) -> PoolStrategy {
        self.doc.strategy
    }

    pub fn set_strategy(&mut self, strategy: PoolStrategy) {
        self.doc.strategy = strategy;
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&self.doc)?;
        let tmp = self
            .path
            .with_extension(format!("json.tmp.{}", std::process::id()));
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(tmp, &self.path)
    }

    pub fn add_credential(&mut self, provider: &str, id: &str, secret: &str, source: &str) {
        let entries = self.doc.providers.entry(provider.to_string()).or_default();
        if let Some(existing) = entries.iter_mut().find(|c| c.id == id) {
            existing.secret = secret.to_string();
            existing.source = source.to_string();
            existing.status = CredStatus::Ok;
            existing.cooldown_until = None;
            existing.failure_reason = None;
        } else {
            entries.push(PooledCredential {
                id: id.to_string(),
                provider: provider.to_string(),
                secret: secret.to_string(),
                source: source.to_string(),
                status: CredStatus::Ok,
                cooldown_until: None,
                failure_reason: None,
                use_count: 0,
            });
        }
    }

    pub fn list(&self, provider: Option<&str>) -> Vec<PooledCredential> {
        match provider {
            Some(p) => self.doc.providers.get(p).cloned().unwrap_or_default(),
            None => self.doc.providers.values().flatten().cloned().collect(),
        }
    }

    pub fn select(&mut self, provider: &str, now_epoch: u64) -> Option<PooledCredential> {
        let entries = self.doc.providers.get_mut(provider)?;
        for entry in entries.iter_mut() {
            if entry.status == CredStatus::Exhausted {
                if let Some(until) = entry.cooldown_until {
                    if now_epoch >= until {
                        entry.status = CredStatus::Ok;
                        entry.cooldown_until = None;
                        entry.failure_reason = None;
                    }
                }
            }
        }

        let eligible_indices: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.status == CredStatus::Ok)
            .map(|(i, _)| i)
            .collect();
        if eligible_indices.is_empty() {
            return None;
        }

        let chosen_idx = match self.doc.strategy {
            PoolStrategy::FillFirst => eligible_indices[0],
            PoolStrategy::RoundRobin => {
                let cursor = self.doc.rr_cursors.entry(provider.to_string()).or_insert(0);
                let pick = eligible_indices[*cursor % eligible_indices.len()];
                *cursor = cursor.wrapping_add(1);
                pick
            }
            PoolStrategy::Random => {
                let mut rng = rand::rng();
                let pos = rng.random_range(0..eligible_indices.len());
                eligible_indices[pos]
            }
            PoolStrategy::LeastUsed => *eligible_indices
                .iter()
                .min_by_key(|&&idx| entries[idx].use_count)
                .unwrap_or(&eligible_indices[0]),
        };

        entries[chosen_idx].use_count += 1;
        Some(entries[chosen_idx].clone())
    }

    pub fn record_failure(
        &mut self,
        provider: &str,
        id: &str,
        http_status: Option<u16>,
        reason: Option<&str>,
        now_epoch: u64,
    ) {
        let Some(entries) = self.doc.providers.get_mut(provider) else {
            return;
        };
        let non_dead_count = entries
            .iter()
            .filter(|e| e.status != CredStatus::Dead)
            .count();
        let Some(entry) = entries.iter_mut().find(|e| e.id == id) else {
            return;
        };

        if let Some(r) = reason {
            entry.failure_reason = Some(r.to_string());
            if is_terminal_auth_reason(r) {
                entry.status = CredStatus::Dead;
                entry.cooldown_until = None;
                return;
            }
        }

        let ttl = if non_dead_count <= 1 {
            EXHAUSTED_TTL_SOLE_CREDENTIAL_SECONDS
        } else {
            match http_status {
                Some(401) => EXHAUSTED_TTL_401_SECONDS,
                Some(429) => EXHAUSTED_TTL_429_SECONDS,
                _ => EXHAUSTED_TTL_DEFAULT_SECONDS,
            }
        };
        entry.status = CredStatus::Exhausted;
        entry.cooldown_until = Some(now_epoch.saturating_add(ttl));
    }
}

/// Read a migrated or OAuth credential from `<saif_home>/auth.json` if present.
pub fn read_auth_json_secret(saif_home: &Path, provider_id: &str) -> Option<String> {
    let raw = fs::read_to_string(saif_home.join("auth.json")).ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    if let Some(entry) = val
        .get(provider_id)
        .or_else(|| val.pointer(&format!("/providers/{provider_id}")))
    {
        if let Some(tok) = extract_token_field(entry) {
            return Some(tok);
        }
    }
    if val.get("provider").and_then(|p| p.as_str()) == Some(provider_id) {
        return extract_token_field(&val);
    }
    None
}

fn extract_token_field(entry: &serde_json::Value) -> Option<String> {
    if let Some(s) = entry.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    for key in ["api_key", "access_token", "token", "key"] {
        if let Some(s) = entry.get(key).and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
