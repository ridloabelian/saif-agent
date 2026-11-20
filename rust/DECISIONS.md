# Architecture Decision Records — `saif-agent`

These ADRs govern the clean-room Rust implementation of `saif-agent`, targeting 1:1 architectural and behavioral parity with `NousResearch/hermes-agent`.

## ADR-001: Pure-Rust Workspace + `rust-anything` Behavioral Oracle
Status: accepted (2026-11-20)
Context: `NousResearch/hermes-agent` is ~135 MB of Python (~12,732 `.py` files) plus ~44 MB of TypeScript/TSX (`ui-tui`, `web`, `apps/desktop`). A line-by-line runtime bridge (PyO3) retains a Python interpreter and heavy venv dependency.
Decision: Build `saif-agent` from zero as a 100% pure-Rust Cargo workspace, while using `erkamyaman/rust-anything` (`analyze`, golden `.jsonl` oracle recordings under `conformance/cases/`, replay tests, and strict `gate`-style clippy/error-handling rules) as the behavioral truth source.
Consequences: Zero runtime dependency on Python, `uv`, or Node.js; sub-10ms cold CLI startup; deterministic replay verification against `hermes-agent`.

## ADR-002: 100% Rust Across Every User-Facing Surface
Status: accepted (2026-11-20)
Context: Upstream Hermes splits UI across Node/Ink (`ui-tui`), React/Vite (`web`), and Electron (`apps/desktop`), talking over `tui_gateway` JSON-RPC 2.0.
Decision: Implement every UI surface in pure Rust while preserving the exact `tui_gateway` wire contract in `saif-rpc-contract`:
- Terminal TUI: native `ratatui` + `crossterm` inside `saif` (`crates/saif-tui`).
- Web Dashboard: `axum` HTTP/WS + `portable-pty` PTY bridge (`/api/pty`) in `crates/saif-web`, paired with a `wasm32-unknown-unknown` frontend (`apps/saif-web-ui`).
- Desktop App: Tauri v2 (`apps/saif-desktop`) managing `saif serve` while keeping `saif gateway run` detached.
Consequences: Single unified Rust toolchain across CLI, TUI, Gateway, Web, Desktop, and ACP. `apps/saif-desktop` and `apps/saif-web-ui` stay in `workspace.exclude` so headless Linux CI tests the core workspace without GUI system packages (`webkit2gtk`).

## ADR-003: Identity (`saif`), State Home (`~/.saif`), and Neutral Upstream Persona
Status: accepted (2026-11-20)
Context: Users may run `saif-agent` alongside an existing `~/.hermes` installation or migrate cleanly.
Decision: Ship binaries `saif` and `saif-acp`. Default home directory is `~/.saif` (override via `SAIF_HOME`), supporting multi-profile islands at `~/.saif/profiles/<name>/` and non-destructive `saif migrate --from-hermes`. Default persona (`DEFAULT_AGENT_IDENTITY`) and base behavior match upstream Hermes neutrally, loading optional `$SAIF_HOME/SOUL.md` and `locales/<lang>.yaml`.
Consequences: Zero risk of corrupting an operator's live `~/.hermes` state while supporting 100% SQLite/YAML/skill migration.

## ADR-004: Task-Local `ProfileScope` and Fail-Closed `SecretScope`
Status: accepted (2026-11-20)
Context: Hermes runs a single gateway/serve process hosting multiple tenant profiles (`HX002`/`HX004`/`HX005`). Reading `std::env::var` for secrets or config causes cross-profile credential leakage.
Decision: Model profile scoping explicitly via `tokio::task_local!` (`ProfileScope` in `saif-config`). When `multiplex_active == true`, secondary profiles read secrets strictly from their profile `.env` snapshot and fail closed (never falling back to process `std::env`). Every child process spawns via `served_profile_child_env`, scrubbing parent API keys and tokens unless explicitly passthrough-listed.
Consequences: Multi-profile isolation is enforced by the Rust type system rather than runtime monkeypatching.

## ADR-005: Zero-OpenSSL Networking (`rustls`) & Bundled SQLite FTS5
Status: accepted (2026-11-20)
Context: Upstream Python relies on `sqlite3` (with FTS5 + trigram), `ruamel.yaml`, `httpx`, and `openssl`. Headless servers and static release builds should not fail on missing `libssl-dev` or `libsqlite3-dev`.
Decision: Use `rusqlite` with `features = ["bundled"]` (which embeds SQLite with `FTS5` and `trigram` enabled) and use `rustls` exclusively across `reqwest` and `tokio-tungstenite` (`default-features = false`).
Consequences: Compiles anywhere with a basic C compiler (`gcc`/`cc`) for SQLite, producing portable binaries free of OpenSSL runtime mismatches.

