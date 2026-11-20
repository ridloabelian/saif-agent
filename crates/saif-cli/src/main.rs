use clap::{Parser, Subcommand};
use saif_config::{
    atomic_yaml_write, clear_named_profile_deleted, default_saif_root, list_live_profiles,
    load_cli_config, load_config, load_effective_saif_config, mark_named_profile_deleted,
    migrate_from_hermes, parse_git_name_status, validate_profile_id, ProfileScope, SaifConfig,
    SecretScope, UpstreamLock,
};
use saif_core::{run_batch_dataset, AgentRunner, TurnOptions};
use saif_cron::{CronScheduler, CronStore};
use saif_gateway::{
    build_gateway_router, builtin_platform_specs, converge_multiplex_migration,
    render_service_unit, GatewayHttpState, GatewayMultiplexer, ServiceKind, TelegramAdapter,
};
use saif_persistence::{KanbanStore, PersistenceStore, SqliteStore, TaskStatus};
use saif_platform::{ExecutableResolver, HostFacts};
use saif_plugins::PluginManager;
use saif_providers::{builtin_providers, ChatMessage, CredentialPool, PoolStrategy};
use saif_skills::{Curator, SkillStore};
use saif_tools::{
    register_all_builtin_tools, register_trusted_mcp_tools, McpServerConfig, McpTrustStore,
    ProcessRegistry, TodoStore, ToolRegistry,
};
use saif_tui::{run_tui, run_tui_gateway};
use saif_web::{run_web_server, WebState};
use std::fs;
use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "saif",
    version,
    about = "Saif Agent — 100% Pure-Rust autonomous self-improving AI agent"
)]
struct Cli {
    /// Profile name under ~/.saif/profiles/<name>
    #[arg(short, long, global = true)]
    profile: Option<String>,

    /// Single-shot query prompt (prints response and exits)
    #[arg(short = 'q', long = "query")]
    query: Option<String>,

    /// Launch the native ratatui TUI
    #[arg(long)]
    tui: bool,

    /// Model override
    #[arg(short, long, global = true)]
    model: Option<String>,

    /// Provider override
    #[arg(long, global = true)]
    provider: Option<String>,

    /// Base URL override
    #[arg(long, global = true)]
    base_url: Option<String>,

    /// Existing session id to continue
    #[arg(long, global = true)]
    session: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive chat REPL or TUI
    Chat,
    /// NDJSON stdio JSON-RPC backend (tui_gateway compatible)
    TuiGateway,
    /// Show profile, config, provider, and database status
    Status,
    /// Inspect runtime readiness, binary paths, and security state
    Doctor,
    /// Read or update configuration & secrets
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Manage isolated profiles
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Search and inspect SQLite session history
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Manage and view skills
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
    /// Skill lifecycle curator & backups
    Curator {
        #[command(subcommand)]
        action: CuratorAction,
    },
    /// Scheduled cron jobs
    Cron {
        #[command(subcommand)]
        action: CronAction,
    },
    /// Cross-profile SQLite Kanban board
    Kanban {
        #[command(subcommand)]
        action: KanbanAction,
    },
    /// Run messaging + OpenAI-compatible HTTP gateway
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// Headless backend server for desktop app (SPA disabled)
    Serve {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: SocketAddr,
    },
    /// Web dashboard + PTY WebSocket bridge
    Dashboard {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: SocketAddr,
    },
    /// Inspect and validate plugins
    Plugins {
        #[command(subcommand)]
        action: PluginsAction,
    },
    /// Manage MCP servers and trust
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Non-destructive migration from ~/.hermes into ~/.saif
    Migrate {
        #[arg(long)]
        from_hermes: bool,
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Batch-process a JSONL prompt dataset in parallel with optional ShareGPT export & --resume
    Batch {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 4)]
        workers: usize,
        #[arg(long)]
        resume: bool,
        #[arg(long, default_value = "sharegpt")]
        format: String,
    },
    /// Manage multi-credential auth failover pool
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Track and classify upstream NousResearch/hermes-agent updates
    Upstream {
        #[command(subcommand)]
        action: UpstreamAction,
    },
}

