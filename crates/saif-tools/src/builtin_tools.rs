use crate::browser::{handle_browser_exec, handle_browser_vault, CdpSupervisorRegistry};
use crate::cloud_env::{DaytonaEnv, ModalEnv, VercelSandboxEnv};
use crate::code_exec::{handle_computer_use, handle_execute_code};
use crate::env::{DockerEnv, ExecutionEnvironment, LocalEnv, ProcessRegistry, SshEnv};
use crate::fs_tools::{handle_patch, handle_read_file, handle_search_files, handle_write_file};
use crate::media_tools::{
    handle_image_generate, handle_speech_to_text, handle_text_to_speech, handle_video_generate,
};
use crate::registry::{ToolContext, ToolError, ToolHandler, ToolRegistry};
use crate::web_tools::{handle_web_extract, handle_web_search};
use async_trait::async_trait;
use saif_cron::CronStore;
use saif_persistence::{PersistenceStore, TaskStatus};
use saif_providers::ToolDefinition;
use saif_skills::{Curator, SkillStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
}

#[derive(Clone, Default)]
pub struct TodoStore {
    sessions: Arc<Mutex<HashMap<String, Vec<TodoItem>>>>,
}

impl TodoStore {
    pub fn get(&self, session_id: &str) -> Vec<TodoItem> {
        self.sessions
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).cloned())
            .unwrap_or_default()
    }

    pub fn set(&self, session_id: &str, items: Vec<TodoItem>) {
        if let Ok(mut g) = self.sessions.lock() {
            g.insert(session_id.to_string(), items);
        }
    }
}

pub fn register_all_builtin_tools(
    registry: &ToolRegistry,
    proc_registry: ProcessRegistry,
    todo_store: TodoStore,
) {
    register_all_builtin_tools_with_cdp(
        registry,
        proc_registry,
        todo_store,
        CdpSupervisorRegistry::new(),
    );
}

