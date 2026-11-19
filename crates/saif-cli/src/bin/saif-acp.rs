use clap::Parser;
use saif_acp::run_acp_stdio;
use saif_config::{default_saif_root, load_config, ProfileScope};
use saif_core::AgentRunner;
use saif_persistence::SqliteStore;
use saif_plugins::HookRegistry;
use saif_tools::{register_all_builtin_tools, ProcessRegistry, TodoStore, ToolRegistry};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "saif-acp",
    version,
    about = "Saif Agent Client Protocol stdio server"
)]
struct AcpArgs {
    #[arg(short, long)]
    profile: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = AcpArgs::parse();
    let root = default_saif_root();
    let multiplex = ProfileScope::detect_multiplex_active(&root);
    let profile = Arc::new(ProfileScope::for_profile(
        root,
        args.profile.as_deref(),
        multiplex,
    )?);
    let _ = profile.ensure_layout();
    let cfg = Arc::new(load_config(&profile.home_dir)?);
    let store =
        SqliteStore::open_with_journal_mode(&profile.state_db_path(), &cfg.database.journal_mode)
            .ok();
    let registry = ToolRegistry::new();
    register_all_builtin_tools(&registry, ProcessRegistry::new(), TodoStore::default());
    let agent = AgentRunner::new(profile, cfg, registry, store, HookRegistry::new());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    run_acp_stdio(agent, cwd).await
}
