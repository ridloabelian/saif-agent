# `saif-agent` Port & Conformance Map (`rust-anything` Methodology)

- **Reference Oracle**: `NousResearch/hermes-agent` (`07f075c2`, Python + TypeScript)
- **Target**: `saif-agent` (100% Pure-Rust Workspace + WASM Web UI + Tauri v2 Desktop)
- **ADRs**: [DECISIONS.md](DECISIONS.md) | **Glossary**: [GLOSSARY.md](GLOSSARY.md) | **Anatomy**: [../docs/HERMES_ANATOMY.md](../docs/HERMES_ANATOMY.md)

## Wave Plan

### Wave 1 — Foundation, Storage, Providers, Skills, Cron, Plugins
| Unit / Crate | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| `saif-platform` | `hermes_platform/`, `process_registry_termination.py` | done | Host facts, binary lookup, `(pid, start_time)`, parent-first teardown |
| `saif-config` | `hermes_constants.py`, `secret_scope.py`, `config*.py`, `skin_engine.py`, `i18n.py` | done | Task-local `ProfileScope`, fail-closed `SecretScope`, 3 loaders, migration |
| `saif-persistence` | `persistence/`, `hermes_state*.py` (v32 + FTS5 v3), `kanban_db.py` | done | Bundled SQLite + FTS5 + trigram + 7-table Kanban store |
| `saif-providers` | `providers/`, `anthropic_adapter.py`, `chat_completions.py`, `auxiliary_client.py` | done | 21 provider profiles, OpenAI + Anthropic SSE/caching, Aux router |
| `saif-skills` | `skill_utils.py`, `threat_patterns.py`, `skill_usage.py`, `curator*.py` | done | Frontmatter, 60-char clip, threat scan, Curator archive/tar.gz |
| `saif-cron` | `cron/jobs.py`, `cron/scheduler*.py`, `cron/occurrences.py` | done | Duration/natural/cron5/ISO parser, catch-up grace, `pending_slot`, tick lock |
| `saif-plugins` | `plugins/`, `plugin_catalog.py`, `plugin_host*.py` | done | Manifests, SHA-40 & `removed.yaml` gate, HookRegistry, host RPC client |

### Wave 2 — Tools, Agent Loop, Contracts, Surfaces, CLI, Conformance
| Unit / Crate | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| `saif-tools` | `toolsets.py`, `tools/registry.py`, `fuzzy_match.py`, `patch_parser.py`, core tools, `environments/` | done | Core toolset, BM25 `tool_search`, V4A/fuzzy patch, Local/Docker/SSH, MCP stdio |
| `saif-core` | `run_agent.py`, `prompt_builder.py`, `context_compressor.py`, `conversation_loop.py`, `delegate_tool.py` | done | Byte-stable prompt cache, role alternation, `/steer`, 2-tier compressor, subagents |
| `saif-rpc-contract` | `tui_gateway/contracts/` | done | WASM-safe shared JSON-RPC 2.0 wire schema |
| `saif-tui` | `ui-tui/`, `tui_gateway/server.py` | done | Native `ratatui` + `crossterm` TUI + stdio `tui-gateway` |
| `saif-gateway` | `gateway/run.py`, `session_identity.py`, `api_server`, `webhook`, `telegram` | done | `RoutingIdentity`, dual busy guard, 4-invariant `StreamDraftController` |
| `saif-web` + `apps/saif-web-ui` | `web/`, `hermes_cli/web_routers/`, `pty_bridge.py` | done | Axum REST/WS + `/api/pty` + pure-Rust `wasm32-unknown-unknown` UI |
| `apps/saif-desktop` | `apps/desktop/` | done | Pure-Rust Tauri v2 desktop scaffold managing `saif serve` |
| `saif-acp` | `acp_adapter/` | done | Native `saif-acp` stdio server |
| `saif-cli` | `cli.py`, `hermes_cli/main.py`, `subcommands/` | done | Unified `saif` binary + all core subcommands |
| `saif-conformance` | `conformance/cases/*.jsonl` + invariant suite | done | Golden replay against Hermes + E2E mock-SSE tests |

### Wave 3 — Full Provider & Transport Parity
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Codex Responses & App-Server | `agent/codex_responses_adapter.py`, `agent/transports/codex*.py` | open | Native Codex `/responses` wire + reasoning replay items |
| AWS Bedrock & Azure/Vertex OAuth | `agent/bedrock_adapter.py`, `azure_identity_adapter.py`, `plugins/model-providers/vertex` | open | SigV4 Bedrock Converse stream + Azure/Vertex credential chains |
| Credential Pool Rotation & RL `batch_runner` | `agent/credential_pool.py`, `batch_runner.py` | open | Multi-key rate-limit rotation + ShareGPT trajectory compression |

### Wave 4 — Remote Sandboxes, Browser CDP, & Code Execution Kernel
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Cloud Sandboxes | `tools/environments/{modal,daytona,vercel_sandbox}.py` | open | REST/gRPC remote execution adapters |
| Native Chromium CDP Driver | `tools/browser_use_cli.py`, `browser_launcher.py`, `browser_vault_tool.py` | open | Direct WebSocket CDP + Chromium process ledger + vault autofill |
| Programmatic `execute_code` & `computer_use` | `tools/code_execution_tool.py`, `tools/computer_use_tool.py` | open | Sandboxed RPC code kernel & screen/input automation |