pub fn register_all_builtin_tools_with_cdp(
    registry: &ToolRegistry,
    proc_registry: ProcessRegistry,
    todo_store: TodoStore,
    cdp_supervisors: CdpSupervisorRegistry,
) {
    let tools: Vec<Arc<dyn ToolHandler>> = vec![
        Arc::new(SimpleTool::new(
            "read_file",
            "file",
            "Read a text file with optional 1-based offset and line limit.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "required": ["path"]
            }),
            |args, ctx| Box::pin(async move { handle_read_file(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "write_file",
            "file",
            "Write UTF-8 text to a file path, creating parent directories if needed.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            |args, ctx| Box::pin(async move { handle_write_file(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "patch",
            "file",
            "Apply fuzzy surgical replacement or V4A multi-file patch.",
            json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["replace", "patch"] },
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" },
                    "patch": { "type": "string" }
                }
            }),
            |args, ctx| Box::pin(async move { handle_patch(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "search_files",
            "file",
            "Search file contents (regex) or file paths respecting .gitignore.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "target": { "type": "string", "enum": ["content", "files"] },
                    "path": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["pattern"]
            }),
            |args, ctx| Box::pin(async move { handle_search_files(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "web_search",
            "web",
            "Search the web using configured search providers.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
            |args, ctx| Box::pin(async move { handle_web_search(&args, &ctx).await }),
        )),
        Arc::new(SimpleTool::new(
            "web_extract",
            "web",
            "Fetch and extract readable text from an HTTP/HTTPS URL.",
            json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
            |args, ctx| Box::pin(async move { handle_web_extract(&args, &ctx).await }),
        )),
        Arc::new(TerminalTool {
            proc_registry: proc_registry.clone(),
        }),
        Arc::new(ProcessManageTool { proc_registry }),
        Arc::new(TodoTool { todo_store }),
        Arc::new(SimpleTool::new(
            "memory",
            "memory",
            "Read, append, replace, or delete entries in persistent MEMORY.md or USER.md.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["read", "add", "replace", "remove"] },
                    "target": { "type": "string", "enum": ["memory", "user"] },
                    "content": { "type": "string" },
                    "old_text": { "type": "string" }
                },
                "required": ["action"]
            }),
            |args, ctx| Box::pin(async move { handle_memory(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "skills_list",
            "skills",
            "List installed skills and their concise descriptions.",
            json!({ "type": "object", "properties": {} }),
            |_, ctx| Box::pin(async move { handle_skills_list(&ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "skill_view",
            "skills",
            "Load the complete SKILL.md document for a skill without pagination.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
            |args, ctx| Box::pin(async move { handle_skill_view(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "skill_manage",
            "skills",
            "Create, patch, or delete agent-authored skills.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["create", "patch", "delete"] },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "content": { "type": "string" },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" },
                    "category": { "type": "string" }
                },
                "required": ["action", "name"]
            }),
            |args, ctx| Box::pin(async move { handle_skill_manage(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "session_search",
            "session_search",
            "Search past conversation history via SQLite FTS5.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }),
            |args, ctx| Box::pin(async move { handle_session_search(&args, &ctx).await }),
        )),
        Arc::new(SimpleTool::new(
            "cronjob_manage",
            "cronjob",
            "Create, list, pause, resume, or remove scheduled cron jobs.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "create", "pause", "resume", "remove"] },
                    "name": { "type": "string" },
                    "schedule": { "type": "string" },
                    "prompt": { "type": "string" },
                    "job_id": { "type": "string" }
                },
                "required": ["action"]
            }),
            |args, ctx| Box::pin(async move { handle_cronjob(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "kanban_list",
            "kanban",
            "List Kanban board tasks.",
            json!({
                "type": "object",
                "properties": { "status": { "type": "string" } }
            }),
            |args, ctx| Box::pin(async move { handle_kanban_list(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "kanban_show",
            "kanban",
            "Show a Kanban task by task_id.",
            json!({
                "type": "object",
                "properties": { "task_id": { "type": "string" } }
            }),
            |args, ctx| Box::pin(async move { handle_kanban_show(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "kanban_create",
            "kanban",
            "Create a new Kanban task.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body": { "type": "string" },
                    "assignee": { "type": "string" }
                },
                "required": ["title"]
            }),
            |args, ctx| Box::pin(async move { handle_kanban_create(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "kanban_complete",
            "kanban",
            "Mark a Kanban task completed with result summary.",
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "summary": { "type": "string" }
                },
                "required": ["summary"]
            }),
            |args, ctx| Box::pin(async move { handle_kanban_complete(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "kanban_block",
            "kanban",
            "Block a Kanban task with an explicit reason.",
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "reason": { "type": "string" }
                },
                "required": ["reason"]
            }),
            |args, ctx| Box::pin(async move { handle_kanban_block(&args, &ctx) }),
        )),
        Arc::new(SimpleTool::new(
            "computer_use",
            "computer_use",
            "Capture screen or send GUI pointer/keyboard input on active desktop display.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["screenshot", "mouse_move", "left_click", "right_click", "type_text", "key_press", "cursor_position"] },
                    "x": { "type": "integer" },
                    "y": { "type": "integer" },
                    "text": { "type": "string" },
                    "dry_run": { "type": "boolean" }
                },
                "required": ["action"]
            }),
            |args, ctx| Box::pin(async move { handle_computer_use(&args, &ctx).await }),
        )),
        Arc::new(SimpleTool::new(
            "text_to_speech",
            "tts",
            "Synthesize speech to audio file via OpenAI, ElevenLabs, xAI, or Gemini.",
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "provider": { "type": "string" },
                    "output_path": { "type": "string" },
                    "voice": { "type": "string" },
                    "speed": { "type": "number" },
                    "api_base_url": { "type": "string" }
                },
                "required": ["text"]
            }),
            |args, ctx| Box::pin(async move { handle_text_to_speech(&args, &ctx).await }),
        )),
        Arc::new(SimpleTool::new(
            "speech_to_text",
            "tts",
            "Transcribe an audio file into text.",
            json!({
                "type": "object",
                "properties": {
                    "audio_path": { "type": "string" },
                    "model": { "type": "string" },
                    "api_base_url": { "type": "string" }
                },
                "required": ["audio_path"]
            }),
            |args, ctx| Box::pin(async move { handle_speech_to_text(&args, &ctx).await }),
        )),
        Arc::new(SimpleTool::new(
            "image_generate",
            "media",
            "Generate images from a text prompt.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string" },
                    "output_path": { "type": "string" },
                    "size": { "type": "string" },
                    "model": { "type": "string" },
                    "api_base_url": { "type": "string" }
                },
                "required": ["prompt"]
            }),
            |args, ctx| Box::pin(async move { handle_image_generate(&args, &ctx).await }),
        )),
        Arc::new(SimpleTool::new(
            "video_generate",
            "media",
            "Generate video clips from a text prompt.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string" },
                    "model": { "type": "string" },
                    "api_base_url": { "type": "string" }
                },
                "required": ["prompt"]
            }),
            |args, ctx| Box::pin(async move { handle_video_generate(&args, &ctx).await }),
        )),
        Arc::new(BrowserExecTool {
            supervisors: cdp_supervisors.clone(),
        }),
        Arc::new(BrowserVaultTool {
            supervisors: cdp_supervisors,
        }),
        Arc::new(ExecuteCodeTool {
            registry: registry.clone(),
        }),
        Arc::new(DelegateTaskTool),
        Arc::new(ToolSearchTool {
            registry: registry.clone(),
        }),
    ];

    for t in tools {
        registry.register(t);
    }
}

