# `saif-agent` Session Handoff

## Current Status
- **Completed Waves & Runtime Hardening**: Waves 1–6 + Phases 1–7 Runtime Parity & Upstream Sync COMPLETE (`saif-platform`, `saif-config`, `saif-persistence`, `saif-providers`, `saif-skills`, `saif-cron`, `saif-plugins`, `saif-tools`, `saif-core`, `saif-rpc-contract`, `saif-tui`, `saif-gateway`, `saif-web`, `saif-acp`, `saif-cli`, `saif-conformance`, `apps/saif-web-ui` WASM, and `apps/saif-desktop` Tauri v2).
- **Behavioral Oracle & Upstream Sync**: `120` golden cases across `10` `.jsonl` files under `conformance/cases/*.jsonl`, plus live upstream lockfile `conformance/UPSTREAM_LOCK.json` and `saif upstream` (`status`, `diff`, `ack`) synced to `NousResearch/hermes-agent` (`32dbc1dcda22`).
- **Verification**: `40/40` automated tests passing (`39` workspace + `1` `saif-web-ui`), `clippy -- -D warnings` + `fmt` clean with zero warnings, `ratatui 0.30.2` (`lru 0.18.5`, resolving Dependabot advisory), `wasm32-unknown-unknown --release` built (`224K` `saif_web_ui.wasm` embedded in `crates/saif-web/assets/`), GitHub Actions CI & weekly `upstream-watch` workflows under `.github/workflows/`, release binaries `saif` & `saif-acp` built & linked into `~/.local/bin/`, and `~/.saif` provisioned via non-destructive multi-profile `saif migrate --from-hermes`. See [MAP.md](MAP.md) for full details.

## Operational & Upstream Sync Notes
1. **Tracking Future `NousResearch/hermes-agent` Updates**: Run `saif upstream status`, then `saif upstream diff --port-only` to see net behavior changes grouped by Rust target crates, port + add tests, and run `saif upstream ack <sha>`. See [../docs/UPSTREAM_SYNC.md](../docs/UPSTREAM_SYNC.md).
2. **Desktop Installer Packaging**: Build `apps/saif-desktop` installers on macOS/Windows/Linux desktop runners with Tauri v2 OS prerequisites (`webkit2gtk` / WebView2) installed.
3. **Gateway Live Credentials**: Supply profile-scoped secrets under `$SAIF_HOME/.env` or `$SAIF_HOME/profiles/<name>/.env` before running `saif gateway run`; `GatewayMultiplexer::preflight_credential_conflicts` automatically prevents duplicate bot token reuse across distinct profiles.
4. **Migration from Hermes**: Run `saif migrate --from-hermes --dry-run` first to preview non-destructive multi-profile import from `~/.hermes` into `~/.saif`.

## Key Documents to Read First
- [MAP.md](MAP.md)
- [DECISIONS.md](DECISIONS.md)
- [GLOSSARY.md](GLOSSARY.md)
- [../docs/HERMES_ANATOMY.md](../docs/HERMES_ANATOMY.md)
- [../docs/UPSTREAM_SYNC.md](../docs/UPSTREAM_SYNC.md)