#[derive(Subcommand)]
enum UpstreamAction {
    /// Compare conformance/UPSTREAM_LOCK.json against upstream NousResearch/hermes-agent
    Status {
        #[arg(long)]
        lock: Option<PathBuf>,
        #[arg(long)]
        local_repo: Option<PathBuf>,
    },
    /// Classify upstream changed files into PORT_REQUIRED vs SKIP_RUNTIME_SPECIFIC vs IGNORED_NOISE
    Diff {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        subsystem: Option<String>,
        #[arg(long)]
        port_only: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        lock: Option<PathBuf>,
        #[arg(long)]
        local_repo: Option<PathBuf>,
    },
    /// Record a verified upstream commit SHA into conformance/UPSTREAM_LOCK.json
    Ack {
        sha: String,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        lock: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    List {
        #[arg(long)]
        provider: Option<String>,
    },
    Add {
        provider: String,
        id: String,
        secret: String,
    },
    Strategy {
        strategy: String,
    },
    Status,
}

#[derive(Subcommand)]
enum ConfigAction {
    Show,
    Set { key: String, value: String },
    SetSecret { key: String, value: String },
}

#[derive(Subcommand)]
enum ProfileAction {
    List,
    Create { name: String },
    Delete { name: String },
}

#[derive(Subcommand)]
enum SessionsAction {
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum SkillsAction {
    List,
    View {
        name: String,
    },
    Create {
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        body: String,
        #[arg(long)]
        category: Option<String>,
    },
    Delete {
        name: String,
    },
}

#[derive(Subcommand)]
enum CuratorAction {
    Status,
    Run {
        #[arg(long)]
        no_backup: bool,
    },
    Pin {
        name: String,
    },
    Unpin {
        name: String,
    },
}

#[derive(Subcommand)]
enum CronAction {
    List,
    Add {
        name: String,
        schedule: String,
        prompt: String,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Remove {
        id: String,
    },
    Tick,
}

#[derive(Subcommand)]
enum KanbanAction {
    Init,
    List {
        #[arg(long)]
        status: Option<String>,
    },
    Create {
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
    },
    Show {
        id: String,
    },
    Complete {
        id: String,
        summary: String,
    },
    Block {
        id: String,
        reason: String,
    },
    Unblock {
        id: String,
    },
    Comment {
        id: String,
        body: String,
    },
}

#[derive(Subcommand)]
enum GatewayAction {
    Run {
        #[arg(long, default_value = "127.0.0.1:8788")]
        listen: SocketAddr,
    },
    Platforms,
    Migrate {
        #[arg(long)]
        multiplex: bool,
    },
    InstallService {
        #[arg(long, default_value = "systemd")]
        kind: String,
        #[arg(long, default_value = "127.0.0.1:8788")]
        listen: SocketAddr,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum PluginsAction {
    List,
    Validate { manifest_path: PathBuf },
}

#[derive(Subcommand)]
enum McpAction {
    List,
    Trust { server_name: String },
    Check { server_name: String },
}

struct RuntimeBundle {
    profile: Arc<ProfileScope>,
    cfg: Arc<SaifConfig>,
    store: SqliteStore,
    agent: AgentRunner,
    cwd: PathBuf,
}

async fn build_runtime(
    profile_name: Option<&str>,
    effective: bool,
) -> anyhow::Result<RuntimeBundle> {
    let root = default_saif_root();
    let multiplex = ProfileScope::detect_multiplex_active(&root);
    let profile = Arc::new(ProfileScope::for_profile(root, profile_name, multiplex)?);
    profile.ensure_layout()?;
    let cfg = Arc::new(if effective {
        load_effective_saif_config(&profile.home_dir, Some(&profile.secrets))?
    } else {
        load_cli_config(&profile.home_dir)?
    });
    let store =
        SqliteStore::open_with_journal_mode(&profile.state_db_path(), &cfg.database.journal_mode)?;
    let registry = ToolRegistry::new();
    register_all_builtin_tools(&registry, ProcessRegistry::new(), TodoStore::default());

    let child_env = profile
        .secrets
        .build_child_env(&profile.home_dir, &cfg.terminal.env_passthrough);
    if !cfg.mcp_servers.is_empty() {
        let trust = McpTrustStore::new(&profile.home_dir);
        let servers: Vec<McpServerConfig> = cfg
            .mcp_servers
            .iter()
            .map(|(name, entry)| McpServerConfig {
                name: name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                env: entry.env.clone(),
            })
            .collect();
        let _ = register_trusted_mcp_tools(&registry, &servers, &trust, &child_env).await;
    }

    let hooks = PluginManager::new(profile.plugins_dir(), &cfg.plugins.disabled)
        .build_hook_registry(&cfg.plugins.isolation, &profile.logs_dir(), child_env);

    let agent = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&cfg),
        registry,
        Some(store.clone()),
        hooks,
    );
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Ok(RuntimeBundle {
        profile,
        cfg,
        store,
        agent,
        cwd,
    })
}

async fn build_cli_runtime(profile_name: Option<&str>) -> anyhow::Result<RuntimeBundle> {
    build_runtime(profile_name, false).await
}

async fn build_effective_runtime(profile_name: Option<&str>) -> anyhow::Result<RuntimeBundle> {
    build_runtime(profile_name, true).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .try_init();

    let cli = Cli::parse();

    if let Some(prompt) = cli.query.as_deref() {
        let rt = build_cli_runtime(cli.profile.as_deref()).await?;
        return ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
            let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
            let printer = tokio::spawn(async move {
                let mut streamed = false;
                while let Some(ev) = ev_rx.recv().await {
                    match ev {
                        saif_core::AgentEvent::ContentDelta(d) => {
                            streamed = true;
                            print!("{d}");
                            let _ = io::stdout().flush();
                        }
                        saif_core::AgentEvent::FinalResponse(full) => {
                            if !streamed && !full.is_empty() {
                                print!("{full}");
                            }
                            println!();
                        }
                        _ => {}
                    }
                }
            });
            let opts = TurnOptions {
                session_id: cli.session.clone(),
                source: "oneshot".to_string(),
                model_override: cli.model.clone(),
                provider_override: cli.provider.clone(),
                base_url_override: cli.base_url.clone(),
                cwd: rt.cwd.clone(),
                ..TurnOptions::default()
            };
            let res = rt
                .agent
                .run_conversation(
                    prompt,
                    None,
                    opts,
                    Some(ev_tx),
                    CancellationToken::new(),
                    None,
                )
                .await;
            let _ = printer.await;
            res.map(|_| ()).map_err(anyhow::Error::from)
        })
        .await;
    }

