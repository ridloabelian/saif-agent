# `saif-agent` Session Handoff

## Current Status
- **Completed Waves**: Foundation + Wave 1 + Wave 2 (`saif-platform`, `saif-config`, `saif-persistence`, `saif-providers`, `saif-skills`, `saif-cron`, `saif-plugins`, `saif-tools`, `saif-core`, `saif-rpc-contract`, `saif-tui`, `saif-gateway`, `saif-web`, `saif-acp`, `saif-cli`, `saif-conformance`, plus pure-Rust `apps/saif-web-ui` WASM and `apps/saif-desktop` Tauri v2).
- **Behavioral Oracle**: 67 golden cases extracted from `NousResearch/hermes-agent` into `conformance/cases/*.jsonl`.
- **Verification**: 19/19 workspace tests passing, `clippy -- -D warnings` + `fmt` clean, `wasm32-unknown-unknown` clean, release binaries `saif` (`12M`) & `saif-acp` (`9.3M`) built and verified. See [MAP.md](MAP.md) for full details.

## Next Steps (Waves 3–6)
1. **Wave 3**: Add `CodexResponsesTransport` (`agent/codex_responses_adapter.py`), AWS SigV4 Bedrock transport, OAuth credential rotation pools, and parallel RL `batch_runner` ShareGPT exporter in `saif-providers` & `saif-cli`.
2. **Wave 4**: Implement `ModalEnv`, `DaytonaEnv`, `VercelSandboxEnv`, direct Chromium WebSocket CDP driver (`browser_exec` / `browser_vault`), programmatic `execute_code`, and `computer_use` in `saif-tools`.
3. **Wave 5**: Expand `saif-gateway` adapters (Discord, Slack, WhatsApp, Signal, Matrix, Email, Feishu, IRC, Mattermost, Google Chat, etc.) using `StreamDraftController` + `RoutingIdentity`, plus single-process profile multiplexer.
4. **Wave 6**: Build interactive Leptos/Yew browser dashboard UI inside `apps/saif-web-ui`, bundle Tauri v2 installers in `apps/saif-desktop`, and implement TTS/STT & image/video media providers.

## Key Documents to Read First
- [MAP.md](MAP.md)
- [DECISIONS.md](DECISIONS.md)
- [GLOSSARY.md](GLOSSARY.md)
- [../docs/HERMES_ANATOMY.md](../docs/HERMES_ANATOMY.md)