type BoxFut =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, ToolError>> + Send>>;
type ExecFn = fn(Value, ToolContext) -> BoxFut;

struct SimpleTool {
    name: &'static str,
    toolset: &'static str,
    description: &'static str,
    parameters: Value,
    exec: ExecFn,
}

impl SimpleTool {
    fn new(
        name: &'static str,
        toolset: &'static str,
        description: &'static str,
        parameters: Value,
        exec: ExecFn,
    ) -> Self {
        Self {
            name,
            toolset,
            description,
            parameters,
            exec,
        }
    }
}

#[async_trait]
impl ToolHandler for SimpleTool {
    fn name(&self) -> &str {
        self.name
    }
    fn toolset(&self) -> &str {
        self.toolset
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: self.description.to_string(),
            parameters: self.parameters.clone(),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        (self.exec)(args, ctx.clone()).await
    }
}

struct TerminalTool {
    proc_registry: ProcessRegistry,
}

#[async_trait]
impl ToolHandler for TerminalTool {
    fn name(&self) -> &str {
        "terminal"
    }
    fn toolset(&self) -> &str {
        "terminal"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "terminal".to_string(),
            description: "Run a shell command in the active execution environment.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "workdir": { "type": "string" },
                    "timeout": { "type": "integer" },
                    "background": { "type": "boolean" }
                },
                "required": ["command"]
            }),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'command'".to_string()))?;
        let cwd = args
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(|p| {
                let path = std::path::Path::new(p);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    ctx.cwd.join(path)
                }
            })
            .unwrap_or_else(|| ctx.cwd.clone());
        let env_vars = ctx
            .profile
            .secrets
            .build_child_env(&ctx.profile.home_dir, &ctx.config.terminal.env_passthrough);

        if args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let entry = self.proc_registry.spawn_background(
                command,
                &cwd,
                &env_vars,
                Some(ctx.session_id.clone()),
            )?;
            return Ok(
                json!({ "success": true, "background": true, "process": entry }).to_string(),
            );
        }

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(ctx.config.terminal.timeout);
        let res = match ctx.config.terminal.backend.as_str() {
            "docker" => {
                DockerEnv {
                    image: ctx.config.terminal.docker_image.clone(),
                    container_name: None,
                }
                .execute(command, &cwd, &env_vars, Duration::from_secs(timeout_secs))
                .await?
            }
            "ssh" => {
                let target = ctx
                    .config
                    .extra
                    .get("ssh_host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("localhost")
                    .to_string();
                SshEnv { target, port: None }
                    .execute(command, &cwd, &env_vars, Duration::from_secs(timeout_secs))
                    .await?
            }
            "modal" => {
                let url = ctx
                    .config
                    .extra
                    .get("modal_api_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://api.modal.com");
                ModalEnv::new(
                    url,
                    ctx.profile.secrets.get_secret("MODAL_TOKEN_ID"),
                    ctx.profile.secrets.get_secret("MODAL_TOKEN_SECRET"),
                    &ctx.config.terminal.docker_image,
                    ctx.task_id.as_deref().unwrap_or(&ctx.session_id),
                    ctx.profile.home_dir.clone(),
                )
                .execute(command, &cwd, &env_vars, Duration::from_secs(timeout_secs))
                .await?
            }
            "daytona" => {
                let url = ctx
                    .config
                    .extra
                    .get("daytona_api_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://app.daytona.io/api");
                DaytonaEnv::new(
                    url,
                    ctx.profile.secrets.get_secret("DAYTONA_API_KEY"),
                    &ctx.config.terminal.docker_image,
                    ctx.task_id.as_deref().unwrap_or(&ctx.session_id),
                    10_240,
                )
                .execute(command, &cwd, &env_vars, Duration::from_secs(timeout_secs))
                .await?
            }
            "vercel_sandbox" => {
                let url = ctx
                    .config
                    .extra
                    .get("vercel_sandbox_api_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://api.vercel.com");
                VercelSandboxEnv::new(
                    url,
                    ctx.profile.secrets.get_secret("VERCEL_TOKEN"),
                    ctx.task_id.as_deref().unwrap_or(&ctx.session_id),
                    ctx.profile.home_dir.clone(),
                )
                .execute(command, &cwd, &env_vars, Duration::from_secs(timeout_secs))
                .await?
            }
            _ => {
                LocalEnv
                    .execute(command, &cwd, &env_vars, Duration::from_secs(timeout_secs))
                    .await?
            }
        };
        Ok(json!({
            "success": res.exit_code == 0,
            "exit_code": res.exit_code,
            "stdout": res.stdout,
            "stderr": res.stderr,
            "timed_out": res.timed_out
        })
        .to_string())
    }
}

