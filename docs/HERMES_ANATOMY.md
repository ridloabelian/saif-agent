# Anatomi Lengkap `NousResearch/hermes-agent` (Hulu sampai Hilir) & Peta 1:1 ke `saif-agent`

Dokumen ini memetakan seluruh arsitektur [`NousResearch/hermes-agent`](https://github.com/NousResearch/hermes-agent) (pada snapshot `07f075c2`, 27.140 file, ~135 MB Python + ~44 MB TypeScript/TSX) dari titik masuk paling hulu hingga subsistem paling hilir, beserta desain padanan **100% Pure Rust** di dalam workspace **`saif-agent`**.

---

## Ringkasan Peta Subsistem (14 Lapisan End-to-End)

```
  [HULU: Entry Points & Config]
   1. Entry Points (CLI, REPL, -q, TUI, Serve/Dashboard, Gateway, ACP, Batch)
   2. Profile & Secret Scope (Multi-tenant isolation, fail-closed secrets, scrubbed env)
   3. Three-Loader Config, Skins, & Layered i18n
            │
            ▼
  [CORE ENGINE: Agent Turn & Context]
   4. Prompt Assembly & Sacred Prompt-Caching Invariant (system[0] byte-stable)
   5. 20-Phase Agent Turn Loop & Strict Role Alternation (/steer, tool hints)
   6. Context Compressor (Tool-output pruning -> Aux LLM summary -> deterministic fallback)
   7. Provider Profiles (36+ providers) & Wire Transports (OpenAI Chat / Codex / Anthropic / Bedrock)
            │
            ▼
  [STATE, TOOLS & LEARNING LOOP]
   8. Persistence Contracts + SQLite v32 (state.db, FTS5 v3, kanban.db)
   9. Tool Registry, 6 Execution Backends, V4A/Fuzzy Patch, CDP Browser, MCP, & Delegation
  10. Closed Learning Loop: Skills, Threat Scanner, Usage Telemetry, & Curator
  11. Plugin Ecosystem (General, Memory, Model, Platform, Context Engine, Host Isolation)
            │
            ▼
  [HILIR: Automation, Gateways & Surfaces]
  12. Cron Scheduler & Kanban Multi-Agent Work Queue
  13. Messaging Gateway (25+ Platforms, Two-Stage Guards, 4-Invariant Draft Streaming)
  14. 100% Rust UI Surfaces (ratatui TUI, Axum/WASM Dashboard + PTY, Tauri v2 Desktop) & ACP
```

---

## 1. Entry Points & Surface Routing
- **Upstream Hermes (`pyproject.toml`, `cli.py`, `hermes_cli/main.py`, `acp_adapter/entry.py`)**:
  - Tiga script entry utama: `hermes` (CLI/REPL/gateway/cron/dashboard/serve/doctor), `hermes-agent` (legacy CLI), dan `hermes-acp` (editor ACP stdio JSON-RPC).
  - `hermes_cli/main.py` menangani early-recovery (interrupted pull), normalisasi `HERMES_HOME`, parsing `-p <profile>` sebelum inisialisasi modul lain, arming startup watchdog untuk `gateway run`, dan one-shot exit cleanup (`_ONESHOT_CLEANUPS`).
- **Padanan `saif-agent` (`crates/saif-cli`, `crates/saif-acp`)**:
  - Dual native binary tanpa Python interpreter: `saif` (CLI, `-q` oneshot, `--tui`, `tui-gateway`, `gateway`, `cron`, `kanban`, `dashboard`, `serve`, `migrate --from-hermes`, `batch`) dan `saif-acp`.
  - Cold startup < 10 ms karena seluruh graph modul terkompilasi statis.

## 2. Profile Scope & Fail-Closed Secret Isolation (`HX001`–`HX012`)
- **Upstream Hermes (`hermes_constants.py`, `agent/secret_scope.py`, `tools/environments/local.py`)**:
  - `get_hermes_home()` membaca `ContextVar` override -> `HERMES_HOME` env -> default `~/.hermes`.
  - Satu proses (`gateway` / `serve`) melayani banyak profile (`~/.hermes/profiles/<name>/`).
  - `named_profile_has_identity` memeriksa 6 file penanda (`config.yaml`, `.env`, `SOUL.md`, `profile.yaml`, `auth.json`, `state.db`) serta menolak direktori kosong/tombstone (`.deleted/`).
  - `secret_scope.get_secret` **fail closed** saat `multiplex_active == True`: profile sekunder tidak pernah jatuh ke `os.environ` milik launch profile.
  - Setiap child spawn melewati `build_subprocess_env` / `served_profile_child_env` yang menyapu API key & token host kecuali yang diizinkan eksplisit.
- **Padanan `saif-agent` (`crates/saif-config`)**:
  - `ProfileScope` diikat via `tokio::task_local!` untuk setiap turn, tick cron, eviction, dan RPC.
  - `SecretScope` menyandikan mode `multiplex_active` langsung pada tipe runtime dan membangun `Command` environment via `SecretScope::build_child_env()`.

## 3. Three-Loader Config System, Skins, & Layered i18n
- **Upstream Hermes (`hermes_cli/config.py`, `config_defaults.py`, `config_effective.py`, `skin_engine.py`, `agent/i18n.py`)**:
  - Tiga loader terpisah dengan semantik sengaja berbeda:
    1. `load_cli_config()` (CLI defaults + YAML merge).
    2. `load_config()` (wizard/subcommand defaults + YAML merge).
    3. `load_user_config_effective()` (gateway/cron/TUI: user YAML + managed overlay `/etc/hermes/config.yaml` + ekspansi `${VAR}`, tanpa defaults untuk presence-sensitive checks).
  - Penulisan `config.yaml` selalu atomik dan menjaga komentar YAML (`atomic_config_write`). `.env` khusus secrets; `config.yaml` khusus perilaku.
  - `SkinConfig` pure-data (`default`, `ares`, `mono`, `slate`).
  - `i18n` berlapis: plugin packs -> user `<home>/locales/<lang>.yaml` -> bundled -> `en` -> bare key.
- **Padanan `saif-agent` (`crates/saif-config`)**:
  - Mengimplementasikan `load_cli_config`, `load_config`, dan `load_effective_config` (termasuk `${VAR}` interpolation & overlay), atomic write (temp + `sync_all` + rename), 4 built-in `SkinConfig`, `I18nCatalog`, dan `saif migrate --from-hermes`.

## 4. Prompt Assembly & Sacred Prompt-Caching Invariant
- **Upstream Hermes (`agent/prompt_builder.py`, `agent/skill_commands.py`, `agent/subdirectory_hints.py`)**:
  - **Invarian Suci #1 (Prompt Caching)**: `messages[0]` (`role="system"`) dan `tools[]` harus **byte-stable** sepanjang umur percakapan. Slash skill, `/steer`, dan hint direktori (`AGENTS.md`) tidak pernah menyentuh system prompt; semuanya masuk lewat pesan `user` atau lampiran hasil `tool` (head+tail cap 32.000 char).
  - `PromptBuilder` menggabungkan: `DEFAULT_AGENT_IDENTITY` (atau `$HOME/SOUL.md`), `MEMORY.md` & `USER.md`, `.hermes.md`/`HERMES.md`/`AGENTS.md` dari CWD naik hingga `.git` root (read timeout 5 detik untuk melindungi network FS), dan indeks skill ringkas (`<= 60` char per baris).
  - File konteks di-scan via `scan_for_threats(..., scope="context")`. File repo yang terdeteksi injeksi diblokir (`[BLOCKED: ...]`), sedangkan `SOUL.md` milik user hanya memberi warning (`user_authored=True`).
- **Padanan `saif-agent` (`crates/saif-core`)**:
  - `PromptBuilder` menyusun prompt identik, menghitung hash SHA-256 ke tabel `system_prompts`, dan `AgentRunner` menguji byte-stability `messages[0]` sepanjang turn.

## 5. Agent Turn Engine & Strict Role Alternation
- **Upstream Hermes (`run_agent.py`, `agent/conversation_loop.py`, `agent/turn_*.py`)**:
  - Loop iterasi terpecah menjadi ~20 modul fase (`turn_preflight`, `turn_iteration_prep`, `turn_request_assembly`, `turn_api_call`, `turn_api_error`, `turn_response_intake`, `turn_empty_response`, `turn_tool_round`, `turn_overflow`, `turn_truncation`, `turn_context_compaction`, `turn_recovery_autorecover`, `turn_stop_gates`, `turn_usage`, `turn_final_response`, `turn_finalizer`).
  - **Invarian Suci #2 (Strict Role Alternation)**: Tidak boleh ada dua pesan berurutan dengan role sama (kecuali urutan legal `assistant(tool_calls) -> tool... -> user` untuk `/steer`).
  - Mendukung budget iterasi (`max_iterations`) + 1 grace call agar agent bisa merangkum sebelum berhenti, interrupt via token/cancel, dan inline tool execution (`todo_list`, `memory`, `delegate_task`).
- **Padanan `saif-agent` (`crates/saif-core`)**:
  - `AgentRunner` memodelkan fase turn sebagai state machine asinkron berbasis `tokio` + `CancellationToken` + `StreamEvent` channel, dengan validator role-alternation sebelum setiap pemanggilan API.

## 6. Context Compressor (2-Tier Compaction)
- **Upstream Hermes (`agent/context_compressor.py`, `agent/conversation_compression.py`)**:
  - Hanya operasi ini yang diizinkan mematahkan KV cache prefix.
  - Tahap 1 (**Tool Output Pruning**): memangkas output tool lama tanpa panggilan LLM.
  - Tahap 2 (**Boundary Selection**): menjaga pasangan `assistant(tool_calls)` dan seluruh `tool` response terkait tidak terpotong di tengah.
  - Tahap 3 (**Summary Generation**): memanggil `auxiliary` model (dengan fallback chain & deterministic summary fallback bila stream stall berulang).
- **Padanan `saif-agent` (`crates/saif-core`)**:
  - `ContextCompressor` mengimplementasikan pruning tool-output + penentuan batas aman pasangan tool-call + ringkasan via `AuxiliaryClient` dan deterministic fallback summary.

## 7. Provider Profiles & Wire Transports
- **Upstream Hermes (`providers/base.py`, `plugins/model-providers/*`, `agent/transports/*`, `agent/auxiliary_client.py`)**:
  - `ProviderProfile` deklaratif untuk 36+ provider (OpenRouter, Nous Portal, OpenAI, Anthropic, Bedrock, Gemini, DeepSeek, Qwen, xAI, Groq, Fireworks, Ollama, LMStudio, Vertex, Kimi, Z.ai, dll).
  - Empat transport inti: OpenAI Chat Completions, Codex Responses, Anthropic Messages (injeksi otomatis `cache_control: ephemeral` & thinking blocks), dan AWS Bedrock.
  - `auxiliary_client.py` merutekan task sampingan (`compression`, `title_gen`, `session_search`, `curator`, `vision`) secara terpisah dari hook `pre/post_api_request` main loop.
- **Padanan `saif-agent` (`crates/saif-providers`)**:
  - `ProviderCatalog` + `LlmTransport` (`OpenAiChatTransport`, `AnthropicMessagesTransport`) + `AuxiliaryClient`, seluruhnya memakai `reqwest` dengan `rustls-tls`.

## 8. Persistence Contracts & SQLite v32 + FTS5 v3
- **Upstream Hermes (`persistence/`, `session_rules/`, `hermes_state*.py`)**:
  - Arsitektur kontrak repository netral backend (`persistence/core`, `conversation`, `gateway`, `workers`).
  - `state.db` Schema v32: `schema_version`, `system_prompts`, `sessions`, `messages`, `session_model_usage`, `runtime_epoch`, `session_admissions`, `worker_executions`, `worker_receipts`, `state_meta`, `gateway_routing`, `gateway_hygiene_state`, `conversation_generations`, `gateway_heartbeats`, `compression_locks`, `session_turn_leases`, `async_delegations`.
  - **FTS5 Storage v3**: `messages_fts` membatasi indeks `role='tool'` hanya 8.192 char pertama (`FTS_TOOL_CONTENT_PREFIX_CHARS`) plus `messages_fts_trigram`; query FTS5 disanitasi (max 2.048 char, token non-operator dikutip); `_shape_preview` merangkai preview 60 karakter satu baris.
- **Padanan `saif-agent` (`crates/saif-persistence`)**:
  - Trait `PersistenceStore` + `SqliteStore` dengan `rusqlite` `bundled` (FTS5 + trigram bawaan), migrasi idempoten v32, query sanitizer, dan `KanbanStore` (`kanban.db`).

## 9. Tool Registry, Backends, V4A/Fuzzy Patch, Browser, MCP & Delegation
- **Upstream Hermes (`model_tools.py`, `toolsets.py`, `tools/*.py`, `tools/environments/*`)**:
  - **Invarian Waist Sempit**: Tool terdaftar di `ToolRegistry` dengan `check_fn` yang di-cache per `hermes_home_key()`, diorganisir ke dalam `TOOLSETS` (`_HERMES_CORE_TOOLS`), dan dicari via BM25 Snowball stemming (`tool_search`).
  - **6 Execution Backends**: Local, Docker, SSH, Modal, Daytona, Vercel Sandbox.
  - **File Tools**: `fuzzy_match.py` (9 strategi progresif + guard escape-drift) & `patch_parser.py` (V4A `*** Begin Patch` + dry-run validation), dilindungi write-guard untuk `SOUL.md`/`config.yaml`/`.env`.
  - **Browser**: `browser_exec` / `browser_cdp` berbasis Chromium CDP + vault login + pencatatan PID Chromium.
  - **MCP Client**: stdio/HTTP JSON-RPC dengan trust gate per profile.
  - **Delegation**: `delegate_task` (`leaf` vs `orchestrator`, `max_spawn_depth=2`, `max_concurrent_children=3`, background async completion queue).
- **Padanan `saif-agent` (`crates/saif-tools`, `crates/saif-core`)**:
  - `ToolRegistry`, `ToolsetCatalog`, `fuzzy_match`, `patch_parser`, `ExecutionEnvironment` (`LocalEnv`, `DockerEnv`, `SshEnv`; Modal/Daytona/Vercel di Wave 4), `ProcessRegistry` (parent-first termination), `web_search`/`web_extract` (dengan guard SSRF), `McpClient` stdio, dan `delegate_task` terisolasi.

## 10. Closed Learning Loop: Skills, Threat Scanner, & Curator
- **Upstream Hermes (`skills/`, `optional-skills/`, `agent/skill_utils.py`, `tools/skill_manager_tool.py`, `tools/threat_patterns.py`, `agent/curator.py`)**:
  - `SKILL.md` YAML frontmatter + support dirs (`references/`, `templates/`, `assets/`, `scripts/`).
  - Hardline rule: `description <= 60` karakter (`SKILL_PROMPT_DESC_LIMIT`), dipotong di batas kata pada indeks prompt, sedangkan `skill_view` selalu mengembalikan isi penuh tanpa pagination.
  - `threat_patterns.py`: scan 3 scope kumulatif (`all` -> `context` -> `strict`) plus deteksi zero-width / bidirectional invisible Unicode.
  - `Curator`: melacak `.usage.json` (`use_count`, `view_count`, `patch_count`), membuat backup `.tar.gz`, memindahkan skill buatan agen yang basi ke `.archive/` (tidak pernah menghapus permanen, menghormati flag `pinned`).
- **Padanan `saif-agent` (`crates/saif-skills`)**:
  - Implementasi penuh parser frontmatter, validator 60-char, threat scanner (`scan_for_threats`), usage telemetry, dan `Curator`.

## 11. Plugin Ecosystem & Out-of-Process Host Isolation
- **Upstream Hermes (`plugins/`, `plugin-catalog/`, `hermes_cli/plugins*.py`, `hermes_cli/plugin_host*.py`)**:
  - 7 jenis plugin: General (`pre_tool_call`, `post_tool_call`, `pre_llm_call`, `post_llm_call`, `on_session_start`, `on_session_end`), Memory Provider, Model Provider, Context Engine, Image Gen, Video Gen, Cron Providers, Platform Adapters.
  - `plugin-catalog/*.yaml` wajib 40-hex commit SHA pin & dicek terhadap daftar blokir `removed.yaml`.
  - Mode `plugins.isolation: host` menjalankan plugin non-bundled di child process terpisah lewat protokol framed JSON-RPC.
- **Padanan `saif-agent` (`crates/saif-plugins`)**:
  - `PluginManager`, validator catalog SHA-40 + `removed.yaml`, `HookRegistry`, dan `PluginHostClient` untuk proses plugin terisolasi.

## 12. Cron Scheduler & Kanban Multi-Agent Board
- **Upstream Hermes (`cron/jobs.py`, `cron/scheduler.py`, `hermes_cli/kanban*.py`)**:
  - Format jadwal: duration (`30m`), natural phrase (`every monday 9am`, `weekdays at 9am`), 5-field cron, ISO one-shot, atau delay (`in 30m`).
  - Invarian ketahanan: `next_run_at` dimajukan **sebelum** dispatch (`pending_slot`), jendela catch-up `clamp(period/2, 120s, 7200s)` (+ 120s grace one-shot), `.tick.lock` per home, dan watchdog inaktivitas 600s.
  - Kanban (`kanban.db`): board isolation via `HERMES_KANBAN_BOARD`, compare-and-swap claim dengan `(worker_pid, worker_started_at)`, auto-block setelah `failure_limit` atau exit code `78` (`KANBAN_TERMINAL_PROVIDER_EXIT_CODE`).
- **Padanan `saif-agent` (`crates/saif-cron`, `crates/saif-persistence`)**:
  - Implementasi penuh parser jadwal, `compute_next_run`, `advance_before_dispatch`, tick lock, dan `KanbanStore`.

## 13. Messaging Gateway & 4-Invariant Draft Streaming
- **Upstream Hermes (`gateway/run.py`, `gateway/session_identity.py`, `gateway/platforms/*`)**:
  - ~25 platform adapter (Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Feishu, IRC, Mattermost, Google Chat, Webhook, OpenAI-compat `api_server`, dll).
  - Canonical `RoutingIdentity` diselesaikan pertama kali saat pesan masuk.
  - **Dual Message Guards**: adapter queue + runner intercept (`/stop`, `/new`, `/approve`, `/deny`, `/status`) agar perintah kontrol tidak ikut terantre saat agen sibuk.
  - **4-Invariant Draft Streaming**: frame prefix-stable, `finish(final_text)` eksplisit, `_interim_send`, dan rekonsiliasi via `edit_message` sebelum `send_message`.
- **Padanan `saif-agent` (`crates/saif-gateway`)**:
  - `GatewayRunner`, `RoutingIdentity`, `BusyMessageGuard`, `StreamDraftController` (teruji ke-4 invarian), adapter `api_server` (`/v1/chat/completions`), Webhook, dan Telegram Bot API (wave 5 melengkapi seluruh 25+ platform).

## 14. 100% Rust UI Surfaces (TUI, Web Dashboard, Desktop) & ACP
- **Upstream Hermes (`tui_gateway/`, `ui-tui/`, `web/`, `apps/desktop/`, `acp_adapter/`)**:
  - `tui_gateway` menjadi backend JSON-RPC 2.0 bersama (stdio untuk TUI, WebSocket untuk Web/Desktop) lengkap dengan `server_requests` (`srq-<n>`).
  - Dashboard web me-mount terminal xterm.js di atas WebSocket PTY (`/api/pty`) + sidebar status.
  - Desktop mengelola proses `serve` (mati bersama app) sekaligus menjaga `gateway run` tetap hidup terpisah (detached).
- **Padanan `saif-agent`**:
  - `crates/saif-rpc-contract`: Kontrak tipe JSON-RPC 2.0 tunggal.
  - `crates/saif-tui`: Native `ratatui` + `crossterm` TUI & mode `saif tui-gateway`.
  - `crates/saif-web` + `apps/saif-web-ui`: Server `axum` + `portable-pty` WS (`/api/pty`) + frontend WASM `wasm32-unknown-unknown`.
  - `apps/saif-desktop`: Shell desktop Tauri v2 murni Rust.
  - `crates/saif-acp`: Native `saif-acp` stdio server untuk editor (Zed, Neovim, JetBrains).
