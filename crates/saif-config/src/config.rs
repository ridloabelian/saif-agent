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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpServerEntry {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomProviderEntry {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub key_env: Option<String>,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub api: Option<String>,
    pub api_mode: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SaifConfig {
    pub _config_version: u32,
    pub model: String,
    pub provider: String,
    pub base_url: Option<String>,
    pub api_mode: Option<String>,
    pub providers: BTreeMap<String, CustomProviderEntry>,
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
    pub mcp_servers: BTreeMap<String, McpServerEntry>,
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
            providers: BTreeMap::new(),
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
            mcp_servers: BTreeMap::new(),
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
    pub allowed_users: Vec<String>,
    pub deliver_outbound_webhooks: bool,
    pub session_hygiene_ratio: f64,
}

impl Default for GatewaySection {
    fn default() -> Self {
        Self {
            multiplex_profiles: None,
            allow_all_users: false,
            allowed_users: Vec::new(),
            deliver_outbound_webhooks: false,
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

/// Normalize polymorphic upstream `~/.hermes/config.yaml` shapes into `SaifConfig`'s canonical YAML layout.
pub fn normalize_hermes_yaml_compat(val: &mut serde_yaml::Value) {
    let Some(root) = val.as_mapping_mut() else {
        return;
    };

    let model_key = serde_yaml::Value::String("model".to_string());
    let prov_key = serde_yaml::Value::String("provider".to_string());
    let base_key = serde_yaml::Value::String("base_url".to_string());

    if let Some(serde_yaml::Value::Mapping(m)) = root.get(&model_key).cloned() {
        let extracted_model = m
            .get(serde_yaml::Value::String("default".to_string()))
            .or_else(|| m.get(serde_yaml::Value::String("name".to_string())))
            .or_else(|| m.get(serde_yaml::Value::String("id".to_string())))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let extracted_prov = m
            .get(serde_yaml::Value::String("provider".to_string()))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let extracted_base = m
            .get(serde_yaml::Value::String("base_url".to_string()))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        root.insert(model_key, serde_yaml::Value::String(extracted_model));
        let top_prov_empty = root
            .get(&prov_key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().is_empty())
            .unwrap_or(true);
        if top_prov_empty {
            if let Some(p) = extracted_prov {
                root.insert(prov_key, serde_yaml::Value::String(p));
            }
        }
        let top_base_empty = root
            .get(&base_key)
            .map(|v| v.is_null() || v.as_str().map(|s| s.trim().is_empty()).unwrap_or(false))
            .unwrap_or(true);
        if top_base_empty {
            if let Some(b) = extracted_base {
                root.insert(base_key, serde_yaml::Value::String(b));
            }
        }
    }

    for list_key in ["fallback_providers", "toolsets"] {
        let k = serde_yaml::Value::String(list_key.to_string());
        if let Some(serde_yaml::Value::String(raw)) = root.get(&k).cloned() {
            let seq = raw
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| serde_yaml::Value::String(s.to_string()))
                .collect();
            root.insert(k, serde_yaml::Value::Sequence(seq));
        }
    }

    let mcp_key = serde_yaml::Value::String("mcp_servers".to_string());
    if let Some(serde_yaml::Value::Mapping(servers)) = root.get_mut(&mcp_key) {
        let args_key = serde_yaml::Value::String("args".to_string());
        for (_, srv_val) in servers.iter_mut() {
            if let Some(srv_map) = srv_val.as_mapping_mut() {
                if let Some(serde_yaml::Value::String(raw)) = srv_map.get(&args_key).cloned() {
                    let seq = raw
                        .split_whitespace()
                        .map(|s| serde_yaml::Value::String(s.to_string()))
                        .collect();
                    srv_map.insert(args_key.clone(), serde_yaml::Value::Sequence(seq));
                }
            }
        }
    }

    let gw_key = serde_yaml::Value::String("gateway".to_string());
    if let Some(serde_yaml::Value::Mapping(gw_map)) = root.get_mut(&gw_key) {
        let allowed_key = serde_yaml::Value::String("allowed_users".to_string());
        let tg_key = serde_yaml::Value::String("telegram".to_string());
        let current_empty = gw_map
            .get(&allowed_key)
            .and_then(|v| v.as_sequence())
            .is_some_and(Vec::is_empty);
        if !gw_map.contains_key(&allowed_key) || current_empty {
            if let Some(tg_allowed) = gw_map
                .get(&tg_key)
                .and_then(|v| v.as_mapping())
                .and_then(|tm| tm.get(&allowed_key))
                .cloned()
            {
                gw_map.insert(allowed_key.clone(), tg_allowed);
            }
        }
        if let Some(raw_allowed) = gw_map.get(&allowed_key).cloned() {
            let normalized: Vec<serde_yaml::Value> = match raw_allowed {
                serde_yaml::Value::Sequence(seq) => seq
                    .into_iter()
                    .filter_map(yaml_scalar_to_string)
                    .map(serde_yaml::Value::String)
                    .collect(),
                serde_yaml::Value::String(s) => s
                    .split(',')
                    .map(|p| p.trim())
                    .filter(|p| !p.is_empty())
                    .map(|p| serde_yaml::Value::String(p.to_string()))
                    .collect(),
                serde_yaml::Value::Number(n) => {
                    vec![serde_yaml::Value::String(n.to_string())]
                }
                _ => Vec::new(),
            };
            gw_map.insert(allowed_key, serde_yaml::Value::Sequence(normalized));
        }
    }

    let mem_key = serde_yaml::Value::String("memory".to_string());
    if let Some(serde_yaml::Value::Mapping(mem_map)) = root.get_mut(&mem_key) {
        let max_k = serde_yaml::Value::String("max_chars".to_string());
        let lim_k = serde_yaml::Value::String("memory_char_limit".to_string());
        if let Some(lim) = mem_map.get(&lim_k).cloned() {
            mem_map.entry(max_k).or_insert(lim);
        }
    }
}

fn yaml_scalar_to_string(v: serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => (!s.trim().is_empty()).then(|| s.trim().to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
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
    normalize_hermes_yaml_compat(&mut merged);
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

/// Typed effective config loader merging `SaifConfig::default()` with `load_effective_config`
/// (`${VAR}` expanded using profile-scoped `SecretScope` + `/etc/saif/config.yaml`).
pub fn load_effective_saif_config(
    home_dir: &Path,
    secrets: Option<&SecretScope>,
) -> Result<SaifConfig, ConfigError> {
    let mut merged =
        serde_yaml::to_value(SaifConfig::default()).map_err(|source| ConfigError::Yaml {
            path: "<defaults>".to_string(),
            source,
        })?;
    let effective = load_effective_config(home_dir, secrets)?;
    merge_yaml_maps(&mut merged, effective);
    normalize_hermes_yaml_compat(&mut merged);
    serde_yaml::from_value(merged).map_err(|source| ConfigError::Yaml {
        path: home_dir.join("config.yaml").display().to_string(),
        source,
    })
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
