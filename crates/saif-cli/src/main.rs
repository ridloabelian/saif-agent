use clap::{Parser, Subcommand};
use saif_config::{
    atomic_yaml_write, clear_named_profile_deleted, default_saif_root, list_live_profiles,
    load_config, mark_named_profile_deleted, migrate_from_hermes, validate_profile_id,
    ProfileScope, SaifConfig, SecretScope,
};
use saif_core::{run_batch_dataset, AgentEvent, AgentRunner, TurnOptions};
use saif_cron::{CronScheduler, CronStore};
use saif_gateway::{
    build_gateway_router, builtin_platform_specs, converge_multiplex_migration,
    render_service_unit, GatewayHttpState, GatewayMultiplexer, ServiceKind, TelegramAdapter,
};
use saif_persistence::{KanbanStore, PersistenceStore, SqliteStore, TaskStatus};
use saif_platform::{ExecutableResolver, HostFacts};
use saif_plugins::{HookRegistry, PluginManager};
use saif_providers::{builtin_providers, ChatMessage, CredentialPool};
use saif_skills::{Curator, SkillStore};
use saif_tools::{
    register_all_builtin_tools, McpTrustStore, ProcessRegistry, TodoStore, ToolRegistry,
};
use saif_tui::{run_tui, run_tui_gateway};
use saif_web::{run_web_server, WebState};
use std::fs;
use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
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

fn build_runtime(profile_name: Option<&str>) -> anyhow::Result<RuntimeBundle> {
    let root = default_saif_root();
    let multiplex = ProfileScope::detect_multiplex_active(&root);
    let profile = Arc::new(ProfileScope::for_profile(root, profile_name, multiplex)?);
    profile.ensure_layout()?;
    let cfg = Arc::new(load_config(&profile.home_dir)?);
    let store =
        SqliteStore::open_with_journal_mode(&profile.state_db_path(), &cfg.database.journal_mode)?;
    let registry = ToolRegistry::new();
    register_all_builtin_tools(&registry, ProcessRegistry::new(), TodoStore::default());
    let agent = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&cfg),
        registry,
        Some(store.clone()),
        HookRegistry::new(),
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .try_init();

    let cli = Cli::parse();

    if let Some(prompt) = cli.query.as_deref() {
        let rt = build_runtime(cli.profile.as_deref())?;
        return ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
            let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
            let printer = tokio::spawn(async move {
                let mut streamed = false;
                while let Some(ev) = ev_rx.recv().await {
                    match ev {
                        AgentEvent::ContentDelta(d) => {
                            streamed = true;
                            print!("{d}");
                            let _ = io::stdout().flush();
                        }
                        AgentEvent::FinalResponse(full) => {
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
        let rt = build_runtime(cli.profile.as_deref())?;
        return ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
            run_tui(rt.profile, rt.cfg, Some(rt.store), rt.agent, rt.cwd).await
        })
        .await;
    }

    match cli.command {
        None | Some(Commands::Chat) => {
            let rt = build_runtime(cli.profile.as_deref())?;
            ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
                run_repl(rt, cli.model, cli.provider, cli.base_url, cli.session).await
            })
            .await
        }
        Some(Commands::TuiGateway) => {
            let rt = build_runtime(cli.profile.as_deref())?;
            ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
                run_tui_gateway(rt.profile, rt.cfg, Some(rt.store), rt.agent, rt.cwd).await
            })
            .await
        }
        Some(Commands::Status) => {
            let rt = build_runtime(cli.profile.as_deref())?;
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
            let rt = build_runtime(cli.profile.as_deref())?;
            match action {
                ConfigAction::Show => {
                    println!("{}", serde_json::to_string_pretty(rt.cfg.as_ref())?);
                }
                ConfigAction::Set { key, value } => {
                    let mut cfg = (*rt.cfg).clone();
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
            let rt = build_runtime(cli.profile.as_deref())?;
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
            let rt = build_runtime(cli.profile.as_deref())?;
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
            let rt = build_runtime(cli.profile.as_deref())?;
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
            let rt = build_runtime(cli.profile.as_deref())?;
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
                    let sched =
                        CronScheduler::new(cron_store, rt.cfg.cron.inactivity_timeout_seconds);
                    let outcome = sched.tick_due_jobs()?;
                    println!(
                        "Due jobs: {} (locked={})",
                        outcome.due_jobs.len(),
                        outcome.skipped_due_to_lock
                    );
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
                let rt = build_runtime(cli.profile.as_deref())?;
                let root = default_saif_root();
                let mut mux = GatewayMultiplexer::new(&rt.profile.profile_name);
                mux.add_slot(
                    Arc::clone(&rt.profile),
                    Arc::clone(&rt.cfg),
                    rt.agent.clone(),
                );
                for (name, _) in list_live_profiles(&root) {
                    if name != rt.profile.profile_name {
                        if let Ok(slot_rt) = build_runtime(Some(&name)) {
                            mux.add_slot(slot_rt.profile, slot_rt.cfg, slot_rt.agent);
                        }
                    }
                }
                mux.preflight_credential_conflicts()
                    .map_err(|e| anyhow::anyhow!(e))?;
                let mux_arc = Arc::new(mux);

                ProfileScope::run_scoped(Arc::clone(&rt.profile), async move {
                    if let Some(tg_token) = rt.profile.secrets.get_secret("TELEGRAM_BOT_TOKEN") {
                        let allow_all = rt.cfg.gateway.allow_all_users;
                        let tg_agent = rt.agent.clone();
                        let tg_cwd = rt.cwd.clone();
                        let pname = rt.profile.profile_name.clone();
                        tokio::spawn(async move {
                            let mut adapter =
                                TelegramAdapter::new(tg_token, pname, allow_all, Vec::new());
                            let _ = adapter.run_polling(tg_agent, tg_cwd).await;
                        });
                    }
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
            let rt = build_runtime(cli.profile.as_deref())?;
            let state = build_web_state(rt, true);
            run_web_server(listen, state).await
        }
        Some(Commands::Dashboard { listen }) => {
            let rt = build_runtime(cli.profile.as_deref())?;
            let state = build_web_state(rt, false);
            run_web_server(listen, state).await
        }
        Some(Commands::Plugins { action }) => {
            let rt = build_runtime(cli.profile.as_deref())?;
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
            let rt = build_runtime(cli.profile.as_deref())?;
            let store = McpTrustStore::new(&rt.profile.home_dir);
            match action {
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
            let rt = build_runtime(cli.profile.as_deref())?;
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
            let rt = build_runtime(cli.profile.as_deref())?;
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