    if cli.tui {
        let rt = build_effective_runtime(cli.profile.as_deref()).await?;
        return ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
            run_tui(rt.profile, rt.cfg, Some(rt.store), rt.agent, rt.cwd).await
        })
        .await;
    }

    match cli.command {
        None | Some(Commands::Chat) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
                run_repl(rt, cli.model, cli.provider, cli.base_url, cli.session).await
            })
            .await
        }
        Some(Commands::TuiGateway) => {
            let rt = build_effective_runtime(cli.profile.as_deref()).await?;
            ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
                run_tui_gateway(rt.profile, rt.cfg, Some(rt.store), rt.agent, rt.cwd).await
            })
            .await
        }
        Some(Commands::Status) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            let sessions = rt.store.count_sessions().unwrap_or(0);
            println!("Saif Agent v{}", env!("CARGO_PKG_VERSION"));
            println!("Profile:  {}", rt.profile.profile_name);
            println!("Home:     {}", rt.profile.home_dir.display());
            println!(
                "Provider: {}",
                if rt.cfg.provider.is_empty() {
                    "auto"
                } else {
                    &rt.cfg.provider
                }
            );
            println!(
                "Model:    {}",
                if rt.cfg.model.is_empty() {
                    "auto"
                } else {
                    &rt.cfg.model
                }
            );
            println!("Sessions: {sessions}");
            Ok(())
        }
        Some(Commands::Doctor) => {
            let facts = HostFacts::detect();
            let root = default_saif_root();
            println!("Host OS:      {} ({})", facts.os, facts.arch);
            println!("CPUs:         {}", facts.cpu_count);
            println!("WSL / Docker: {} / {}", facts.is_wsl, facts.is_container);
            println!("SAIF_HOME:    {}", root.display());
            println!(
                "git / docker: {} / {}",
                ExecutableResolver::which("git").is_some(),
                ExecutableResolver::which("docker").is_some()
            );
            println!(
                "Providers:    {} built-in profiles",
                builtin_providers().len()
            );
            Ok(())
        }
        Some(Commands::Config { action }) => {
            let rt = build_runtime_for_config(cli.profile.as_deref()).await?;
            match action {
                ConfigAction::Show => {
                    println!("{}", serde_json::to_string_pretty(rt.cfg.as_ref())?);
                }
                ConfigAction::Set { key, value } => {
                    let mut cfg = load_config(&rt.profile.home_dir)?;
                    match key.as_str() {
                        "model" => cfg.model = value,
                        "provider" => cfg.provider = value,
                        "display.skin" => cfg.display.skin = value,
                        "display.language" => cfg.display.language = value,
                        "terminal.backend" => cfg.terminal.backend = value,
                        other => {
                            cfg.extra
                                .insert(other.to_string(), serde_json::Value::String(value));
                        }
                    }
                    atomic_yaml_write(&rt.profile.config_path(), &cfg)?;
                    println!("Updated {key} in {}", rt.profile.config_path().display());
                }
                ConfigAction::SetSecret { key, value } => {
                    SecretScope::write_secret_to_env(&rt.profile.home_dir, &key, &value)?;
                    println!("Saved secret {key} to {}", rt.profile.env_path().display());
                }
            }
            Ok(())
        }
        Some(Commands::Profile { action }) => {
            let root = default_saif_root();
            match action {
                ProfileAction::List => {
                    println!("default\t{}", root.display());
                    for (name, path) in list_live_profiles(&root) {
                        println!("{name}\t{}", path.display());
                    }
                }
                ProfileAction::Create { name } => {
                    validate_profile_id(&name)?;
                    let dir = root.join("profiles").join(&name);
                    clear_named_profile_deleted(&dir)?;
                    fs::create_dir_all(&dir)?;
                    let cfg = SaifConfig::default();
                    atomic_yaml_write(&dir.join("config.yaml"), &cfg)?;
                    println!("Created profile '{name}' at {}", dir.display());
                }
                ProfileAction::Delete { name } => {
                    validate_profile_id(&name)?;
                    let dir = root.join("profiles").join(&name);
                    mark_named_profile_deleted(&dir)?;
                    println!("Tombstoned profile '{name}'");
                }
            }
            Ok(())
        }
        Some(Commands::Sessions { action }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            match action {
                SessionsAction::List { limit } => {
                    for s in rt.store.list_sessions(limit).await? {
                        println!(
                            "{} | {} | {} msgs | {}",
                            s.id, s.source, s.message_count, s.preview
                        );
                    }
                }
                SessionsAction::Search { query, limit } => {
                    for hit in rt.store.search_messages(&query, limit).await? {
                        println!(
                            "[{} {}] {} -> {}",
                            hit.session_id, hit.role, hit.source, hit.snippet
                        );
                    }
                }
            }
            Ok(())
        }
        Some(Commands::Skills { action }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            let store = SkillStore::new(rt.profile.skills_dir(), &rt.cfg.skills.disabled);
            match action {
                SkillsAction::List => {
                    for s in store.list_skills() {
                        println!("{}\t{}", s.name, s.prompt_description);
                    }
                }
                SkillsAction::View { name } => {
                    let doc = store.view_skill(&name)?;
                    println!("{}", doc.raw);
                }
                SkillsAction::Create {
                    name,
                    description,
                    body,
                    category,
                } => {
                    let p = store.create_skill(&name, &description, &body, category.as_deref())?;
                    println!("Created skill at {}", p.display());
                }
                SkillsAction::Delete { name } => {
                    let curator = Curator::new(rt.profile.skills_dir());
                    store.delete_skill(&name, curator.is_pinned(&name))?;
                    println!("Deleted skill '{name}'");
                }
            }
            Ok(())
        }
        Some(Commands::Curator { action }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            let store = SkillStore::new(rt.profile.skills_dir(), &rt.cfg.skills.disabled);
            let curator = Curator::new(rt.profile.skills_dir());
            match action {
                CuratorAction::Status => {
                    println!("{}", serde_json::to_string_pretty(&curator.load_usage())?);
                }
                CuratorAction::Run { no_backup } => {
                    let report = curator.run_cycle(
                        &store,
                        rt.cfg.curator.stale_after_days,
                        rt.cfg.curator.archive_after_days,
                        !no_backup && rt.cfg.curator.backup_enabled,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                CuratorAction::Pin { name } => {
                    curator.set_pinned(&name, true)?;
                    println!("Pinned skill '{name}'");
                }
                CuratorAction::Unpin { name } => {
                    curator.set_pinned(&name, false)?;
                    println!("Unpinned skill '{name}'");
                }
            }
            Ok(())
        }
        Some(Commands::Cron { action }) => {
            let rt = build_effective_runtime(cli.profile.as_deref()).await?;
            let cron_store = CronStore::new(rt.profile.cron_dir());
            match action {
                CronAction::List => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&cron_store.load_jobs()?)?
                    );
                }
                CronAction::Add {
                    name,
                    schedule,
                    prompt,
                } => {
                    let job = cron_store.create_job(&name, &schedule, &prompt, None, None)?;
                    println!("{}", serde_json::to_string_pretty(&job)?);
                }
                CronAction::Pause { id } => {
                    let job = cron_store.set_enabled(&id, false)?;
                    println!("Paused {}", job.id);
                }
                CronAction::Resume { id } => {
                    let job = cron_store.set_enabled(&id, true)?;
                    println!("Resumed {}", job.id);
                }
                CronAction::Remove { id } => {
                    cron_store.remove_job(&id)?;
                    println!("Removed {id}");
                }
                CronAction::Tick => {
                    let (executed, skipped) = run_cron_tick_once(&rt).await?;
                    println!("Due jobs: {executed} (locked={skipped})");
                }
            }
            Ok(())
        }
        Some(Commands::Kanban { action }) => {
            let root = default_saif_root();
            let kb = KanbanStore::open(&root.join("kanban.db"))?;
            match action {
                KanbanAction::Init => {
                    println!("Initialized Kanban DB at {}", kb.db_path().display());
                }
                KanbanAction::List { status } => {
                    let st = status.as_deref().map(TaskStatus::parse);
                    println!("{}", serde_json::to_string_pretty(&kb.list_tasks(st)?)?);
                }
                KanbanAction::Create {
                    title,
                    body,
                    assignee,
                } => {
                    let task =
                        kb.create_task(&title, body.as_deref(), assignee.as_deref(), 0, &[])?;
                    println!("{}", serde_json::to_string_pretty(&task)?);
                }
                KanbanAction::Show { id } => {
                    println!("{}", serde_json::to_string_pretty(&kb.show_task(&id)?)?);
                }
                KanbanAction::Complete { id, summary } => {
                    kb.complete_task(&id, &summary)?;
                    println!("Completed {id}");
                }
                KanbanAction::Block { id, reason } => {
                    let st = kb.record_failure(&id, &reason, None, 1)?;
                    println!("Task {id} status -> {st:?}");
                }
                KanbanAction::Unblock { id } => {
                    kb.unblock_task(&id)?;
                    println!("Unblocked {id}");
                }
                KanbanAction::Comment { id, body } => {
                    let cid = kb.add_comment(&id, "cli", &body)?;
                    println!("Added comment #{cid}");
                }
            }
            Ok(())
        }
        Some(Commands::Gateway { action }) => match action {
            GatewayAction::Platforms => {
                for spec in builtin_platform_specs() {
                    println!(
                        "{}\t{}\tmax_chars={}\tedit={}",
                        spec.id, spec.display_name, spec.max_chars, spec.supports_edit
                    );
                }
                Ok(())
            }
            GatewayAction::Migrate { multiplex } => {
                if !multiplex {
                    anyhow::bail!("pass --multiplex to converge gateways onto host multiplexer");
                }
                let root = default_saif_root();
                let manifest =
                    converge_multiplex_migration(&root).map_err(|e| anyhow::anyhow!(e))?;
                println!("{}", serde_json::to_string_pretty(&manifest)?);
                Ok(())
            }
            GatewayAction::InstallService {
                kind,
                listen,
                output,
            } => {
                let svc_kind = ServiceKind::parse(&kind).map_err(|e| anyhow::anyhow!(e))?;
                let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("saif"));
                let root = default_saif_root();
                let rendered = render_service_unit(svc_kind, &exe, &root, listen);
                if let Some(path) = output {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&path, &rendered)?;
                    println!("Wrote service unit -> {}", path.display());
                } else {
                    print!("{rendered}");
                }
                Ok(())
            }
            GatewayAction::Run { listen } => {
                let rt = build_effective_runtime(cli.profile.as_deref()).await?;
                let root = default_saif_root();
                let mut mux = GatewayMultiplexer::new(&rt.profile.profile_name);
                mux.add_slot(
                    Arc::clone(&rt.profile),
                    Arc::clone(&rt.cfg),
                    rt.agent.clone(),
                );
                for (name, _) in list_live_profiles(&root) {
                    if name != rt.profile.profile_name {
                        if let Ok(slot_rt) = build_effective_runtime(Some(&name)).await {
                            mux.add_slot(slot_rt.profile, slot_rt.cfg, slot_rt.agent);
                        }
                    }
                }
                mux.preflight_credential_conflicts()
                    .map_err(|e| anyhow::anyhow!(e))?;
                let mux_arc = Arc::new(mux);

                for slot in mux_arc.slots() {
                    if let Some(tg_token) = slot.profile.secrets.get_secret("TELEGRAM_BOT_TOKEN") {
                        let allow_all = slot.config.gateway.allow_all_users;
                        let allowed_users = slot.config.gateway.allowed_users.clone();
                        let tg_agent = slot.agent.clone();
                        let tg_cwd = rt.cwd.clone();
                        let pname = slot.profile.profile_name.clone();
                        let scope = Arc::clone(&slot.profile);
                        tokio::spawn(async move {
                            ProfileScope::run_scoped(scope, async move {
                                let mut adapter =
                                    TelegramAdapter::new(tg_token, pname, allow_all, allowed_users);
                                let _ = adapter.run_polling(tg_agent, tg_cwd).await;
                            })
                            .await;
                        });
                    }
                }

                // Background cron scheduler loop across all multiplexed profile slots
                let cron_mux = Arc::clone(&mux_arc);
                let cron_cwd = rt.cwd.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        for slot in cron_mux.slots() {
                            let _ = execute_cron_for_profile(
                                Arc::clone(&slot.profile),
                                Arc::clone(&slot.config),
                                slot.agent.clone(),
                                cron_cwd.clone(),
                            )
                            .await;
                        }
                    }
                });

                ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
                    let router = build_gateway_router(GatewayHttpState {
                        agent: rt.agent,
                        cwd: rt.cwd,
                        multiplexer: Some(mux_arc),
                    });
                    let listener = tokio::net::TcpListener::bind(listen).await?;
                    println!("saif-gateway listening on http://{listen}");
                    axum::serve(listener, router).await?;
                    Ok(())
                })
                .await
            }
        },
        Some(Commands::Serve { listen }) => {
            let rt = build_effective_runtime(cli.profile.as_deref()).await?;
            let state = build_web_state(rt, true);
            run_web_server(listen, state).await
        }
        Some(Commands::Dashboard { listen }) => {
            let rt = build_effective_runtime(cli.profile.as_deref()).await?;
            let state = build_web_state(rt, false);
            run_web_server(listen, state).await
        }
        Some(Commands::Plugins { action }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            let mgr = PluginManager::new(rt.profile.plugins_dir(), &rt.cfg.plugins.disabled);
            match action {
                PluginsAction::List => {
                    for p in mgr.discover() {
                        println!(
                            "{}\t{:?}\tenabled={}",
                            p.manifest.name, p.manifest.kind, p.enabled
                        );
                    }
                }
                PluginsAction::Validate { manifest_path } => {
                    let m = PluginManager::validate_manifest_file(&manifest_path)?;
                    println!("Valid plugin manifest: {} ({:?})", m.name, m.kind);
                }
            }
            Ok(())
        }
        Some(Commands::Mcp { action }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            let store = McpTrustStore::new(&rt.profile.home_dir);
            match action {
                McpAction::List => {
                    for (name, entry) in &rt.cfg.mcp_servers {
                        println!(
                            "{name}\t{}\ttrusted={}",
                            entry.command,
                            store.is_trusted(name)
                        );
                    }
                }
                McpAction::Trust { server_name } => {
                    store.grant_trust(&server_name)?;
                    println!(
                        "Trusted MCP server '{server_name}' for profile '{}'",
                        rt.profile.profile_name
                    );
                }
                McpAction::Check { server_name } => {
                    println!(
                        "server={server_name} trusted={}",
                        store.is_trusted(&server_name)
                    );
                }
            }
            Ok(())
        }
        Some(Commands::Migrate {
            from_hermes,
            source,
            dry_run,
        }) => {
            if !from_hermes {
                anyhow::bail!("pass --from-hermes to migrate from ~/.hermes");
            }
            let src =
                source.unwrap_or_else(|| saif_config::profile::user_home_dir().join(".hermes"));
            let dst = default_saif_root();
            let report = migrate_from_hermes(&src, &dst, dry_run)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some(Commands::Batch {
            input,
            output,
            workers,
            resume,
            format,
        }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            if format == "raw" {
                let raw = fs::read_to_string(&input)?;
                let mut out_lines = Vec::new();
                for line in raw.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let item: serde_json::Value = serde_json::from_str(line)?;
                    let prompt = item.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
                    let opts = TurnOptions {
                        source: "batch".to_string(),
                        skip_persistence: true,
                        cwd: rt.cwd.clone(),
                        ..TurnOptions::default()
                    };
                    let outcome = rt
                        .agent
                        .run_conversation(prompt, None, opts, None, CancellationToken::new(), None)
                        .await?;
                    out_lines.push(serde_json::to_string(&serde_json::json!({
                        "prompt": prompt,
                        "response": outcome.final_response,
                        "usage": outcome.usage
                    }))?);
                }
                fs::write(&output, out_lines.join("\n") + "\n")?;
                println!(
                    "Processed {} raw batch entries -> {}",
                    out_lines.len(),
                    output.display()
                );
            } else {
                let recs =
                    run_batch_dataset(rt.agent, &input, &output, workers, resume, rt.cwd).await?;
                println!(
                    "Processed {} ShareGPT batch entries -> {}",
                    recs.len(),
                    output.display()
                );
            }
            Ok(())
        }
        Some(Commands::Auth { action }) => {
            let rt = build_cli_runtime(cli.profile.as_deref()).await?;
            let mut pool = CredentialPool::open(&rt.profile.home_dir);
            match action {
                AuthAction::List { provider } => {
                    let entries = pool.list(provider.as_deref());
                    for e in entries {
                        println!(
                            "{}\t{}\t{:?}\tuses={}\tsource={}",
                            e.provider, e.id, e.status, e.use_count, e.source
                        );
                    }
                }
                AuthAction::Add {
                    provider,
                    id,
                    secret,
                } => {
                    pool.add_credential(&provider, &id, &secret, "manual");
                    pool.save()?;
                    println!("Added pooled credential '{id}' for '{provider}'");
                }
                AuthAction::Strategy { strategy } => {
                    let parsed = match strategy.trim().to_ascii_lowercase().as_str() {
                        "fill_first" => PoolStrategy::FillFirst,
                        "round_robin" => PoolStrategy::RoundRobin,
                        "random" => PoolStrategy::Random,
                        "least_used" => PoolStrategy::LeastUsed,
                        other => anyhow::bail!(
                            "unsupported strategy '{other}' (expected fill_first|round_robin|random|least_used)"
                        ),
                    };
                    pool.set_strategy(parsed);
                    pool.save()?;
                    println!("Updated auth pool strategy -> {parsed:?}");
                }
                AuthAction::Status => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "strategy": pool.strategy(),
                            "credentials": pool.list(None).len()
                        }))?
                    );
                }
            }
            Ok(())
        }
        Some(Commands::Upstream { action }) => handle_upstream_cmd(action),
    }
}

