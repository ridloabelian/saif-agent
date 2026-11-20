# Domain & Symbol Glossary (`hermes-agent` -> `saif-agent`)

Canonical mapping between `NousResearch/hermes-agent` (Python/TypeScript) and `saif-agent` (Pure Rust) so every crate uses identical terminology.

| Hermes Concept / File | Meaning | `saif-agent` Crate & Rust Symbol |
|---|---|---|
| `get_hermes_home()` (`hermes_constants.py`) | Profile-aware home directory (`~/.hermes` or profile subdir) | `saif_config::ProfileScope::home_dir()` (`~/.saif` / `SAIF_HOME`) |
| `named_profile_has_identity()` | Checks `config.yaml`, `.env`, `SOUL.md`, `profile.yaml`, `auth.json`, `state.db` | `saif_config::profile::named_profile_has_identity` |
| `secret_scope.get_secret()` | Fail-closed secret reader under multiplex isolation | `saif_config::SecretScope::get_secret` |
| `agent/redact_*.py` | Structured secret, bearer, connstr, and API key redaction | `saif_config::redact_sensitive_text` |
| `served_profile_child_env()` | Scrubbed subprocess env stripping launch secrets | `saif_config::SecretScope::build_child_env` |
| `load_cli_config` / `load_config` / `load_user_config_effective` | 3 distinct config loaders (`DEFAULT_CONFIG` vs `${VAR}` effective) | `saif_config::loader::{load_cli_config, load_config, load_effective_config, load_effective_saif_config}` |
| `SkinConfig` (`skin_engine.py`) | Pure-data UI theme (`default`, `ares`, `mono`, `slate`) | `saif_config::SkinConfig` |
| `hermes_platform.host` / `ProcessIdentity` | OS/arch facts + `(pid, start_time)` anti-PID-reuse fingerprint | `saif_platform::{HostFacts, ProcessIdentity}` |
| `_terminate_host_pid` (`process_registry_termination.py`) | Parent-first SIGTERM -> grace -> children SIGTERM -> SIGKILL | `saif_platform::process::terminate_tree_parent_first` |
| `SessionDB` (`hermes_state.py`, v32 + FTS5 v3) | Durable SQLite store for sessions, messages, admissions, receipts | `saif_persistence::SqliteStore` (`compact_session_history`) |
| `FTS_TOOL_CONTENT_PREFIX_CHARS` (`8_192`) | Truncation limit for `role='tool'` rows in `messages_fts` | `saif_persistence::FTS_TOOL_CONTENT_PREFIX_CHARS` |
| `_shape_preview` (`hermes_state_common.py`) | 60-char single-line session list preview | `saif_persistence::preview::shape_preview` |
| `kanban_db.py` (`kanban.db`) | Cross-profile SQLite task board with atomic claim CAS | `saif_persistence::KanbanStore` |
| `ProviderProfile` (`providers/base.py`) | Declarative inference provider auth, endpoints, request quirks | `saif_providers::ProviderProfile` (`36` profiles) |
| `anthropic_adapter.py` / `chat_completions.py` | Wire transports (SSE streaming, prompt `cache_control`, reasoning) | `saif_providers::{AnthropicMessagesTransport, OpenAiChatTransport, dispatch_completion_with_failover}` |
| `auxiliary_client.py` | Side-LLM router (`compression`, `title_gen`, `curator`, `vision`) | `saif_providers::AuxiliaryClient` |
| `AIAgent` / `conversation_loop.py` (`run_agent.py`) | Turn loop, budget, grace call, role alternation, streaming events | `saif_core::{AgentRunner, reconstruct_messages_from_records}` |
| `prompt_builder.py` (`DEFAULT_AGENT_IDENTITY`) | Byte-stable system prompt assembly + `SOUL.md` + `.saif.md`/`AGENTS.md` | `saif_core::PromptBuilder` |
| `ContextCompressor` (`context_compressor.py`) | Tool-output pruning + auxiliary LLM summarization + deterministic fallback | `saif_core::ContextCompressor` |
| `ToolRegistry` / `TOOLSETS` (`tools/registry.py`, `toolsets.py`) | Tool schemas, per-home `check_fn` cache, BM25 `tool_search`, trusted MCP | `saif_tools::{ToolRegistry, ToolsetCatalog, register_trusted_mcp_tools}` |
| `fuzzy_find_and_replace` (`tools/fuzzy_match.py`) | 9-strategy progressive edit matcher + escape drift guard | `saif_tools::fuzzy_match::fuzzy_find_and_replace` |
| `parse_v4a_patch` (`tools/patch_parser.py`) | V4A (`*** Begin Patch`) parser & dry-run validator | `saif_tools::patch_parser::parse_v4a_patch` |
| `scan_for_threats` (`tools/threat_patterns.py`) | Cumulative `all`/`context`/`strict` injection & invisible-char scanner | `saif_skills::threat::scan_for_threats` |
| `skill_utils.py` / `Curator` (`agent/curator.py`) | `SKILL.md` frontmatter, 60-char limit, `.archive/` + `.tar.gz` backups | `saif_skills::{SkillStore, Curator}` |
| `cron/jobs.py` / `cron/scheduler.py` | Natural/cron5/ISO parser, catch-up grace, `advance_before_dispatch` | `saif_cron::{parse_schedule, compute_next_run, CronScheduler}` |
| `PluginManager` / `plugin_host.py` | Manifest loader, SHA-40 catalog, hooks, out-of-process stdio host | `saif_plugins::{PluginManager, PluginHookBridge, PluginHostClient}` |
| `GatewayRunner` / `RoutingIdentity` (`gateway/`) | Two-stage busy guard, canonical routing, 4-invariant streaming | `saif_gateway::{GatewayRunner, RoutingIdentity, StreamDraftController}` |
| `tui_gateway/contracts/` | Shared typed JSON-RPC 2.0 envelope, server-requests, events | `saif_rpc_contract::{RpcRequest, RpcResponse, GatewayEvent, ServerRequest}` |
| `ui-tui/` | Terminal UI surface (pure Rust `ratatui`) | `saif_tui::run_tui` |
| `web/` & `hermes_cli/web_routers/` | Axum REST/WS + POSIX PTY bridge + Rust WASM UI | `saif_web` + `apps/saif-web-ui` |
| `apps/desktop/` | Native desktop shell (Tauri v2) | `apps/saif-desktop` |
| `acp_adapter/` | Editor Agent Client Protocol server (`saif-acp`) | `saif_acp::AcpServer` |
| `hermes_cli/` (`cli.py`) | Unified CLI + REPL + subcommand dispatcher (`saif`) | `saif_cli` |
| Upstream Hermes Parity Tracker | Declarative lockfile & net-tree subsystem diff classifier | `saif_config::UpstreamLock` + `saif upstream` (`conformance/UPSTREAM_LOCK.json`) |
