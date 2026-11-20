# `saif-agent` Port & Conformance Map (`rust-anything` Methodology)

- **Reference Oracle**: `NousResearch/hermes-agent` (`32dbc1dcda22`, Python + TypeScript; tracked via `conformance/UPSTREAM_LOCK.json`)
- **Target**: `saif-agent` (100% Pure-Rust Workspace + WASM Web UI + Tauri v2 Desktop)
- **ADRs**: [DECISIONS.md](DECISIONS.md) | **Glossary**: [GLOSSARY.md](GLOSSARY.md) | **Anatomy**: [../docs/HERMES_ANATOMY.md](../docs/HERMES_ANATOMY.md) | **Sync**: [../docs/UPSTREAM_SYNC.md](../docs/UPSTREAM_SYNC.md)

## Wave Plan

### Wave 1 — Foundation, Storage, Providers, Skills, Cron, Plugins
| Unit / Crate | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| `saif-platform` | `hermes_platform/`, `process_registry_termination.py` | done | Host facts, binary lookup, `(pid, start_time)`, parent-first teardown |
| `saif-config` | `hermes_constants.py`, `secret_scope.py`, `config*.py`, `skin_engine.py`, `i18n.py` | done | Task-local `ProfileScope`, fail-closed `SecretScope`, 3 loaders, migration |
| `saif-persistence` | `persistence/`, `hermes_state*.py` (v32 + FTS5 v3), `kanban_db.py` | done | Bundled SQLite + FTS5 + trigram + 7-table Kanban store |
| `saif-providers` | `providers/`, `anthropic_adapter.py`, `chat_completions.py`, `auxiliary_client.py` | done | 24 provider profiles, OpenAI + Anthropic + Codex + Bedrock, `CredentialPool` |
| `saif-skills` | `skill_utils.py`, `threat_patterns.py`, `skill_usage.py`, `curator*.py` | done | Frontmatter, 60-char clip, threat scan, Curator archive/tar.gz |
| `saif-cron` | `cron/jobs.py`, `cron/scheduler*.py`, `cron/occurrences.py` | done | Duration/natural/cron5/ISO parser, catch-up grace, `pending_slot`, tick lock |
| `saif-plugins` | `plugins/`, `plugin_catalog.py`, `plugin_host*.py` | done | Manifests, SHA-40 & `removed.yaml` gate, HookRegistry, host RPC client |

### Wave 2 — Tools, Agent Loop, Contracts, Surfaces, CLI, Conformance
| Unit / Crate | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| `saif-tools` | `toolsets.py`, `tools/registry.py`, `fuzzy_match.py`, `patch_parser.py`, core tools, `environments/` | done | Core toolset, BM25 `tool_search`, V4A/fuzzy patch, Local/Docker/SSH/Cloud, MCP, CDP, PTC, Media |
| `saif-core` | `run_agent.py`, `prompt_builder.py`, `context_compressor.py`, `conversation_loop.py`, `delegate_tool.py` | done | Byte-stable prompt cache, role alternation, `/steer`, 2-tier compressor, subagents, ShareGPT `batch` |
| `saif-rpc-contract` | `tui_gateway/contracts/` | done | WASM-safe shared JSON-RPC 2.0 & dashboard view contracts |
| `saif-tui` | `ui-tui/`, `tui_gateway/server.py` | done | Native `ratatui` + `crossterm` TUI + stdio `tui-gateway` |
| `saif-gateway` | `gateway/run.py`, `session_identity.py`, `api_server`, `webhook`, `telegram` | done | `RoutingIdentity`, dual busy guard, 4-invariant `StreamDraftController`, 30 platforms, multiplexer |
| `saif-web` + `apps/saif-web-ui` | `web/`, `hermes_cli/web_routers/`, `pty_bridge.py` | done | Axum REST/WS + `/api/pty` + multi-tab SPA + pure-Rust `wasm32-unknown-unknown` UI |
| `apps/saif-desktop` | `apps/desktop/` | done | Pure-Rust Tauri v2 desktop shell (`build.rs`, `ServeSidecarConfig`, SHA-256 updater check) |
| `saif-acp` | `acp_adapter/` | done | Native `saif-acp` stdio server |
| `saif-cli` | `cli.py`, `hermes_cli/main.py`, `subcommands/` | done | Unified `saif` binary + all subcommands (`auth`, `batch --resume`, `gateway`, etc.) |
| `saif-conformance` | `conformance/cases/*.jsonl` + invariant suite | done | 108 golden oracle cases across 9 suites + full Waves 1–6 E2E suite |