## ADR-006: Sacred Prompt-Caching & Strict Role-Alternation Invariants
Status: accepted (2026-11-20)
Context: Breaking the prefix of `messages[0]` or mutating `tools[]` mid-conversation invalidates provider KV prompt caches (Anthropic/OpenAI/OpenRouter) and multiplies API cost.
Decision: `PromptBuilder` constructs `messages[0]` once at session creation and stores its SHA-256 hash in `system_prompts`. Mid-turn injections (`/skill` slash commands, `/steer`, directory `AGENTS.md` hints) travel strictly in `user` messages or appended `tool` outputs. Only `ContextCompressor` (explicit `/compress` or context-window threshold) is permitted to break the prefix.
Consequences: Provider KV cache hit rates remain optimal across long multi-tool turns.

## ADR-007: `state.db` Schema v32 & FTS5 Storage v3 Parity
Status: accepted (2026-11-20)
Context: Hermes stores durable sessions, messages, lineage, admissions, worker executions, token usage, and FTS5 indices in SQLite (`state.db` v32 + `kanban.db`).
Decision: Implement the exact v32 table definitions, indices, triggers, 8,192-char tool-output FTS5 truncation view/projection, 2,048-char FTS5 query sanitizer, and 60-char session preview shaper (`_shape_preview`) in `saif-persistence`.
Consequences: Databases created by `saif` and migrated from `hermes` share identical schema guarantees and search semantics.

## ADR-008: Four-Invariant Streaming Delivery in Messaging Gateways
Status: accepted (2026-11-20)
Context: Messaging platforms with editable native draft streams (Slack, Telegram, Discord) suffer duplicate messages if drafts mutate backwards or interim updates seal the stream.
Decision: Implement `StreamDraftController` in `saif-gateway` enforcing Hermes's 4 invariants: (1) prefix-stable frames (`frame[n]` starts with `frame[n-1]`), (2) explicit final seal via `finish(final_text)`, (3) `_interim_send` metadata bypass, and (4) reconcile via `edit_message` before falling back to `send_message`.
Consequences: Prevents duplicate/garbled streaming replies across messaging platforms.

## ADR-009: Error Handling Discipline (`rust-anything gate` Parity)
Status: accepted (2026-11-20)
Context: `rust-anything gate` rejects `unwrap()`, `expect()`, `panic!()`, `todo!()`, and `unimplemented!()` on fallible paths.
Decision: All workspace crates forbid `todo!()` / `unimplemented!()` and propagate typed `thiserror` / `anyhow` errors on every I/O, parsing, network, and DB boundary (`clippy -D warnings` enforced).
Consequences: Robust resilience against malformed YAML, corrupt SQLite cells, network timeouts, and adversarial tool arguments.

## ADR-010: Phased Wave Delivery & Conformance Gates
Status: accepted (2026-11-20)
Context: Replicating 27k files of Hermes requires disciplined, verifiable waves.
Decision: Execute in 6 waves tracked in `rust/MAP.md`:
- Wave 1–2 (Foundation + Core Runtime + Native TUI + Core Gateway/Web/ACP/CLI): Delivered & verified immediately with golden oracle replay.
- Wave 3–6 (Full 36+ Provider Quirks, Docker/Modal/Daytona/Vercel Sandboxes, CDP Browser Harness, Full 25+ Gateway Platforms, Leptos/Yew Dashboard UI + Tauri v2 Bundling, RL/Atropos Batch Runner): Tracked with explicit contracts in `rust/MAP.md` and `rust/HANDOFF.md`.
Consequences: Immediate runnable, tested `saif` binary in Wave 1–2 with clear zero-ambiguity expansion paths for Waves 3–6.

## ADR-011: Net-Tree Subsystem Upstream Sync & Lockfile (`UPSTREAM_LOCK.json` + `saif upstream`)
Status: accepted (2026-11-20)
Context: `NousResearch/hermes-agent` advances by thousands of commits per month across `plugin-catalog/` bumps, Python/Node packaging files, and substantive runtime/security fixes (`agent/redact_*.py`, `fix(profiles)`, `gateway/`, `web/`). Reviewing raw commit logs manually does not scale, and cloning full blobs wastes host disk.
Decision: Track upstream parity declaratively via `conformance/UPSTREAM_LOCK.json` and a native `saif upstream` CLI (`status`, `diff`, `ack`) using ephemeral `--bare --filter=blob:none` temporary fetches to compute net diffs per subsystem (`PORT_REQUIRED` vs `SKIP_RUNTIME_SPECIFIC` vs `IGNORED_NOISE` vs `UNMAPPED`).
Consequences: Ongoing upstream parity tracking requires a single `saif upstream diff --port-only` check, keeps `~/.hermes` untouched, leaves zero cached git bloat on disk, and ties every upstream behavior change directly to target Rust files and golden `.jsonl` suites.