async fn build_runtime_for_config(profile_name: Option<&str>) -> anyhow::Result<RuntimeBundle> {
    build_cli_runtime(profile_name).await
}

async fn run_cron_tick_once(rt: &RuntimeBundle) -> anyhow::Result<(usize, bool)> {
    execute_cron_for_profile(
        Arc::clone(&rt.profile),
        Arc::clone(&rt.cfg),
        rt.agent.clone(),
        rt.cwd.clone(),
    )
    .await
}

async fn execute_cron_for_profile(
    profile: Arc<ProfileScope>,
    cfg: Arc<SaifConfig>,
    agent: AgentRunner,
    default_cwd: PathBuf,
) -> anyhow::Result<(usize, bool)> {
    let cron_store = CronStore::new(profile.cron_dir());
    let timeout_secs = cfg.cron.inactivity_timeout_seconds.max(1);
    let sched = CronScheduler::new(cron_store.clone(), timeout_secs);
    let outcome = sched.tick_due_jobs()?;
    if outcome.skipped_due_to_lock {
        return Ok((0, true));
    }
    let count = outcome.due_jobs.len();
    for job in outcome.due_jobs {
        let job_cwd = job
            .workdir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_cwd.clone());
        let _ = fs::create_dir_all(cron_store.output_dir());
        let out_file = cron_store.output_dir().join(format!("{}.json", job.id));

        if job.no_agent {
            let script_str = job.script.as_deref().unwrap_or("");
            let res: Result<(), String> =
                match CronStore::validate_script_path(&profile.home_dir, script_str) {
                    Ok(script_path) => {
                        let child_env = profile
                            .secrets
                            .build_child_env(&profile.home_dir, &cfg.terminal.env_passthrough);
                        match tokio::time::timeout(
                            Duration::from_secs(timeout_secs),
                            tokio::process::Command::new(&script_path)
                                .current_dir(&job_cwd)
                                .env_clear()
                                .envs(&child_env)
                                .output(),
                        )
                        .await
                        {
                            Ok(Ok(out)) => {
                                let payload = serde_json::json!({
                                    "job_id": job.id,
                                    "exit_code": out.status.code(),
                                    "stdout": String::from_utf8_lossy(&out.stdout),
                                    "stderr": String::from_utf8_lossy(&out.stderr),
                                });
                                let _ = fs::write(
                                    &out_file,
                                    serde_json::to_string_pretty(&payload).unwrap_or_default(),
                                );
                                if out.status.success() {
                                    Ok(())
                                } else {
                                    Err(format!("script exited with {:?}", out.status.code()))
                                }
                            }
                            Ok(Err(e)) => Err(e.to_string()),
                            Err(_) => Err(format!("timeout after {timeout_secs}s")),
                        }
                    }
                    Err(e) => Err(e.to_string()),
                };
            let (status, err_msg) = match res {
                Ok(()) => ("ok", None),
                Err(ref e) if e.starts_with("timeout") => ("timeout", Some(e.as_str())),
                Err(ref e) => ("error", Some(e.as_str())),
            };
            let _ = cron_store.record_completion(&job.id, status, err_msg);
        } else {
            let scope = Arc::clone(&profile);
            let runner = agent.clone();
            let opts = TurnOptions {
                source: "cron".to_string(),
                model_override: job.model.clone(),
                provider_override: job.provider.clone(),
                cwd: job_cwd,
                ..TurnOptions::default()
            };
            let fut = ProfileScope::run_scoped(scope, async move {
                runner
                    .run_conversation(
                        &job.prompt,
                        None,
                        opts,
                        None,
                        CancellationToken::new(),
                        None,
                    )
                    .await
            });
            match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
                Ok(Ok(out)) => {
                    let payload = serde_json::json!({
                        "job_id": job.id,
                        "session_id": out.session_id,
                        "response": out.final_response,
                    });
                    let _ = fs::write(
                        &out_file,
                        serde_json::to_string_pretty(&payload).unwrap_or_default(),
                    );
                    let _ = cron_store.record_completion(&job.id, "ok", None);
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let _ = cron_store.record_completion(&job.id, "error", Some(&msg));
                }
                Err(_) => {
                    let msg = format!("timeout after {timeout_secs}s");
                    let _ = cron_store.record_completion(&job.id, "timeout", Some(&msg));
                }
            }
        }
    }
    Ok((count, false))
}

