# `saif-agent`

**100% Pure-Rust autonomous self-improving AI agent**, built from scratch for 1:1 architectural and behavioral parity with [`NousResearch/hermes-agent`](https://github.com/NousResearch/hermes-agent).

## Architecture Overview

- **Zero Python / Node Runtime**: Single statically compiled native binaries (`saif`, `saif-acp`) plus pure-Rust UI surfaces (`ratatui` TUI, `axum` + `portable-pty` + WASM Dashboard, Tauri v2 Desktop).
- **Behavioral Oracle Parity (`rust-anything` + Golden `.jsonl` Replay)**: Verified against `108` golden recordings across `9` `.jsonl` suites extracted directly from `NousResearch/hermes-agent` (`conformance/cases/*.jsonl`).
- **State Home & Compatibility**: Uses `~/.saif` (`SAIF_HOME`) with isolated multi-profile islands (`~/.saif/profiles/<name>/`) and `saif migrate --from-hermes`. Default persona (`DEFAULT_AGENT_IDENTITY`) stays neutral upstream Hermes unless customized via `$SAIF_HOME/SOUL.md`.

## Workspace Crates (Waves 1–6 Complete)

| Crate | Purpose |
|---|---|
| `saif-platform` | OS/arch facts, deterministic binary resolution, `(pid, start_time)` process identity, parent-first tree termination |
| `saif-config` | Task-local `ProfileScope`, fail-closed `SecretScope`, 3 YAML loaders, atomic config writes, `SkinConfig`, i18n, Hermes migration |
| `saif-persistence` | `state.db` Schema v32 (`SqliteStore`) + FTS5 v3 (8,192-char tool cutoff + trigram + sanitizer) + `kanban.db` (`KanbanStore`) |
| `saif-providers` | `24` built-in `ProviderProfile`s, OpenAI + Anthropic (`cache_control`) + Codex `/responses` + pure-Rust AWS SigV4 Bedrock + `CredentialPool` |
| `saif-skills` | `SKILL.md` frontmatter parser, 60-char index rule, `scan_for_threats` injection scanner, `.usage.json`, `Curator` |
| `saif-cron` | Natural/cron5/ISO schedule parser, catch-up grace window, `advance_before_dispatch`, `.tick.lock`, 600s watchdog |
| `saif-plugins` | `plugin.yaml` loader, SHA-40 catalog & `removed.yaml` enforcer, `HookRegistry`, stdio `PluginHostClient` |
| `saif-tools` | `ToolRegistry`, BM25 `tool_search`, 9-strategy `fuzzy_match`, V4A `patch_parser`, Local/Docker/SSH/Modal/Daytona/Vercel envs, WebSocket CDP + `BrowserVault`, `execute_code` loopback RPC kernel, `computer_use`, TTS/STT/Image/Video media tools |
| `saif-core` | `PromptBuilder` (byte-stable prefix), `ContextCompressor`, `AgentRunner` (strict role alternation, `/steer`, `delegate_task`), parallel ShareGPT `--resume` `batch` runner |
| `saif-rpc-contract` | Shared WASM-compatible JSON-RPC 2.0 & dashboard view types |
| `saif-tui` | Native `ratatui` + `crossterm` TUI (`saif --tui`) + stdio `tui-gateway` |
| `saif-gateway` | `RoutingIdentity`, dual busy guard, 4-invariant `StreamDraftController`, `30` platform specs, `GatewayMultiplexer`, systemd/launchd/Windows service units |
| `saif-web` | `axum` REST/WS (`/api/kanban`, `/api/cron`, `/api/skills`, `/api/config`, `/ws`, `/api/pty`) + multi-tab pure-Rust/WASM dashboard host |
| `saif-acp` | Native Agent Client Protocol stdio server (`saif-acp`) |
| `saif-cli` | Unified `saif` CLI & REPL binary (`auth`, `batch`, `gateway`, `profile`, `cron`, `kanban`, `migrate`, `dashboard`, `serve`) |
| `saif-conformance` | `108` golden oracle cases (`conformance/cases/*.jsonl`) + Waves 1–6 invariant & E2E suites |
| `apps/saif-web-ui` | Pure-Rust `wasm32-unknown-unknown` UI crate with `TerminalScreenBuffer` & `\x1b[RESIZE:<cols>;<rows>]` control frames |
| `apps/saif-desktop` | Pure-Rust Tauri v2 Desktop shell (`build.rs`, `ServeSidecarConfig`, detached `gateway run`, SHA-256 updater check) |

## Documentation

- [docs/HERMES_ANATOMY.md](docs/HERMES_ANATOMY.md) — Hulu-ke-Hilir technical dissection of all 14 `hermes-agent` subsystems
- [rust/MAP.md](rust/MAP.md) — Complete Wave 1–6 tracker & verification metrics
- [rust/DECISIONS.md](rust/DECISIONS.md) — Architecture Decision Records (`ADR-001`..`ADR-010`)
- [rust/GLOSSARY.md](rust/GLOSSARY.md) — 1:1 symbol glossary (`hermes-agent` -> `saif-agent`)
