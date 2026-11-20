# `saif-agent` Session Handoff

## Current Status
- **Completed Waves**: Waves 1–6 COMPLETE (`saif-platform`, `saif-config`, `saif-persistence`, `saif-providers`, `saif-skills`, `saif-cron`, `saif-plugins`, `saif-tools`, `saif-core`, `saif-rpc-contract`, `saif-tui`, `saif-gateway`, `saif-web`, `saif-acp`, `saif-cli`, `saif-conformance`, `apps/saif-web-ui` WASM, and `apps/saif-desktop` Tauri v2).
- **Behavioral Oracle**: `108` golden cases across `9` `.jsonl` files extracted from `NousResearch/hermes-agent` into `conformance/cases/*.jsonl`.
- **Verification**: `29/29` automated tests passing (`28` workspace + `1` `saif-web-ui`), `clippy -- -D warnings` + `fmt` clean with zero warnings, `wasm32-unknown-unknown --release` built (`saif_web_ui.wasm` `401` bytes), release binaries `saif` (`13M`) & `saif-acp` (`10M`) built cleanly. See [MAP.md](MAP.md) for full details.

## Operational & Desktop Bundling Notes
1. **Desktop Installer Packaging**: Build `apps/saif-desktop` installers on macOS/Windows/Linux desktop runners with Tauri v2 OS prerequisites (`webkit2gtk` / WebView2) installed.
2. **Gateway Live Credentials**: Supply profile-scoped secrets under `$SAIF_HOME/.env` or `$SAIF_HOME/profiles/<name>/.env` before running `saif gateway run`; `GatewayMultiplexer::preflight_credential_conflicts` automatically prevents duplicate bot token reuse across distinct profiles.
3. **Migration from Hermes**: Run `saif migrate --from-hermes --dry-run` first to preview non-destructive import from `~/.hermes` into `~/.saif`.

## Key Documents to Read First
- [MAP.md](MAP.md)
- [DECISIONS.md](DECISIONS.md)
- [GLOSSARY.md](GLOSSARY.md)
- [../docs/HERMES_ANATOMY.md](../docs/HERMES_ANATOMY.md)