fn resolve_upstream_lock_path(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    let cwd = std::env::current_dir()?;
    let rel = cwd.join("conformance/UPSTREAM_LOCK.json");
    if rel.exists() {
        return Ok(rel);
    }
    if let Ok(exe) = std::env::current_exe() {
        for anc in exe.ancestors() {
            let cand = anc.join("conformance/UPSTREAM_LOCK.json");
            if cand.exists() {
                return Ok(cand);
            }
        }
    }
    let manifest_rel = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/UPSTREAM_LOCK.json");
    if manifest_rel.exists() {
        return Ok(manifest_rel);
    }
    anyhow::bail!("could not locate conformance/UPSTREAM_LOCK.json; pass --lock <path>")
}

fn resolve_remote_head_sha(
    lock: &UpstreamLock,
    local_repo: Option<&Path>,
) -> anyhow::Result<String> {
    if let Some(repo) = local_repo {
        let out = StdCommand::new("git")
            .args([
                "-C",
                &repo.to_string_lossy(),
                "rev-parse",
                &lock.tracked_branch,
            ])
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "git rev-parse failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let ref_name = format!("refs/heads/{}", lock.tracked_branch);
    let out = StdCommand::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", &lock.upstream_repo, &ref_name])
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git ls-remote failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let sha = stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty ls-remote response for {ref_name}"))?;
    Ok(sha.to_string())
}

fn git_name_status_diff(
    lock: &UpstreamLock,
    from: &str,
    to: &str,
    local_repo: Option<&Path>,
) -> anyhow::Result<String> {
    if let Some(repo) = local_repo {
        let out = StdCommand::new("git")
            .args([
                "-C",
                &repo.to_string_lossy(),
                "diff",
                "--name-status",
                from,
                to,
            ])
            .output()?;
        if !out.status.success() {
            anyhow::bail!("git diff failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }

    let tmp_bare = tempfile::TempDir::new()?;
    let tmp_path = tmp_bare.path().to_string_lossy().into_owned();
    let init = StdCommand::new("git")
        .args(["init", "--bare", &tmp_path])
        .output()?;
    if !init.status.success() {
        anyhow::bail!("git init --bare failed");
    }

    let fetch = StdCommand::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-C",
            &tmp_path,
            "fetch",
            "--no-tags",
            "--filter=blob:none",
            "--depth=1",
            &lock.upstream_repo,
            from,
            to,
        ])
        .output()?;
    if !fetch.status.success() {
        let branch_fetch = StdCommand::new("git")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args([
                "-C",
                &tmp_path,
                "fetch",
                "--no-tags",
                "--filter=blob:none",
                &lock.upstream_repo,
                &lock.tracked_branch,
            ])
            .output()?;
        if !branch_fetch.status.success() {
            anyhow::bail!(
                "git fetch failed: {}",
                String::from_utf8_lossy(&branch_fetch.stderr)
            );
        }
    }

    let diff = StdCommand::new("git")
        .args(["-C", &tmp_path, "diff", "--name-status", from, to])
        .output()?;
    if !diff.status.success() {
        anyhow::bail!("git diff failed: {}", String::from_utf8_lossy(&diff.stderr));
    }
    Ok(String::from_utf8_lossy(&diff.stdout).into_owned())
}

fn handle_upstream_cmd(action: UpstreamAction) -> anyhow::Result<()> {
    match action {
        UpstreamAction::Status { lock, local_repo } => {
            let lock_path = resolve_upstream_lock_path(lock)?;
            let parsed = UpstreamLock::load(&lock_path).map_err(|e| anyhow::anyhow!(e))?;
            let remote_sha = resolve_remote_head_sha(&parsed, local_repo.as_deref())?;
            let state = if remote_sha.eq_ignore_ascii_case(&parsed.synced_commit) {
                "UP_TO_DATE"
            } else {
                "UPDATE_AVAILABLE"
            };
            println!("Upstream Repo:   {}", parsed.upstream_repo);
            println!("Tracked Branch:  {}", parsed.tracked_branch);
            println!("Synced Commit:   {}", parsed.synced_commit);
            println!("Upstream HEAD:   {remote_sha}");
            println!("Status:          {state}");
            Ok(())
        }
        UpstreamAction::Diff {
            from,
            to,
            subsystem,
            port_only,
            json,
            lock,
            local_repo,
        } => {
            let lock_path = resolve_upstream_lock_path(lock)?;
            let parsed = UpstreamLock::load(&lock_path).map_err(|e| anyhow::anyhow!(e))?;
            let from_sha = from.unwrap_or_else(|| parsed.synced_commit.clone());
            let to_sha = match to {
                Some(t) => t,
                None => resolve_remote_head_sha(&parsed, local_repo.as_deref())?,
            };
            let raw_diff =
                git_name_status_diff(&parsed, &from_sha, &to_sha, local_repo.as_deref())?;
            let entries = parse_git_name_status(&raw_diff);
            let report = parsed.build_report(&from_sha, &to_sha, entries);
            let output = if json {
                serde_json::to_string_pretty(&report)? + "\n"
            } else {
                report.render_markdown(port_only, subsystem.as_deref())
            };
            if let Err(err) = io::stdout().write_all(output.as_bytes()) {
                if err.kind() != io::ErrorKind::BrokenPipe {
                    return Err(err.into());
                }
            }
            Ok(())
        }
        UpstreamAction::Ack { sha, subject, lock } => {
            let lock_path = resolve_upstream_lock_path(lock)?;
            let mut parsed = UpstreamLock::load(&lock_path).map_err(|e| anyhow::anyhow!(e))?;
            parsed
                .ack(&sha, subject.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?;
            parsed.save(&lock_path).map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "Updated {} -> {}",
                lock_path.display(),
                parsed.synced_commit
            );
            Ok(())
        }
    }
}

fn build_web_state(rt: RuntimeBundle, headless: bool) -> WebState {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "saif".to_string());
    WebState {
        session_token: Uuid::new_v4().to_string(),
        headless,
        profile: rt.profile,
        config: rt.cfg,
        store: Some(rt.store),
        agent: rt.agent,
        pty_program: exe,
        pty_args: vec!["--tui".to_string()],
        cwd: rt.cwd,
    }
}

async fn run_repl(
    rt: RuntimeBundle,
    model_override: Option<String>,
    provider_override: Option<String>,
    base_url_override: Option<String>,
    initial_session: Option<String>,
) -> anyhow::Result<()> {
    println!(
        "Saif Agent v{} (profile: {}, /help for commands, /quit to exit)",
        env!("CARGO_PKG_VERSION"),
        rt.profile.profile_name
    );
    let stdin = io::stdin();
    let mut history: Vec<ChatMessage> = Vec::new();
    let mut session_id = initial_session;

    loop {
        print!("saif> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }
        if prompt == "/quit" || prompt == "/exit" {
            break;
        }
        if prompt == "/new" {
            session_id = None;
            history.clear();
            println!("Started new session.");
            continue;
        }
        if prompt == "/help" {
            println!("Commands: /new, /quit, /skill <name> <task>");
            continue;
        }

        let opts = TurnOptions {
            session_id: session_id.clone(),
            source: "cli".to_string(),
            model_override: model_override.clone(),
            provider_override: provider_override.clone(),
            base_url_override: base_url_override.clone(),
            cwd: rt.cwd.clone(),
            ..TurnOptions::default()
        };
        let prior = (!history.is_empty()).then(|| history.clone());
        match rt
            .agent
            .run_conversation(prompt, prior, opts, None, CancellationToken::new(), None)
            .await
        {
            Ok(out) => {
                session_id = Some(out.session_id);
                history = out.messages;
                println!("{}", out.final_response);
            }
            Err(e) => {
                eprintln!("Error: {e}");
            }
        }
    }
    Ok(())
}