### Wave 3 — Full Provider & Transport Parity
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Codex Responses & App-Server | `agent/codex_responses_adapter.py`, `agent/transports/codex*.py` | done | `CodexResponsesTransport`, bounded `prompt_cache_key`, reserved tool aliasing |
| AWS Bedrock & Azure/Vertex OAuth | `agent/bedrock_adapter.py`, `azure_identity_adapter.py`, `plugins/model-providers/vertex` | done | Pure-Rust RFC 2104 SigV4 `BedrockConverseTransport`, fail-closed multiplex guard, `vertex`/`azure-openai` catalog profiles |
| Credential Pool Rotation & RL `batch_runner` | `agent/credential_pool.py`, `batch_runner.py` | done | `CredentialPool` (`FillFirst`/`RoundRobin`/`Random`/`LeastUsed`, cooldowns, terminal `Dead`) + parallel `--resume` ShareGPT `batch` |

### Wave 4 — Remote Sandboxes, Browser CDP, & Code Execution Kernel
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Cloud Sandboxes | `tools/environments/{modal,daytona,vercel_sandbox}.py` | done | `ModalEnv`, `DaytonaEnv` (10 GiB disk cap), `VercelSandboxEnv` (telemetry off, transient retries), `SnapshotLedger` |
| Native Chromium CDP Driver | `tools/browser_use_cli.py`, `browser_launcher.py`, `browser_vault_tool.py` | done | Pure-Rust `tokio-tungstenite` `CdpClient`, `CdpSupervisorRegistry`, origin-bound `BrowserVault`, `SecretRedactor` |
| Programmatic `execute_code` & `computer_use` | `tools/code_execution_tool.py`, `tools/computer_use_tool.py` | done | Loopback TCP RPC kernel (`saif_tools`/`hermes_tools`), 50 KB 40/60 truncation & `cache/exec` spill, desktop input guard |

### Wave 5 — Full 30 Gateway Platforms & Multi-Profile Multiplexing
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Chat & Collaboration Adapters | `plugins/platforms/*` (all 30 platforms) | done | `builtin_platform_specs()` (30 specs), `parse_inbound_webhook`, `chunk_for_platform`, `build_outbound_request` |
| Gateway Multiplexer & Service Units | `hermes_cli/gateway_migrate.py`, systemd/launchd/Windows units | done | `GatewayMultiplexer`, SHA-256 duplicate credential conflict check, `converge_multiplex_migration`, `render_service_unit` |

### Wave 6 — Complete WASM Dashboard, Tauri v2 Installer, TTS/STT & Media
| Unit / Feature | Hermes Modules Covered | Status | Notes |
|---|---|---|---|
| Pure-Rust WASM UI + Terminal Grid | `apps/saif-web-ui`, `crates/saif-web`, `crates/saif-rpc-contract` | done | `TerminalScreenBuffer` + `\x1b[RESIZE:<cols>;<rows>]`, multi-tab `DashboardUiState`, REST `/api/kanban`, `/api/cron`, `/api/skills`, `PUT /api/config` |
| Tauri v2 Desktop Bundling & Updater | `apps/saif-desktop/src-tauri` | done | `build.rs`, `ServeSidecarConfig`, detached `gateway run`, `verify_release_sha256` updater checksum check |
| Voice (TTS/STT) & Media Gen | `tools/tts_tool_providers.py`, `image_generation_tool.py`, `plugins/video_gen/` | done | `text_to_speech` (44-byte RIFF PCM-to-WAV, 16 MiB cap, xAI speed clamp), `speech_to_text`, `image_generate`, `video_generate` |

