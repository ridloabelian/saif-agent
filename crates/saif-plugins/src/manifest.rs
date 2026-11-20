use crate::hooks::{HookRegistry, PluginHookBridge};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin error: {0}")]
    Invalid(String),
    #[error("plugin '{0}' is on the removed blocklist")]
    RemovedFromCatalog(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    #[default]
    General,
    MemoryProvider,
    ModelProvider,
    ContextEngine,
    ImageGen,
    VideoGen,
    CronProvider,
    Platform,
    LocalePack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: PluginKind,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub provides_locales: bool,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub manifest: PluginManifest,
    pub root_dir: PathBuf,
    pub enabled: bool,
}

#[derive(Clone, Default)]
pub struct PluginManager {
    plugins_dir: PathBuf,
    disabled: HashSet<String>,
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf, disabled: &[String]) -> Self {
        Self {
            plugins_dir,
            disabled: disabled.iter().cloned().collect(),
        }
    }

    pub fn discover(&self) -> Vec<DiscoveredPlugin> {
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir(&self.plugins_dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let manifest_path = dir.join("plugin.yaml");
            let Ok(raw) = fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = serde_yaml::from_str::<PluginManifest>(&raw) else {
                continue;
            };
            let enabled = !self.disabled.contains(&manifest.name);
            found.push(DiscoveredPlugin {
                manifest,
                root_dir: dir,
                enabled,
            });
        }
        found.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        found
    }

    pub fn validate_manifest_file(path: &Path) -> Result<PluginManifest, PluginError> {
        let raw = fs::read_to_string(path)?;
        let manifest: PluginManifest = serde_yaml::from_str(&raw)?;
        if manifest.name.trim().is_empty() {
            return Err(PluginError::Invalid(
                "plugin name must not be empty".to_string(),
            ));
        }
        Ok(manifest)
    }

    pub fn build_hook_registry(
        &self,
        isolation: &str,
        logs_dir: &Path,
        child_env: BTreeMap<String, String>,
    ) -> HookRegistry {
        let registry = HookRegistry::new();
        let plugins = self.discover();
        if plugins.iter().any(|p| p.enabled) {
            let isolation_host = isolation.trim().eq_ignore_ascii_case("host");
            registry.register(Arc::new(PluginHookBridge::new(
                plugins,
                isolation_host,
                child_env,
                logs_dir.join("plugin_hooks.jsonl"),
            )));
        }
        registry
    }
}
