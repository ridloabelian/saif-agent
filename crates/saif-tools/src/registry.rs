use async_trait::async_trait;
use regex::Regex;
use saif_config::{ProfileScope, SaifConfig};
use saif_persistence::{KanbanStore, SqliteStore};
use saif_providers::ToolDefinition;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use thiserror::Error;

pub const CORE_TOOLS: &[&str] = &[
    "web_search",
    "web_extract",
    "terminal",
    "process_manage",
    "read_file",
    "write_file",
    "patch",
    "search_files",
    "skills_list",
    "skill_view",
    "skill_manage",
    "todo_list",
    "memory",
    "session_search",
    "cronjob_manage",
    "kanban_show",
    "kanban_list",
    "kanban_create",
    "kanban_complete",
    "kanban_block",
    "delegate_task",
    "tool_search",
];

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool '{0}'")]
    UnknownTool(String),
    #[error("tool '{0}' unavailable for current profile/session")]
    Unavailable(String),
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("execution error: {0}")]
    Execution(String),
}

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub cwd: PathBuf,
    pub profile: Arc<ProfileScope>,
    pub config: Arc<SaifConfig>,
    pub store: Option<SqliteStore>,
    pub kanban: Option<KanbanStore>,
    pub task_id: Option<String>,
    pub spawn_depth: usize,
    pub is_delegated_child: bool,
}