struct ProcessManageTool {
    proc_registry: ProcessRegistry,
}

#[async_trait]
impl ToolHandler for ProcessManageTool {
    fn name(&self) -> &str {
        "process_manage"
    }
    fn toolset(&self) -> &str {
        "terminal"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "process_manage".to_string(),
            description: "List, kill, or hand off background processes.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "kill", "handoff"] },
                    "process_id": { "type": "string" }
                },
                "required": ["action"]
            }),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;
        match action {
            "list" => {
                let list = self.proc_registry.list();
                Ok(json!({ "success": true, "processes": list }).to_string())
            }
            "kill" => {
                let pid = args
                    .get("process_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidArgs("missing 'process_id'".to_string()))?;
                let grace =
                    Duration::from_secs_f64(ctx.config.terminal.daemon_term_grace_seconds.max(0.1));
                let killed = self.proc_registry.kill(pid, grace)?;
                Ok(json!({ "success": killed, "process_id": pid }).to_string())
            }
            "handoff" => {
                let pid = args
                    .get("process_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidArgs("missing 'process_id'".to_string()))?;
                let ok = self
                    .proc_registry
                    .transfer_ownership(pid, Some(ctx.session_id.clone()));
                Ok(json!({ "success": ok, "process_id": pid }).to_string())
            }
            other => Err(ToolError::InvalidArgs(format!("unknown action '{other}'"))),
        }
    }
}

struct TodoTool {
    todo_store: TodoStore,
}

#[async_trait]
impl ToolHandler for TodoTool {
    fn name(&self) -> &str {
        "todo_list"
    }
    fn toolset(&self) -> &str {
        "todo"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "todo_list".to_string(),
            description: "Read or update the structured task list for this session.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["read", "write"] },
                    "todos": { "type": "array" }
                }
            }),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        if let Some(todos_val) = args.get("todos") {
            let items: Vec<TodoItem> = serde_json::from_value(todos_val.clone())
                .map_err(|e| ToolError::InvalidArgs(format!("invalid todos: {e}")))?;
            self.todo_store.set(&ctx.session_id, items.clone());
            return Ok(json!({ "success": true, "todos": items }).to_string());
        }
        let items = self.todo_store.get(&ctx.session_id);
        Ok(json!({ "success": true, "todos": items }).to_string())
    }
}

struct BrowserExecTool {
    supervisors: CdpSupervisorRegistry,
}

#[async_trait]
impl ToolHandler for BrowserExecTool {
    fn name(&self) -> &str {
        "browser_exec"
    }
    fn toolset(&self) -> &str {
        "browser"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "browser_exec".to_string(),
            description: "Control a supervised Chromium session over WebSocket CDP.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["attach", "navigate", "eval", "click", "screenshot"] },
                    "cdp_url": { "type": "string" },
                    "url": { "type": "string" },
                    "expression": { "type": "string" },
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                },
                "required": ["action"]
            }),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        handle_browser_exec(&args, ctx, &self.supervisors).await
    }
}

struct BrowserVaultTool {
    supervisors: CdpSupervisorRegistry,
}

#[async_trait]
impl ToolHandler for BrowserVaultTool {
    fn name(&self) -> &str {
        "browser_vault"
    }
    fn toolset(&self) -> &str {
        "browser"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "browser_vault".to_string(),
            description: "Model-blind origin-bound browser credential autofill via CDP WebSocket."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "save_login", "fill"] },
                    "handle": { "type": "string" },
                    "origin": { "type": "string" },
                    "username": { "type": "string" },
                    "password": { "type": "string" }
                },
                "required": ["action"]
            }),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        handle_browser_vault(&args, ctx, &self.supervisors).await
    }
}