### Phases 1–7 — Full Runtime Wiring Parity & Permanent Upstream Sync Engine
| Phase / Unit | Scope Delivered | Status | Notes |
|---|---|---|---|
| Phase 1: Persistence & Hydration | `SqliteStore::compact_session_history`, `reconstruct_messages_from_records` | done | Resumes `--session` across restarts with byte-stable `system[0]` & durable compaction |
| Phase 2: Tool/Plugin/Config Wiring | `DelegateTaskTool`, `KanbanStore` ctx, `register_trusted_mcp_tools`, `PluginHookBridge`, `load_effective_saif_config` | done | `delegate_task` depth-gated, MCP + plugin hooks + `${VAR}` effective config active |
| Phase 3: Providers & Streaming | `36` profiles, live `CredentialPool`/`auth.json` failover, Anthropic SSE | done | Auto-retry 401/403/429 pool rotation + real-time `text_delta`/`thinking_delta`/`input_json_delta` |
| Phase 4: Cron & Multi-Profile Sync | Live `saif cron tick`, 30s gateway ticker, `profiles/<name>` migration, boundary guard | done | Executes due cron jobs with timeout + rejects file-at-home / tombstoned profile resurrection |
| Phase 5: Live Gateway Hardening | `/stop` cancel, `/steer`, `/new`, FIFO queue drain, progressive Telegram edits, outbound HTTP | done | Cancel-safe `tokio::select!` in `AgentRunner`, concurrent Telegram polling & draft edits |
| Phase 6: WASM UI & Live WS | Linear-memory FFI `saif_web_ui.wasm` (`224K`), CSP, 404 fallback, real-time WS events | done | `/assets/saif_web_ui.wasm` embedded & instantiated, `MessageDelta` streams before final RPC |
| Phase 7: Upstream Parity Engine | `conformance/UPSTREAM_LOCK.json`, `saif upstream status/diff/ack`, `redact_sensitive_text`, `docs/UPSTREAM_SYNC.md` | done | Ephemeral `--filter=blob:none` subsystem diff classifier + 12 new golden oracle cases |

## Verification & Benchmark Summary

Verified after final Phases 1–7 edits on `rustc 1.100.0` / `cargo 1.100.0`:

- **Formatting & Static Analysis**:
  - `cargo fmt --all -- --check`: passed (exit `0`).
  - `cargo clippy --workspace --all-targets -- -D warnings` + `apps/saif-web-ui`: passed across all workspace crates with `0` warnings.
  - Stub scan (`grep -rn --include="*.rs" "todo!\|unimplemented!" crates/ apps/`): `0` matches (`ZERO_STUBS`).
- **Automated Test Suites (`cargo test --workspace` + `saif-web-ui`)**: **39 passed, 0 failed, 0 ignored**:
  - `saif-platform` unit tests: **4 passed**.
  - `saif-conformance` `tests/oracle_replay.rs`: **10 passed** replaying **120 golden behavioral cases** across 10 `.jsonl` files (`cron_schedules.jsonl` [31], `patch_fuzzy.jsonl` [11], `skills_frontmatter.jsonl` [7], `prompt_injection_scan.jsonl` [7], `sqlite_fts_preview.jsonl` [5], `profile_paths.jsonl` [6], `codex_transport.jsonl` [14], `batch_sharegpt.jsonl` [9], `media_exec_helpers.jsonl` [18], `upstream_parity_guards.jsonl` [12]).
  - `saif-conformance` `tests/invariants_e2e.rs`: **7 passed**.
  - `saif-conformance` `tests/waves3_6_e2e.rs`: **6 passed**.
  - `saif-conformance` `tests/phases1_7_e2e.rs`: **7 passed** (SQLite session hydration & durable compaction, `delegate_task` + Kanban + trusted MCP + plugin hooks, `36` providers + live `CredentialPool` failover + Anthropic SSE, multi-profile Hermes migration & boundary guard, `/stop`/queue drain/Telegram draft streaming, WASM asset + CSP + 404 + real-time WS streaming, `UpstreamLock` classifier).
  - `saif-cli` `tests/cli_e2e.rs`: **4 passed** (`saif doctor` verifying `36 built-in profiles`, `cron tick` live execution, `upstream status/diff/ack`, `gateway`, `auth`, `mcp`, `kanban`, `profile`, `migrate`).
  - `apps/saif-web-ui` unit tests (`cargo test --manifest-path apps/saif-web-ui/Cargo.toml`): **1 passed** (`wasm_ui_terminal_buffer_and_tabs_work`).
- **WebAssembly & Desktop Targets**:
  - `cargo build --manifest-path apps/saif-web-ui/Cargo.toml --target wasm32-unknown-unknown --release`: passed (exit `0`), emitting `saif_web_ui.wasm` (`224K`, identical to embedded `crates/saif-web/assets/saif_web_ui.wasm`).
  - `apps/saif-desktop/src-tauri`: pure-Rust Tauri v2 shell (`build.rs`, `tauri.conf.json`, `src/main.rs`), excluded from headless Linux workspace build because `webkit2gtk` GUI libraries are not installed on this server.
- **Cold Startup Baseline (`--version`, measured on Wave 1–2 release binary)**:
  - `./target/release/saif --version` (5 runs, isolated temp `SAIF_HOME`): median `2.51 ms` vs `/home/ridlo/.local/bin/hermes --version` (2 read-only runs): median `575.83 ms` (`229.4x` faster).