### Wave 5 — Full 25+ Gateway Platforms & Multi-Profile Multiplexing
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Chat & Collaboration Adapters | `plugins/platforms/{discord,slack,whatsapp,signal,matrix,email,feishu,irc,mattermost,google_chat,line,teams,wecom,weixin,qqbot,dingtalk,sms,a2a,...}` | open | Adapter parity implementing `PlatformAdapter` + `StreamDraftController` |
| Gateway Multiplexer & Service Units | `hermes_cli/gateway_migrate.py`, systemd/launchd/Windows units | open | Single-process multi-profile bot multiplexing & OS supervisor integration |

### Wave 6 — Complete WASM Dashboard, Tauri v2 Installer, TTS/STT & Media
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Full Leptos/Yew Dashboard + Xterm WASM | `apps/saif-web-ui` | open | Interactive browser PTY canvas, Kanban board viewer, live logs, config editor |
| Tauri v2 Desktop Bundling & Updater | `apps/saif-desktop` | open | Signed macOS/Windows/Linux bundles + self-update verification |
| Voice (TTS/STT) & Media Gen | `tools/tts_tool_providers.py`, `image_generation_tool.py`, `plugins/video_gen/` | open | Edge TTS, ElevenLabs, OpenAI/xAI audio, Fal/OpenRouter image/video |

## Verification & Benchmark Summary

Verified after final Wave 1–2 edits on `rustc 1.100.0` / `cargo 1.100.0` / `rust-anything v2.0.2`:

- **Formatting & Static Analysis**:
  - `cargo fmt --all -- --check`: passed (exit `0`).
  - `cargo clippy --workspace --all-targets -- -D warnings`: passed across all 16 workspace crates with `0` warnings and `0` unused-dependency manifest warnings.
  - Stub scan (`grep -rn --include="*.rs" "todo!\|unimplemented!" crates/ apps/`): `0` matches (`ZERO_STUBS`).
- **Workspace Test Suite (`cargo test --workspace`)**: **19 passed, 0 failed, 0 ignored**:
  - `saif-platform` unit tests: **4 passed** (`host_facts_detects_non_empty_values`, `current_process_identity_matches_itself`, `which_finds_sh_on_unix`, `parent_first_termination_reaps_child`).
  - `saif-conformance` `tests/oracle_replay.rs`: **6 passed** replaying **67 golden behavioral cases** recorded from `NousResearch/hermes-agent` (`cron_schedules.jsonl` [31], `patch_fuzzy.jsonl` [11], `skills_frontmatter.jsonl` [7], `prompt_injection_scan.jsonl` [7], `sqlite_fts_preview.jsonl` [5], `profile_paths.jsonl` [6]).
  - `saif-conformance` `tests/invariants_e2e.rs`: **7 passed** (`secret_scope_fail_closed_under_multiplex`, `role_alternation_allows_steer_after_tool_forbids_double_user`, `gateway_streaming_four_invariants_and_busy_guard`, `plugin_catalog_enforces_sha40_and_removed_kill_list`, `kanban_dependencies_and_pty_resize_contract`, `hermes_migration_is_non_destructive`, `end_to_end_agent_loop_with_mock_openai_tool_call_and_prompt_cache_stability`).
  - `saif-cli` `tests/cli_e2e.rs`: **2 passed** (`cli_version_and_doctor_run_cleanly`, `cli_status_cron_kanban_profile_and_migrate_e2e`).
- **WebAssembly & Desktop Targets**:
  - `cargo check --manifest-path apps/saif-web-ui/Cargo.toml --target wasm32-unknown-unknown`: passed (exit `0`).
  - `apps/saif-desktop/src-tauri`: pure-Rust Tauri v2 wrapper scaffold (`tauri.conf.json` + `src/main.rs`), excluded from headless Linux workspace build because `webkit2gtk` GUI libraries are not installed on this server.
- **Release Binaries (`cargo build --release -p saif-cli`)**:
  - `target/release/saif`: `12M`
  - `target/release/saif-acp`: `9.3M`
  - Isolated `SAIF_HOME` smoke tests (`saif --version`, `saif doctor`, `saif status`, `saif profile list`): all exited `0` (detected `21` built-in provider profiles).
- **Cold Startup Benchmark (`--version`, wall-clock)**:
  - `./target/release/saif --version` (5 runs, isolated temp `SAIF_HOME`): `[5.0, 2.51, 2.46, 2.4, 2.8]` ms -> **min `2.40 ms` | median `2.51 ms` | mean `3.03 ms`**.
  - `/home/ridlo/.local/bin/hermes --version` (2 read-only runs): `[779.92, 371.73]` ms -> **min `371.73 ms` | median `575.83 ms` | mean `575.83 ms`**.
  - **Median speedup**: **`229.4x`**.
