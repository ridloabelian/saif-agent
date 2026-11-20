# Saif Agent

**Saif Agent (`saif-agent`)** is an independent **100% pure-Rust autonomous self-improving AI agent**, built for single-binary deployment with zero Python or Node runtime dependencies while remaining compatible with the Hermes agent state, skill, and multi-profile ecosystem.

## Quickstart

```bash
# Build native release binaries (`saif`, `saif-acp`)
cargo build --release --bins

# Verify environment & configuration
./target/release/saif doctor
./target/release/saif status

# Launch interactive TUI or Web/WASM dashboard
./target/release/saif --tui
./target/release/saif dashboard

# Preview non-destructive migration from an existing ~/.hermes installation
./target/release/saif migrate --from-hermes --dry-run
```

## Architecture Overview

- **Zero Python / Node Runtime**: Single statically compiled native binaries (`saif`, `saif-acp`) plus pure-Rust UI surfaces (`ratatui` TUI, `axum` + `portable-pty` + WASM Dashboard, Tauri v2 Desktop).
- **Behavioral Oracle Parity & Upstream Tracking**: Verified against `120` golden recordings across `10` `.jsonl` suites (`conformance/cases/*.jsonl`), tracked cleanly via `conformance/UPSTREAM_LOCK.json` and `saif upstream`.
- **State Home & Compatibility**: Uses `~/.saif` (`SAIF_HOME`) with isolated multi-profile islands (`~/.saif/profiles/<name>/`) and multi-profile `saif migrate --from-hermes`. Default persona (`DEFAULT_AGENT_IDENTITY`) stays neutral unless customized via `$SAIF_HOME/SOUL.md`.

## Workspace Crates (Waves 1–6 + Runtime Wiring & Upstream Sync Complete)

| Crate | Purpose |
|---|---|
| `saif-platform` | OS/arch facts, deterministic binary resolution, `(pid, start_time)` process identity, parent-first tree termination |
| `saif-config` | Task-local `ProfileScope` (boundary-guarded), fail-closed `SecretScope`, structured `redact_sensitive_text`, 3 YAML loaders (`load_effective_saif_config`), `SkinConfig`, i18n, multi-profile Hermes migration, `UpstreamLock` diff classifier |
| `saif-persistence` | `state.db` Schema v32 (`SqliteStore` + `compact_session_history`) + FTS5 v3 (8,192-char tool cutoff + trigram + sanitizer) + `kanban.db` (`KanbanStore`) |
| `saif-providers` | `36` built-in `ProviderProfile`s, OpenAI + Anthropic (SSE streaming + `cache_control`) + Codex `/responses` + pure-Rust AWS SigV4 Bedrock + live `CredentialPool`/`auth.json` failover |
| `saif-skills` | `SKILL.md` frontmatter parser, 60-char index rule, `scan_for_threats` injection scanner, `.usage.json`, `Curator` |
| `saif-cron` | Natural/cron5/ISO schedule parser, catch-up grace window, `advance_before_dispatch`, `.tick.lock`, live execution + 600s watchdog |
| `saif-plugins` | `plugin.yaml` loader, SHA-40 catalog & `removed.yaml` enforcer, `HookRegistry` + `PluginHookBridge`, stdio `PluginHostClient` |
| `saif-tools` | `ToolRegistry`, BM25 `tool_search`, `delegate_task`, 9-strategy `fuzzy_match`, V4A `patch_parser`, Local/Docker/SSH/Modal/Daytona/Vercel envs, WebSocket CDP + `BrowserVault` + redaction, `execute_code` loopback RPC kernel, trusted MCP tool registration, `computer_use`, TTS/STT/Image/Video media tools |
| `saif-core` | `PromptBuilder` (byte-stable prefix), `ContextCompressor`, `AgentRunner` (SQLite session hydration, durable compaction, cancel-safe `/stop` & `/steer`, `delegate_task`, `KanbanStore`), parallel ShareGPT `--resume` `batch` runner |
| `saif-rpc-contract` | Shared WASM-compatible JSON-RPC 2.0 & dashboard view types |
| `saif-tui` | Native `ratatui` + `crossterm` TUI (`saif --tui`) + stdio `tui-gateway` |
| `saif-gateway` | `RoutingIdentity`, dual busy guard (`/stop`/`/steer`/queue drain), 4-invariant progressive `StreamDraftController`, `30` platform specs, `GatewayMultiplexer` + outbound webhooks, systemd/launchd/Windows service units |
| `saif-web` | `axum` REST/WS (real-time streaming + `turn.cancel`, CSP, 404 fallback, `/assets/saif_web_ui.wasm`) + POSIX PTY bridge + multi-tab pure-Rust/WASM dashboard host |
| `saif-acp` | Native Agent Client Protocol stdio server (`saif-acp`) |
| `saif-cli` | Unified `saif` CLI & REPL binary (`auth`, `batch`, `gateway`, `profile`, `cron`, `kanban`, `migrate`, `dashboard`, `serve`, `upstream`) |
| `saif-conformance` | `120` golden oracle cases (`conformance/cases/*.jsonl`) + Waves 1–6 & Phases 1–7 runtime E2E suites |
| `apps/saif-web-ui` | Pure-Rust `wasm32-unknown-unknown` UI crate with linear-memory FFI (`saif_ui_feed_pty`, `saif_ui_apply_event`), `TerminalScreenBuffer` & `\x1b[RESIZE:<cols>;<rows>]` control frames |
| `apps/saif-desktop` | Pure-Rust Tauri v2 Desktop shell (`build.rs`, `ServeSidecarConfig`, detached `gateway run`, SHA-256 updater check) |

## Documentation

- [docs/HERMES_ANATOMY.md](docs/HERMES_ANATOMY.md) — Hulu-ke-Hilir technical dissection of all 14 `hermes-agent` subsystems
- [docs/UPSTREAM_SYNC.md](docs/UPSTREAM_SYNC.md) — 4-step workflow for tracking & porting future `NousResearch/hermes-agent` updates
- [rust/MAP.md](rust/MAP.md) — Complete Wave 1–6 & Phases 1–7 tracker & verification metrics
- [rust/DECISIONS.md](rust/DECISIONS.md) — Architecture Decision Records (`ADR-001`..`ADR-011`)
- [rust/GLOSSARY.md](rust/GLOSSARY.md) — 1:1 symbol glossary (`hermes-agent` -> `saif-agent`)

## Acknowledgments

Saif Agent draws upon the foundational agent architecture, SQLite/FTS5 state schema, and behavioral specifications of [`NousResearch/hermes-agent`](https://github.com/NousResearch/hermes-agent) (MIT License). Saif Agent is an independent pure-Rust implementation and is not affiliated with, endorsed by, or sponsored by Nous Research.

## License

Released under the [MIT License](LICENSE). See [NOTICE](NOTICE) for upstream attribution details and [SECURITY.md](SECURITY.md) for responsible vulnerability reporting.
