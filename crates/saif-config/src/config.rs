use crate::secrets::SecretScope;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed reading config {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed parsing YAML config {path}: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SaifConfig {
    pub _config_version: u32,
    pub model: String,
    pub provider: String,
    pub base_url: Option<String>,
    pub api_mode: Option<String>,
    pub toolsets: Vec<String>,
    pub fallback_providers: Vec<String>,
    pub max_live_sessions: usize,
    pub agent: AgentSection,
    pub terminal: TerminalSection,
    pub compression: CompressionSection,
    pub display: DisplaySection,
    pub memory: MemorySection,
    pub security: SecuritySection,
    pub delegation: DelegationSection,
    pub auxiliary: AuxiliarySection,
    pub curator: CuratorSection,
    pub skills: SkillsSection,
    pub gateway: GatewaySection,
    pub cron: CronSection,
    pub plugins: PluginsSection,
    pub database: DatabaseSection,
    #[serde(flatten)]
    pub extra: BTreeMap<String, JsonValue>,
}

impl Default for SaifConfig {
    fn default() -> Self {
        Self {
            _config_version: 49,
            model: String::new(),
            provider: String::new(),
            base_url: None,
            api_mode: None,
            toolsets: vec!["hermes-cli".to_string()],
            fallback_providers: Vec::new(),
            max_live_sessions: 16,
            agent: AgentSection::default(),
            terminal: TerminalSection::default(),
            compression: CompressionSection::default(),
            display: DisplaySection::default(),
            memory: MemorySection::default(),
            security: SecuritySection::default(),
            delegation: DelegationSection::default(),
            auxiliary: AuxiliarySection::default(),
            curator: CuratorSection::default(),
            skills: SkillsSection::default(),
            gateway: GatewaySection::default(),
            cron: CronSection::default(),
            plugins: PluginsSection::default(),
            database: DatabaseSection::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSection {
    pub max_iterations: usize,
    pub max_turns: Option<usize>,
    pub gateway_timeout: u64,
    pub api_retries: usize,
    pub reasoning_effort: Option<String>,
    pub context_file_max_chars: usize,
    pub context_file_read_timeout: f64,
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            max_iterations: 500,
            max_turns: None,
            gateway_timeout: 1800,
            api_retries: 3,
            reasoning_effort: None,
            context_file_max_chars: 32_000,
            context_file_read_timeout: 5.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSection {
    pub backend: String,
    pub cwd: Option<String>,
    pub timeout: u64,
    pub daemon_term_grace_seconds: f64,
    pub docker_image: String,
    pub env_passthrough: Vec<String>,
}

impl Default for TerminalSection {
    fn default() -> Self {
        Self {
            backend: "local".to_string(),
            cwd: None,
            timeout: 180,
            daemon_term_grace_seconds: 5.0,
            docker_image: "nousresearch/hermes-sandbox:desktop".to_string(),
            env_passthrough: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionSection {
    pub enabled: bool,
    pub threshold_ratio: f64,
    pub keep_tail_tokens: usize,
}

impl Default for CompressionSection {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_ratio: 0.50,
            keep_tail_tokens: 16_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySection {
    pub skin: String,
    pub language: String,
    pub compact: bool,
    pub streaming: bool,
}

impl Default for DisplaySection {
    fn default() -> Self {
        Self {
            skin: "default".to_string(),
            language: "en".to_string(),
            compact: false,
            streaming: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySection {
    pub enabled: bool,
    pub user_profile_enabled: bool,
    pub provider: String,
    pub max_chars: usize,
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            enabled: true,
            user_profile_enabled: true,
            provider: "builtin".to_string(),
            max_chars: 16_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecuritySection {
    pub approval_mode: String,
    pub allow_private_urls: bool,
}

impl Default for SecuritySection {
    fn default() -> Self {
        Self {
            approval_mode: "ask".to_string(),
            allow_private_urls: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DelegationSection {
    pub max_concurrent_children: usize,
    pub max_spawn_depth: usize,
    pub orchestrator_enabled: bool,
    pub independent_completions: bool,
    pub child_timeout_seconds: u64,
    pub max_iterations: usize,
}

impl Default for DelegationSection {
    fn default() -> Self {
        Self {
            max_concurrent_children: 3,
            max_spawn_depth: 2,
            orchestrator_enabled: false,
            independent_completions: false,
            child_timeout_seconds: 900,
            max_iterations: 200,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuxiliaryTaskConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuxiliarySection {
    pub compression: AuxiliaryTaskConfig,
    pub title_gen: AuxiliaryTaskConfig,
    pub session_search: AuxiliaryTaskConfig,
    pub curator: AuxiliaryTaskConfig,
    pub vision: AuxiliaryTaskConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CuratorSection {
    pub enabled: bool,
    pub interval_hours: u64,
    pub min_idle_hours: u64,
    pub stale_after_days: u64,
    pub archive_after_days: u64,
    pub backup_enabled: bool,
}

impl Default for CuratorSection {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_hours: 24,
            min_idle_hours: 12,
            stale_after_days: 30,
            archive_after_days: 60,
            backup_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsSection {
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewaySection {
    pub multiplex_profiles: Option<bool>,
    pub allow_all_users: bool,
    pub session_hygiene_ratio: f64,
}

impl Default for GatewaySection {
    fn default() -> Self {
        Self {
            multiplex_profiles: None,
            allow_all_users: false,
            session_hygiene_ratio: 0.85,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CronSection {
    pub inactivity_timeout_seconds: u64,
    pub catch_up_missed: bool,
    pub max_parallel_jobs: usize,
}

impl Default for CronSection {
    fn default() -> Self {
        Self {
            inactivity_timeout_seconds: 600,
            catch_up_missed: true,
            max_parallel_jobs: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsSection {
    pub isolation: String,
    pub disabled: Vec<String>,
}

impl Default for PluginsSection {
    fn default() -> Self {
        Self {
            isolation: "in_process".to_string(),
            disabled: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseSection {
    pub journal_mode: String,
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            journal_mode: "wal".to_string(),
        }
    }
}

fn env_placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap_or_else(|_| unreachable!())
    })
}

pub fn expand_env_placeholders(input: &str, secrets: Option<&SecretScope>) -> String {
    env_placeholder_re()
        .replace_all(input, |caps: &regex::Captures| {
            let key = &caps[1];
            if let Some(scope) = secrets {
                if let Some(val) = scope.get_secret(key) {
                    return val;
                }
            }
            env::var(key).unwrap_or_default()
        })
        .into_owned()
}

fn read_yaml_value(path: &Path) -> Result<Option<serde_yaml::Value>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let val: serde_yaml::Value =
        serde_yaml::from_str(&raw).map_err(|source| ConfigError::Yaml {
            path: path.display().to_string(),
            source,
        })?;
    Ok(Some(val))
}

fn merge_yaml_maps(base: &mut serde_yaml::Value, overlay: serde_yaml::Value) {
    match (base, overlay) {
        (serde_yaml::Value::Mapping(dst), serde_yaml::Value::Mapping(src)) => {
            for (k, v) in src {
                match dst.get_mut(&k) {
                    Some(existing) => merge_yaml_maps(existing, v),
                    None => {
                        dst.insert(k, v);
                    }
                }
            }
        }
        (dst, src) => *dst = src,
    }
}

fn expand_yaml_strings(val: &mut serde_yaml::Value, secrets: Option<&SecretScope>) {
    match val {
        serde_yaml::Value::String(s) => {
            *s = expand_env_placeholders(s, secrets);
        }
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                expand_yaml_strings(item, secrets);
            }
        }
        serde_yaml::Value::Mapping(map) => {
            for (_, v) in map.iter_mut() {
                expand_yaml_strings(v, secrets);
            }
        }
        _ => {}
    }
}

/// Loader 1: `load_cli_config` — merges `SaifConfig::default()` with `<home>/config.yaml`.
pub fn load_cli_config(home_dir: &Path) -> Result<SaifConfig, ConfigError> {
    load_config(home_dir)
}

/// Loader 2: `load_config` — merges defaults + `<home>/config.yaml` + optional managed overlay.
pub fn load_config(home_dir: &Path) -> Result<SaifConfig, ConfigError> {
    let defaults =
        serde_yaml::to_value(SaifConfig::default()).map_err(|source| ConfigError::Yaml {
            path: "<defaults>".to_string(),
            source,
        })?;
    let mut merged = defaults;
    let user_path = home_dir.join("config.yaml");
    if let Some(user_val) = read_yaml_value(&user_path)? {
        merge_yaml_maps(&mut merged, user_val);
    }
    let managed = Path::new("/etc/saif/config.yaml");
    if let Some(overlay) = read_yaml_value(managed)? {
        merge_yaml_maps(&mut merged, overlay);
    }
    serde_yaml::from_value(merged).map_err(|source| ConfigError::Yaml {
        path: user_path.display().to_string(),
        source,
    })
}

/// Loader 3: `load_effective_config` — user YAML + managed overlay + `${VAR}` expansion
/// WITHOUT injecting static defaults (presence-sensitive for gateway/cron/TUI).
pub fn load_effective_config(
    home_dir: &Path,
    secrets: Option<&SecretScope>,
) -> Result<serde_yaml::Value, ConfigError> {
    let user_path = home_dir.join("config.yaml");
    let mut base = read_yaml_value(&user_path)?
        .unwrap_or_else(|| serde_yaml::Value::Mapping(Default::default()));
    let managed = Path::new("/etc/saif/config.yaml");
    if let Some(overlay) = read_yaml_value(managed)? {
        merge_yaml_maps(&mut base, overlay);
    }
    expand_yaml_strings(&mut base, secrets);
    Ok(base)
}

/// Atomic config persistence (`temp -> write_all -> sync_all -> rename`) so crashes never leave truncated YAML.
pub fn atomic_yaml_write<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let body = serde_yaml::to_string(value).map_err(|source| ConfigError::Yaml {
        path: path.display().to_string(),
        source,
    })?;
    let tmp = path.with_extension(format!("yaml.tmp.{}", std::process::id()));
    let mut file = fs::File::create(&tmp).map_err(|source| ConfigError::Io {
        path: tmp.display().to_string(),
        source,
    })?;
    file.write_all(body.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|source| ConfigError::Io {
            path: tmp.display().to_string(),
            source,
        })?;
    fs::rename(&tmp, path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}