struct ExecuteCodeTool {
    registry: ToolRegistry,
}

struct DelegateTaskTool;

#[async_trait]
impl ToolHandler for DelegateTaskTool {
    fn name(&self) -> &str {
        "delegate_task"
    }

    fn toolset(&self) -> &str {
        "delegation"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delegate_task".to_string(),
            description: "Delegate a scoped sub-task to an isolated child agent session."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "goal": { "type": "string" },
                    "context": { "type": "string" },
                    "toolsets": { "type": "array", "items": { "type": "string" } },
                    "background": { "type": "boolean" }
                },
                "required": ["goal"]
            }),
        }
    }

    fn is_available(&self, ctx: &ToolContext) -> bool {
        ctx.spawn_depth < ctx.config.delegation.max_spawn_depth
            && (!ctx.is_delegated_child || ctx.config.delegation.orchestrator_enabled)
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<String, ToolError> {
        Err(ToolError::Execution(
            "delegate_task must be executed via AgentRunner".to_string(),
        ))
    }
}

#[async_trait]
impl ToolHandler for ExecuteCodeTool {
    fn name(&self) -> &str {
        "execute_code"
    }
    fn toolset(&self) -> &str {
        "code_execution"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "execute_code".to_string(),
            description: "Run Python script calling allowlisted saif_tools via loopback RPC."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string" },
                    "max_tool_calls": { "type": "integer" }
                },
                "required": ["code"]
            }),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        handle_execute_code(&args, ctx, &self.registry).await
    }
}

struct ToolSearchTool {
    registry: ToolRegistry,
}

#[async_trait]
impl ToolHandler for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }
    fn toolset(&self) -> &str {
        "core"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "tool_search".to_string(),
            description: "Search available tools by keyword.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }),
        }
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, ToolError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'query'".to_string()))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let hits = self.registry.search_tools(query, limit);
        Ok(json!({ "success": true, "tools": hits }).to_string())
    }
}

fn handle_memory(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;
    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("memory");
    let filename = if target == "user" {
        "USER.md"
    } else {
        "MEMORY.md"
    };
    let path = ctx.profile.memories_dir().join(filename);
    let current = fs::read_to_string(&path).unwrap_or_default();

    match action {
        "read" => Ok(json!({ "success": true, "target": target, "content": current }).to_string()),
        "add" => {
            let entry = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'content'".to_string()))?;
            if let Some(msg) =
                saif_skills::first_threat_message(entry, saif_skills::ThreatScope::Strict)
            {
                return Ok(json!({ "success": false, "error": msg }).to_string());
            }
            fs::create_dir_all(ctx.profile.memories_dir())
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            let next = if current.trim().is_empty() {
                format!("- {entry}\n")
            } else {
                format!("{}\n- {entry}\n", current.trim_end())
            };
            fs::write(&path, &next).map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "target": target, "chars": next.len() }).to_string())
        }
        "replace" => {
            let old_text = args
                .get("old_text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'old_text'".to_string()))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'content'".to_string()))?;
            if !current.contains(old_text) {
                return Ok(json!({ "success": false, "error": "old_text not found" }).to_string());
            }
            let next = current.replacen(old_text, content, 1);
            fs::write(&path, &next).map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "target": target }).to_string())
        }
        "remove" => {
            let old_text = args
                .get("old_text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'old_text'".to_string()))?;
            let next = current.replace(old_text, "");
            fs::write(&path, &next).map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "target": target }).to_string())
        }
        other => Err(ToolError::InvalidArgs(format!("unknown action '{other}'"))),
    }
}

fn handle_skills_list(ctx: &ToolContext) -> Result<String, ToolError> {
    let store = SkillStore::new(ctx.profile.skills_dir(), &ctx.config.skills.disabled);
    let items: Vec<Value> = store
        .list_skills()
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "category": s.category,
                "description": s.prompt_description
            })
        })
        .collect();
    Ok(json!({ "success": true, "skills": items }).to_string())
}

fn handle_skill_view(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'name'".to_string()))?;
    let store = SkillStore::new(ctx.profile.skills_dir(), &ctx.config.skills.disabled);
    let doc = store
        .view_skill(name)
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let curator = Curator::new(ctx.profile.skills_dir());
    let _ = curator.record_view(name);
    Ok(json!({
        "success": true,
        "name": doc.name,
        "path": doc.path.display().to_string(),
        "content": doc.raw
    })
    .to_string())
}

