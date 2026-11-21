use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;
use thiserror::Error;

pub const SENSITIVE_ENV_VARS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENROUTER_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "XAI_API_KEY",
    "DEEPSEEK_API_KEY",
    "GROQ_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "DEEPINFRA_API_KEY",
    "DASHSCOPE_API_KEY",
    "NVIDIA_API_KEY",
    "MISTRAL_API_KEY",
    "MINIMAX_API_KEY",
    "MOONSHOT_API_KEY",
    "NOUS_API_KEY",
    "TELEGRAM_BOT_TOKEN",
    "DISCORD_BOT_TOKEN",
    "SLACK_BOT_TOKEN",
    "SLACK_APP_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "HERMES_HOME",
    "HERMES_MANAGED_DIR",
    "OP_SERVICE_ACCOUNT_TOKEN",
    "VAULT_TOKEN",
    "BW_SESSION",
];

pub const SENSITIVE_ENV_PREFIXES: &[&str] = &[
    "SAIF_INTERNAL_",
    "HERMES_INTERNAL_",
    "SAIF_GATEWAY_",
    "TELEGRAM_",
    "DISCORD_",
    "SLACK_",
    "HERMES_TUI_",
    "SAIF_TUI_",
    "A2A_",
];

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("unscoped or fail-closed secret access for '{key}' under multiplexed profile")]
    FailClosedMiss { key: String },
}

/// Fail-closed per-profile secret store (`agent/secret_scope.py` parity).
/// When `multiplex_active` is true AND `!is_default`, lookups NEVER fall back to `std::env::var`.
#[derive(Debug, Clone)]
pub struct SecretScope {
    values: BTreeMap<String, String>,
    is_default: bool,
    multiplex_active: bool,
}

impl SecretScope {
    pub fn empty(is_default: bool, multiplex_active: bool) -> Self {
        Self {
            values: BTreeMap::new(),
            is_default,
            multiplex_active,
        }
    }

    pub fn from_map(
        values: BTreeMap<String, String>,
        is_default: bool,
        multiplex_active: bool,
    ) -> Self {
        Self {
            values,
            is_default,
            multiplex_active,
        }
    }

    pub fn load_from_home(home_dir: &Path, is_default: bool, multiplex_active: bool) -> Self {
        let env_path = home_dir.join(".env");
        let mut values = BTreeMap::new();
        if let Ok(iter) = dotenvy::from_path_iter(&env_path) {
            for item in iter.flatten() {
                values.insert(item.0, item.1);
            }
        }
        Self {
            values,
            is_default,
            multiplex_active,
        }
    }

    /// Read a secret respecting multiplex fail-closed boundaries.
    pub fn get_secret(&self, key: &str) -> Option<String> {
        if let Some(val) = self.values.get(key) {
            if !val.trim().is_empty() {
                return Some(val.clone());
            }
        }
        if self.multiplex_active && !self.is_default {
            return None;
        }
        env::var(key).ok().filter(|v| !v.trim().is_empty())
    }

    pub fn require_secret(&self, key: &str) -> Result<String, SecretError> {
        self.get_secret(key)
            .ok_or_else(|| SecretError::FailClosedMiss {
                key: key.to_string(),
            })
    }

    pub fn keys(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }

    pub fn is_multiplex(&self) -> bool {
        self.multiplex_active
    }

    /// Build a scrubbed environment for terminal/tool/MCP/delegation children
    /// (`served_profile_child_env` / `build_subprocess_env` parity).
    pub fn build_child_env(
        &self,
        saif_home: &Path,
        passthrough_vars: &[String],
    ) -> BTreeMap<String, String> {
        let allow_set: HashSet<&str> = passthrough_vars.iter().map(String::as_str).collect();
        let mut child = BTreeMap::new();

        for (k, v) in env::vars() {
            if allow_set.contains(k.as_str()) {
                child.insert(k, v);
                continue;
            }
            if self.values.contains_key(&k) || is_sensitive_var(&k) {
                continue;
            }
            child.insert(k, v);
        }

        // Overlay explicitly allowed passthrough secrets from THIS profile's scope only.
        for key in passthrough_vars {
            if let Some(val) = self.get_secret(key) {
                child.insert(key.clone(), val);
            }
        }

        child.insert(
            "SAIF_HOME".to_string(),
            saif_home.to_string_lossy().into_owned(),
        );
        child
    }

    /// Persist a single key=value update cleanly into `<home>/.env` with 0600 permissions on Unix.
    pub fn write_secret_to_env(home_dir: &Path, key: &str, value: &str) -> std::io::Result<()> {
        fs::create_dir_all(home_dir)?;
        let path = home_dir.join(".env");
        let existing = fs::read_to_string(&path).unwrap_or_default();
        let mut lines: Vec<String> = Vec::new();
        let mut replaced = false;
        for line in existing.lines() {
            if let Some((k, _)) = line.split_once('=') {
                if k.trim() == key {
                    lines.push(format!("{key}={value}"));
                    replaced = true;
                    continue;
                }
            }
            lines.push(line.to_string());
        }
        if !replaced {
            lines.push(format!("{key}={value}"));
        }
        let body = lines.join("\n") + "\n";
        let tmp = home_dir.join(format!(".env.tmp.{}", std::process::id()));
        fs::write(&tmp, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(tmp, path)
    }
}

fn is_sensitive_var(key: &str) -> bool {
    if SENSITIVE_ENV_VARS.contains(&key) {
        return true;
    }
    if SENSITIVE_ENV_PREFIXES.iter().any(|p| key.starts_with(p)) {
        return true;
    }
    let upper = key.to_ascii_uppercase();
    upper.ends_with("_API_KEY")
        || upper.ends_with("_BOT_TOKEN")
        || upper.ends_with("_APP_SECRET")
        || upper.ends_with("_SECRET_KEY")
        || upper.ends_with("_ACCESS_TOKEN")
        || upper.ends_with("_AUTH_TOKEN")
        || upper.ends_with("_SESSION_TOKEN")
        || upper.ends_with("_BEARER_TOKEN")
}