#[async_trait]
pub trait ToolHandler: Send + Sync {
    fn name(&self) -> &str;
    fn toolset(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    fn is_available(&self, _ctx: &ToolContext) -> bool {
        true
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError>;
}

#[derive(Clone, Default)]
pub struct ToolsetCatalog {
    groups: BTreeMap<String, Vec<String>>,
}

impl ToolsetCatalog {
    pub fn insert(&mut self, name: impl Into<String>, tools: Vec<String>) {
        self.groups.insert(name.into(), tools);
    }

    pub fn resolve_enabled(
        &self,
        enabled_toolsets: &[String],
        disabled_toolsets: &[String],
        is_delegated_child: bool,
        orchestrator_enabled: bool,
    ) -> HashSet<String> {
        let mut active: HashSet<String> = HashSet::new();
        if enabled_toolsets.is_empty()
            || enabled_toolsets
                .iter()
                .any(|t| t == "hermes-cli" || t == "all")
        {
            for t in CORE_TOOLS {
                active.insert((*t).to_string());
            }
            for tools in self.groups.values() {
                for t in tools {
                    active.insert(t.clone());
                }
            }
        } else {
            for ts in enabled_toolsets {
                if let Some(tools) = self.groups.get(ts) {
                    for t in tools {
                        active.insert(t.clone());
                    }
                }
            }
        }
        for ts in disabled_toolsets {
            if let Some(tools) = self.groups.get(ts) {
                for t in tools {
                    active.remove(t);
                }
            }
        }
        if is_delegated_child && !orchestrator_enabled {
            active.remove("delegate_task");
            active.remove("memory");
            active.remove("cronjob_manage");
        }
        active
    }

    pub fn names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }
}

pub fn register_default_toolsets() -> ToolsetCatalog {
    let mut cat = ToolsetCatalog::default();
    cat.insert(
        "file",
        vec![
            "read_file".into(),
            "write_file".into(),
            "patch".into(),
            "search_files".into(),
        ],
    );
    cat.insert("terminal", vec!["terminal".into(), "process_manage".into()]);
    cat.insert("web", vec!["web_search".into(), "web_extract".into()]);
    cat.insert(
        "skills",
        vec![
            "skills_list".into(),
            "skill_view".into(),
            "skill_manage".into(),
        ],
    );
    cat.insert("todo", vec!["todo_list".into()]);
    cat.insert("memory", vec!["memory".into()]);
    cat.insert("session_search", vec!["session_search".into()]);
    cat.insert("cronjob", vec!["cronjob_manage".into()]);
    cat.insert("delegation", vec!["delegate_task".into()]);
    cat.insert(
        "kanban",
        vec![
            "kanban_show".into(),
            "kanban_list".into(),
            "kanban_create".into(),
            "kanban_complete".into(),
            "kanban_block".into(),
        ],
    );
    cat.insert(
        "browser",
        vec!["browser_exec".into(), "browser_vault".into()],
    );
    cat.insert("code_execution", vec!["execute_code".into()]);
    cat.insert("computer_use", vec!["computer_use".into()]);
    cat.insert(
        "tts",
        vec!["text_to_speech".into(), "speech_to_text".into()],
    );
    cat.insert(
        "media",
        vec!["image_generate".into(), "video_generate".into()],
    );
    cat.insert("mcp", Vec::new());
    cat
}

type AvailabilityCache = Arc<RwLock<HashMap<(String, bool, usize, String), (bool, Instant)>>>;

#[derive(Clone)]
pub struct ToolRegistry {
    handlers: Arc<RwLock<BTreeMap<String, Arc<dyn ToolHandler>>>>,
    toolsets: Arc<RwLock<ToolsetCatalog>>,
    availability_cache: AvailabilityCache,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(BTreeMap::new())),
            toolsets: Arc::new(RwLock::new(register_default_toolsets())),
            availability_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&self, handler: Arc<dyn ToolHandler>) {
        let name = handler.name().to_string();
        if let Ok(mut guard) = self.handlers.write() {
            guard.insert(name, handler);
        }
    }

    pub fn get_definitions(
        &self,
        ctx: &ToolContext,
        disabled_toolsets: &[String],
    ) -> Vec<ToolDefinition> {
        let allowed = {
            let ts = self.toolsets.read().ok();
            ts.map(|c| {
                c.resolve_enabled(
                    &ctx.config.toolsets,
                    disabled_toolsets,
                    ctx.is_delegated_child,
                    ctx.config.delegation.orchestrator_enabled,
                )
            })
            .unwrap_or_default()
        };

        let mcp_enabled = (ctx.config.toolsets.is_empty()
            || ctx
                .config
                .toolsets
                .iter()
                .any(|t| t == "hermes-cli" || t == "all" || t == "mcp"))
            && !disabled_toolsets.iter().any(|t| t == "mcp");

        let handlers = self.handlers.read().map(|g| g.clone()).unwrap_or_default();
        let mut defs = Vec::new();
        for (name, handler) in handlers {
            let is_mcp_tool = name.starts_with("mcp__") && mcp_enabled;
            if !allowed.contains(&name) && !is_mcp_tool {
                continue;
            }
            if !self.check_available_cached(&name, handler.as_ref(), ctx) {
                continue;
            }
            defs.push(handler.definition());
        }
        defs
    }

    fn check_available_cached(
        &self,
        name: &str,
        handler: &dyn ToolHandler,
        ctx: &ToolContext,
    ) -> bool {
        let home_key = ctx.profile.home_dir.to_string_lossy().into_owned();
        let cache_key = (
            home_key,
            ctx.is_delegated_child,
            ctx.spawn_depth,
            name.to_string(),
        );
        if let Ok(guard) = self.availability_cache.read() {
            if let Some((ok, expires)) = guard.get(&cache_key) {
                if Instant::now() < *expires {
                    return *ok;
                }
            }
        }
        let ok = handler.is_available(ctx);
        if let Ok(mut guard) = self.availability_cache.write() {
            guard.insert(cache_key, (ok, Instant::now() + Duration::from_secs(30)));
        }
        ok
    }

    pub async fn dispatch(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<String, ToolError> {
        let handler = {
            let guard = self
                .handlers
                .read()
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            guard
                .get(name)
                .cloned()
                .ok_or_else(|| ToolError::UnknownTool(name.to_string()))?
        };
        handler.execute(args, ctx).await
    }

    /// Lightweight Snowball-style BM25-inspired keyword/stem search over registered tool schemas.
    pub fn search_tools(&self, query: &str, limit: usize) -> Vec<ToolDefinition> {
        let q_terms = stem_tokens(query);
        if q_terms.is_empty() {
            return Vec::new();
        }
        let handlers = self.handlers.read().map(|g| g.clone()).unwrap_or_default();
        let mut scored: Vec<(usize, ToolDefinition)> = handlers
            .values()
            .filter_map(|h| {
                let def = h.definition();
                let doc = format!("{} {} {}", def.name, h.toolset(), def.description);
                let doc_terms = stem_tokens(&doc);
                let hits = q_terms.iter().filter(|t| doc_terms.contains(*t)).count();
                (hits > 0).then_some((hits, def))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.into_iter().take(limit).map(|(_, d)| d).collect()
    }
}

fn stem_tokens(text: &str) -> HashSet<String> {
    let re = Regex::new(r"[A-Za-z0-9_]+").unwrap_or_else(|_| unreachable!());
    re.find_iter(text)
        .flat_map(|m| m.as_str().split('_'))
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            lower
                .strip_suffix("ing")
                .or_else(|| lower.strip_suffix("ed"))
                .or_else(|| lower.strip_suffix("es"))
                .or_else(|| lower.strip_suffix('s'))
                .filter(|stem| stem.len() >= 3)
                .unwrap_or(&lower)
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}