fn handle_skill_manage(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'name'".to_string()))?;
    let store = SkillStore::new(ctx.profile.skills_dir(), &ctx.config.skills.disabled);
    let curator = Curator::new(ctx.profile.skills_dir());
    match action {
        "create" => {
            let desc = args
                .get("description")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'description'".to_string()))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'content'".to_string()))?;
            let cat = args.get("category").and_then(|v| v.as_str());
            let path = store
                .create_skill(name, desc, content, cat)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "path": path.display().to_string() }).to_string())
        }
        "patch" => {
            let old_text = args
                .get("old_text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'old_text'".to_string()))?;
            let new_text = args
                .get("new_text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'new_text'".to_string()))?;
            let path = store
                .patch_skill(name, old_text, new_text)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            let _ = curator.record_patch(name);
            Ok(json!({ "success": true, "path": path.display().to_string() }).to_string())
        }
        "delete" => {
            let pinned = curator.is_pinned(name);
            store
                .delete_skill(name, pinned)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "deleted": name }).to_string())
        }
        other => Err(ToolError::InvalidArgs(format!("unknown action '{other}'"))),
    }
}

async fn handle_session_search(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'query'".to_string()))?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let Some(store) = &ctx.store else {
        return Ok(json!({ "success": false, "error": "session store not attached" }).to_string());
    };
    let hits = store
        .search_messages(query, limit)
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({ "success": true, "hits": hits }).to_string())
}

fn handle_cronjob(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;
    let store = CronStore::new(ctx.profile.cron_dir());
    match action {
        "list" => {
            let jobs = store
                .load_jobs()
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "jobs": jobs }).to_string())
        }
        "create" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("job");
            let schedule = args
                .get("schedule")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'schedule'".to_string()))?;
            let prompt = args
                .get("prompt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'prompt'".to_string()))?;
            let job = store
                .create_job(name, schedule, prompt, None, None)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "job": job }).to_string())
        }
        "pause" | "resume" => {
            let id = args
                .get("job_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'job_id'".to_string()))?;
            let job = store
                .set_enabled(id, action == "resume")
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "job": job }).to_string())
        }
        "remove" => {
            let id = args
                .get("job_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'job_id'".to_string()))?;
            store
                .remove_job(id)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(json!({ "success": true, "removed": id }).to_string())
        }
        other => Err(ToolError::InvalidArgs(format!("unknown action '{other}'"))),
    }
}

fn handle_kanban_list(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let Some(kb) = &ctx.kanban else {
        return Ok(json!({ "success": false, "error": "kanban store unavailable" }).to_string());
    };
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(TaskStatus::parse);
    let tasks = kb
        .list_tasks(status)
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({ "success": true, "tasks": tasks }).to_string())
}

fn handle_kanban_show(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let Some(kb) = &ctx.kanban else {
        return Ok(json!({ "success": false, "error": "kanban store unavailable" }).to_string());
    };
    let tid = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .or(ctx.task_id.as_deref())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'task_id'".to_string()))?;
    let task = kb
        .show_task(tid)
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({ "success": true, "task": task }).to_string())
}

fn handle_kanban_create(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    if ctx.is_delegated_child {
        return Err(ToolError::Execution(
            "delegate_task child contexts cannot mutate Kanban boards".to_string(),
        ));
    }
    let Some(kb) = &ctx.kanban else {
        return Ok(json!({ "success": false, "error": "kanban store unavailable" }).to_string());
    };
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'title'".to_string()))?;
    let body = args.get("body").and_then(|v| v.as_str());
    let assignee = args.get("assignee").and_then(|v| v.as_str());
    let task = kb
        .create_task(title, body, assignee, 0, &[])
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({ "success": true, "task": task }).to_string())
}

fn handle_kanban_complete(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let Some(kb) = &ctx.kanban else {
        return Ok(json!({ "success": false, "error": "kanban store unavailable" }).to_string());
    };
    let tid = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .or(ctx.task_id.as_deref())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'task_id'".to_string()))?;
    let summary = args
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("completed");
    kb.complete_task(tid, summary)
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({ "success": true, "task_id": tid }).to_string())
}

fn handle_kanban_block(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let Some(kb) = &ctx.kanban else {
        return Ok(json!({ "success": false, "error": "kanban store unavailable" }).to_string());
    };
    let tid = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .or(ctx.task_id.as_deref())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'task_id'".to_string()))?;
    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("blocked");
    kb.record_failure(tid, reason, None, 1)
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({ "success": true, "task_id": tid, "reason": reason }).to_string())
}
