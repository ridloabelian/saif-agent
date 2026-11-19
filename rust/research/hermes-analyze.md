# Rust migration plan: tmp.iRJE9hpaWn (Python)

1 packages, 87 source files (46465 lines), 0 test files, 0 UI files, grouped into 12 units.

| Ready | Caution | Blocked | Keep in source language |
| ---: | ---: | ---: | ---: |
| 2 | 7 | 3 | 0 |

## Start here

Best first ports: safe to port (readiness) and worth porting (loops and string work, tests, size).

| # | Unit | Lines | Priority | Readiness | Tests | Bridge | Main risks |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | `hermes_state_fts.py` | 517 | 48 | 62 | 0 | PyO3 | String offsets and slicing (4); Dynamic typing (4); Exceptions (10) |
| 2 | `toolsets.py` | 432 | 45 | 51 | 0 | PyO3 | Hash iteration order (24); String offsets and slicing (2); Dynamic typing (6) |
| 3 | `hermes_state_search.py` | 1412 | 20 | 9 | 0 | PyO3 | Hash iteration order (8); String offsets and slicing (6); Regexes built at runtime (1) |
| 4 | `agent/` | 9097 | 14 | 0 | 0 | PyO3 | String offsets and slicing (90); Hash iteration order (74); Regexes built at runtime (7) |
| 5 | `hermes_state_common.py` | 1321 | 13 | 12 | 0 | PyO3 | Hash iteration order (6); String offsets and slicing (2); Dynamic typing (24) |
| 6 | `acp_adapter/` | 5669 | 13 | 0 | 0 | PyO3 | Hash iteration order (53); String offsets and slicing (47); Async and promises (570) |
| 7 | `hermes_constants.py` | 1188 | 12 | 0 | 0 | PyO3 | Hash iteration order (16); String offsets and slicing (4); Dynamic typing (22) |
| 8 | `cron/` | 16622 | 9 | 0 | 0 | PyO3 | Hash iteration order (153); String offsets and slicing (58); Regexes built at runtime (11) |
| 9 | `hermes_state.py` | 1592 | 7 | 0 | 0 | PyO3 | Hash iteration order (4); String offsets and slicing (1); Dynamic typing (37) |

### `hermes_state_fts.py`

517 lines in 1 files, wave 4, priority 48, readiness 62, bridge **PyO3**.

- 1 PyPI dependencies not in the crate table: check them by hand.
- No tests found: write characterization tests against the original code first.
- String offsets and slicing x4: `hermes_state_fts.py:145`, `hermes_state_fts.py:146`, `hermes_state_fts.py:147`
- Dynamic typing x4: `hermes_state_fts.py:327`, `hermes_state_fts.py:352`, `hermes_state_fts.py:516`
- Exceptions x10: `hermes_state_fts.py:119`, `hermes_state_fts.py:121`, `hermes_state_fts.py:358`
- Number semantics x2: `hermes_state_fts.py:464`, `hermes_state_fts.py:464`

### `toolsets.py`

432 lines in 1 files, wave 1, priority 45, readiness 51, bridge **PyO3**.

- 4 imports use aliases or virtual modules and were not followed.
- Import cycle with tools/: port them together.
- No tests found: write characterization tests against the original code first.
- Hash iteration order x24: `toolsets.py:68`, `toolsets.py:68`, `toolsets.py:144`
- String offsets and slicing x2: `toolsets.py:168`, `toolsets.py:348`
- Dynamic typing x6: `toolsets.py:47`, `toolsets.py:52`, `toolsets.py:261`
- Exceptions x4: `toolsets.py:254`, `toolsets.py:263`, `toolsets.py:349`
- Number semantics x4: `toolsets.py:296`, `toolsets.py:439`, `toolsets.py:446`

### `hermes_state_search.py`

1412 lines in 1 files, wave 5, priority 20, readiness 9, bridge **PyO3**.

- 1 PyPI dependencies not in the crate table: check them by hand.
- 1 imports use aliases or virtual modules and were not followed.
- No tests found: write characterization tests against the original code first.
- Hash iteration order x8: `hermes_state_search.py:136`, `hermes_state_search.py:136`, `hermes_state_search.py:137`
- String offsets and slicing x6: `hermes_state_search.py:1005`, `hermes_state_search.py:1017`, `hermes_state_search.py:1231`
- Regexes built at runtime x1: `hermes_state_search.py:134`
- Dynamic typing x27: `hermes_state_search.py:176`, `hermes_state_search.py:181`, `hermes_state_search.py:231`
- Division and modulo x2: `hermes_state_search.py:45`, `hermes_state_search.py:64`
- Concurrency x2: `hermes_state_search.py:110`, `hermes_state_search.py:800`
- Exceptions x26: `hermes_state_search.py:125`, `hermes_state_search.py:239`, `hermes_state_search.py:241`
- Number semantics x4: `hermes_state_search.py:137`, `hermes_state_search.py:1259`, `hermes_state_search.py:1282`

### `agent/`

9097 lines in 4 files, wave 2, priority 14, readiness 0, bridge **PyO3**.

- 8 PyPI dependencies not in the crate table: check them by hand.
- 120 imports use aliases or virtual modules and were not followed.
- No tests found: write characterization tests against the original code first.
- String offsets and slicing x90: `agent/context_compressor.py:197`, `agent/context_compressor.py:545`, `agent/context_compressor.py:557`
- Hash iteration order x74: `agent/context_compressor.py:240`, `agent/context_compressor.py:245`, `agent/context_compressor.py:500`
- Regexes built at runtime x7: `agent/context_compressor.py:176`, `agent/context_compressor.py:870`, `agent/context_compressor.py:871`
- Regex features the `regex` crate lacks x2: `agent/context_compressor.py:1162`, `agent/skill_utils.py:804`
- Dynamic typing x435: `agent/context_compressor.py:86`, `agent/context_compressor.py:111`, `agent/context_compressor.py:151`
- Generators x6: `agent/context_compressor.py:88`, `agent/context_compressor.py:88`, `agent/skill_utils.py:661`
- Division and modulo x6: `agent/context_compressor.py:1300`, `agent/context_compressor.py:1305`, `agent/context_compressor.py:1315`
- Class inheritance x2: `agent/context_compressor.py:391`, `agent/context_compressor.py:2149`
- Concurrency x1: `agent/prompt_builder.py:73`
- Exceptions x113: `agent/context_compressor.py:87`, `agent/context_compressor.py:150`, `agent/context_compressor.py:226`
- Number semantics x39: `agent/context_compressor.py:1986`, `agent/context_compressor.py:2674`, `agent/context_compressor.py:2685`

### `hermes_state_common.py`

1321 lines in 1 files, wave 3, priority 13, readiness 12, bridge **PyO3**.

- 3 PyPI dependencies not in the crate table: check them by hand.
- 1 imports use aliases or virtual modules and were not followed.
- No tests found: write characterization tests against the original code first.
- Hash iteration order x6: `hermes_state_common.py:184`, `hermes_state_common.py:184`, `hermes_state_common.py:201`
- String offsets and slicing x2: `hermes_state_common.py:157`, `hermes_state_common.py:408`
- Dynamic typing x24: `hermes_state_common.py:205`, `hermes_state_common.py:211`, `hermes_state_common.py:338`
- Generators x8: `hermes_state_common.py:408`, `hermes_state_common.py:408`, `hermes_state_common.py:1439`
- Exceptions x17: `hermes_state_common.py:47`, `hermes_state_common.py:341`, `hermes_state_common.py:424`
- Number semantics x3: `hermes_state_common.py:184`, `hermes_state_common.py:201`, `hermes_state_common.py:1355`

## Order by package

Packages come after the workspace packages they use. Inside a package, port by wave: a unit's imports are in earlier waves.

### 1. hermes-agent (`.`, portable, 0 test files)

| Wave | Unit | Status | Lines | Readiness | Bridge | Main risks |
| ---: | --- | --- | ---: | ---: | --- | --- |
| 0 | `hermes_constants.py` | caution | 1188 | 0 | PyO3 | Hash iteration order (16); String offsets and slicing (4) |
| 1 | `toolsets.py` | ready | 432 | 51 | PyO3 | Hash iteration order (24); String offsets and slicing (2) |
| 1 | `providers/` | blocked | 1364 | 23 | PyO3 | Hash iteration order (4); String offsets and slicing (1) |
| 1 | `tools/` | blocked | 2481 | 0 | PyO3 | String offsets and slicing (43); Hash iteration order (20) |
| 2 | `agent/` | caution | 9097 | 0 | PyO3 | String offsets and slicing (90); Hash iteration order (74) |
| 3 | `hermes_state_common.py` | caution | 1321 | 12 | PyO3 | Hash iteration order (6); String offsets and slicing (2) |
| 3 | `cron/` | caution | 16622 | 0 | PyO3 | Hash iteration order (153); String offsets and slicing (58) |
| 4 | `hermes_state_fts.py` | ready | 517 | 62 | PyO3 | String offsets and slicing (4); Dynamic typing (4) |
| 5 | `hermes_state_search.py` | caution | 1412 | 9 | PyO3 | Hash iteration order (8); String offsets and slicing (6) |
| 6 | `hermes_state.py` | caution | 1592 | 0 | PyO3 | Hash iteration order (4); String offsets and slicing (1) |
| 7 | `acp_adapter/` | caution | 5669 | 0 | PyO3 | Hash iteration order (53); String offsets and slicing (47) |
| 7 | `gateway/` | blocked | 4770 | 0 | PyO3 | Hash iteration order (47); String offsets and slicing (22) |

## PyPI dependencies

| Package | Units | Rust | Fit | Note |
| --- | ---: | --- | --- | --- |
| `hermes-cli` | 9 | ? | unknown |  |
| `hermes-state-errors` | 4 | ? | unknown |  |
| `hermes-state-registry` | 4 | ? | unknown |  |
| `pm` | 4 | ? | unknown |  |
| `run-agent` | 4 | ? | unknown |  |
| `tui-gateway` | 4 | ? | unknown |  |
| `utils` | 4 | ? | unknown |  |
| `hermes-bootstrap` | 3 | ? | unknown |  |
| `hermes-dir-acl` | 3 | ? | unknown |  |
| `hermes-logging` | 3 | ? | unknown |  |
| `hermes-time` | 3 | ? | unknown |  |
| `model-tools` | 3 | ? | unknown |  |
| `plugins` | 3 | ? | unknown |  |
| `hermes-platform` | 2 | ? | unknown |  |
| `hermes-state-pidns` | 2 | ? | unknown |  |
| `acp` | 1 | ? | unknown |  |
| `croniter` | 1 | ? | unknown |  |
| `hermes-constants-scratch` | 1 | ? | unknown |  |
| `hermes-constants-scratch-detach` | 1 | ? | unknown |  |
| `hermes-startup-watchdog` | 1 | ? | unknown |  |
| `hermes-state-api-content` | 1 | ? | unknown |  |
| `hermes-state-compression` | 1 | ? | unknown |  |
| `hermes-state-coverage` | 1 | ? | unknown |  |
| `hermes-state-dbfile` | 1 | ? | unknown |  |
| `hermes-state-gateway` | 1 | ? | unknown |  |
| `hermes-state-guard` | 1 | ? | unknown |  |
| `hermes-state-health` | 1 | ? | unknown |  |
| `hermes-state-holders` | 1 | ? | unknown |  |
| `hermes-state-keys` | 1 | ? | unknown |  |
| `hermes-state-local` | 1 | ? | unknown |  |
| `hermes-state-lockguard` | 1 | ? | unknown |  |
| `hermes-state-lockowners` | 1 | ? | unknown |  |
| `hermes-state-maintenance` | 1 | ? | unknown |  |
| `hermes-state-messages` | 1 | ? | unknown |  |
| `hermes-state-portability` | 1 | ? | unknown |  |
| `hermes-state-profile-repair` | 1 | ? | unknown |  |
| `hermes-state-prune-rewrite` | 1 | ? | unknown |  |
| `hermes-state-readpool` | 1 | ? | unknown |  |
| `hermes-state-repair` | 1 | ? | unknown |  |
| `hermes-state-rewind` | 1 | ? | unknown |  |
| `hermes-state-schema` | 1 | ? | unknown |  |
| `hermes-state-sessions` | 1 | ? | unknown |  |
| `hermes-state-telegram` | 1 | ? | unknown |  |
| `hermes-state-titles` | 1 | ? | unknown |  |
| `hermes-state-usage` | 1 | ? | unknown |  |
| `hermes-state-user-copy` | 1 | ? | unknown |  |
| `hermes-state-wal` | 1 | ? | unknown |  |
| `hermes-yaml` | 1 | ? | unknown |  |
| `mutagen` | 1 | ? | unknown |  |
| `psutil` | 1 | ? | unknown |  |

## Blocked

- `gateway/`: Uses dynamic code (eval, new Function or computed imports).
- `providers/`: Uses dynamic code (eval, new Function or computed imports).
- `tools/`: Uses dynamic code (eval, new Function or computed imports).

## Risk guide

- **String offsets and slicing** (High): JS indexes strings in UTF-16 code units, Rust in UTF-8 bytes. Offsets that cross the boundary must be converted, or non-ASCII text gets wrong ranges silently.
- **Regex features the `regex` crate lacks** (High): Lookaround, backreferences and the `y` flag are not in the `regex` crate. Use `fancy-regex` for those patterns, or rewrite them, and test them on non-ASCII text: `\w`, `\b` and case-insensitive matching differ between engines even where a pattern compiles. JS `/i` without `u` never matches a non-ASCII letter to an ASCII one, Rust `(?i)` does (`ſ` matches `s`, the Kelvin sign `K` matches `k`): write `(?i-u)` or compare with `eq_ignore_ascii_case`.
- **Regexes built at runtime** (High): Patterns come from data or config in JS syntax. Rust regex syntax differs, so user-visible behavior can change. Pin it with tests on real patterns.
- **Dynamic code (eval, new Function, computed import)** (High): eval, new Function and computed imports have no Rust equivalent. Keep this part in JS or replace it with an explicit registry.
- **Dynamic typing** (Medium): Values whose shape is checked at runtime need an explicit Rust enum or `serde_json::Value`.
- **Exceptions** (Low): Thrown errors become `Result`. Errors crossing the bridge must map back to the same JS error shape.
- **Async and promises** (Medium): Promises become futures. Bridge with napi-rs `async fn` or `wasm-bindgen-futures`, or keep the async shell in JS and port the sync core.
- **Generators** (Medium): Lazy generators become iterators, or return a collected Vec across the bridge.
- **Class inheritance** (Medium): Class hierarchies become traits plus composition; `instanceof` checks become enums.
- **Reflection and prototypes** (Medium): Proxy, Reflect, Symbol and prototype tricks need a redesign, not a translation.
- **Number semantics** (Low): JS numbers are f64; bitwise ops truncate to i32; parseInt is lenient. Pick Rust integer types deliberately. Printing differs: `${n}` gives `1e+21`, `1e-7` and `0` for -0 where Rust's `{}` does not, so use `js::number_to_string`. `Math.round` rounds halves up (-2.5 gives -2), Rust's `round` away from zero: use `js::math_round`. `toFixed` rounds exact halves up ((2.5).toFixed(0) is "3"), Rust's `{:.0}` to even.
- **Integer width and overflow** (Medium): Python ints never overflow; C, Go and Java ints wrap or are undefined on overflow. Pick i64, i128 or a bignum crate deliberately, and use wrapping_* where the source wraps.
- **Division and modulo** (Medium): Python `//` and `%` floor toward negative infinity; Rust `/` and `%` truncate: use `div_euclid`/`rem_euclid` or explicit floor math. C division by zero is undefined.
- **Hash iteration order** (High): Set and map iteration order comes from the source language's hash function. A port cannot reproduce it: sort before output, or make the order part of the contract.
- **Concurrency** (Medium): Threads, goroutines and locks need a design in Rust (channels, Send/Sync), not a line-by-line translation; keep concurrency in the caller where possible.

## Whole app

Shape: **cli**.

| Entry point | Kind | Found from |
| --- | --- | --- |
| hermes: `hermes_cli.main:main` | cli | pyproject.toml scripts |
| hermes-agent: `agent.legacy_cli:main` | cli | pyproject.toml scripts |
| hermes-acp: `acp_adapter.entry:main` | cli | pyproject.toml scripts |
| `acp_adapter/entry.py` | cli | `if __name__ == "__main__"` at acp_adapter/entry.py:224 |
| `cron/scheduler.py` | cli | `if __name__ == "__main__"` at cron/scheduler.py:4402 |
| `gateway/run.py` | cli | `if __name__ == "__main__"` at gateway/run.py:5229 |

### Every dependency

| Dependency | Declared | Used by | Rust | Fit | Note |
| --- | --- | --- | --- | --- | --- |
| `fastapi` | yes | 0 | - | none | Web framework: keep the app in Python. |
| `hermes-cli` | no | 9 | ? | unknown |  |
| `hermes-state-errors` | no | 4 | ? | unknown |  |
| `hermes-state-registry` | no | 4 | ? | unknown |  |
| `pm` | no | 4 | ? | unknown |  |
| `run-agent` | no | 4 | ? | unknown |  |
| `tui-gateway` | no | 4 | ? | unknown |  |
| `utils` | no | 4 | ? | unknown |  |
| `hermes-bootstrap` | no | 3 | ? | unknown |  |
| `hermes-dir-acl` | no | 3 | ? | unknown |  |
| `hermes-logging` | no | 3 | ? | unknown |  |
| `hermes-time` | no | 3 | ? | unknown |  |
| `model-tools` | no | 3 | ? | unknown |  |
| `plugins` | no | 3 | ? | unknown |  |
| `hermes-platform` | no | 2 | ? | unknown |  |
| `hermes-state-pidns` | no | 2 | ? | unknown |  |
| `acp` | no | 1 | ? | unknown |  |
| `croniter` | yes | 1 | ? | unknown |  |
| `hermes-constants-scratch` | no | 1 | ? | unknown |  |
| `hermes-constants-scratch-detach` | no | 1 | ? | unknown |  |
| `hermes-startup-watchdog` | no | 1 | ? | unknown |  |
| `hermes-state-api-content` | no | 1 | ? | unknown |  |
| `hermes-state-compression` | no | 1 | ? | unknown |  |
| `hermes-state-coverage` | no | 1 | ? | unknown |  |
| `hermes-state-dbfile` | no | 1 | ? | unknown |  |
| `hermes-state-gateway` | no | 1 | ? | unknown |  |
| `hermes-state-guard` | no | 1 | ? | unknown |  |
| `hermes-state-health` | no | 1 | ? | unknown |  |
| `hermes-state-holders` | no | 1 | ? | unknown |  |
| `hermes-state-keys` | no | 1 | ? | unknown |  |
| `hermes-state-local` | no | 1 | ? | unknown |  |
| `hermes-state-lockguard` | no | 1 | ? | unknown |  |
| `hermes-state-lockowners` | no | 1 | ? | unknown |  |
| `hermes-state-maintenance` | no | 1 | ? | unknown |  |
| `hermes-state-messages` | no | 1 | ? | unknown |  |
| `hermes-state-portability` | no | 1 | ? | unknown |  |
| `hermes-state-profile-repair` | no | 1 | ? | unknown |  |
| `hermes-state-prune-rewrite` | no | 1 | ? | unknown |  |
| `hermes-state-readpool` | no | 1 | ? | unknown |  |
| `hermes-state-repair` | no | 1 | ? | unknown |  |
| `hermes-state-rewind` | no | 1 | ? | unknown |  |
| `hermes-state-schema` | no | 1 | ? | unknown |  |
| `hermes-state-sessions` | no | 1 | ? | unknown |  |
| `hermes-state-telegram` | no | 1 | ? | unknown |  |
| `hermes-state-titles` | no | 1 | ? | unknown |  |
| `hermes-state-usage` | no | 1 | ? | unknown |  |
| `hermes-state-user-copy` | no | 1 | ? | unknown |  |
| `hermes-state-wal` | no | 1 | ? | unknown |  |
| `hermes-yaml` | no | 1 | ? | unknown |  |
| `mutagen` | no | 1 | ? | unknown |  |
| `psutil` | yes | 1 | ? | unknown |  |
| `browser-harness` | yes | 0 | ? | unknown |  |
| `certifi` | yes | 0 | ? | unknown |  |
| `concurrent-log-handler` | yes | 0 | ? | unknown |  |
| `fire` | yes | 0 | ? | unknown |  |
| `firecrawl-anydoc` | yes | 0 | ? | unknown |  |
| `httptools` | yes | 0 | ? | unknown |  |
| `markdown` | yes | 0 | ? | unknown |  |
| `nemo-relay` | yes | 0 | ? | unknown |  |
| `openai` | yes | 0 | ? | unknown |  |
| `packaging` | yes | 0 | ? | unknown |  |
| `pathspec` | yes | 0 | ? | unknown |  |
| `pillow-heif` | yes | 0 | ? | unknown |  |
| `prompt-toolkit` | yes | 0 | ? | unknown |  |
| `ptyprocess` | yes | 0 | ? | unknown |  |
| `pyjwt` | yes | 0 | ? | unknown |  |
| `python-dotenv` | yes | 0 | ? | unknown |  |
| `python-multipart` | yes | 0 | ? | unknown |  |
| `pywin32` | yes | 0 | ? | unknown |  |
| `pywinpty` | yes | 0 | ? | unknown |  |
| `resvg-py` | yes | 0 | ? | unknown |  |
| `ruamel.yaml` | yes | 0 | ? | unknown |  |
| `ruamel.yaml.clib` | yes | 0 | ? | unknown |  |
| `snowballstemmer` | yes | 0 | ? | unknown |  |
| `tenacity` | yes | 0 | ? | unknown |  |
| `truststore` | yes | 0 | ? | unknown |  |
| `tzdata` | yes | 0 | ? | unknown |  |
| `uvicorn` | yes | 0 | ? | unknown |  |
| `watchfiles` | yes | 0 | ? | unknown |  |
| `websockets` | yes | 0 | ? | unknown |  |
| `winrt-windows-applicationmodel` | yes | 0 | ? | unknown |  |
| `winrt-windows-foundation` | yes | 0 | ? | unknown |  |
| `winrt-windows-foundation-collections` | yes | 0 | ? | unknown |  |
| `winrt-windows-services-store` | yes | 0 | ? | unknown |  |
| `cryptography` | yes | 0 | ring, RustCrypto crates | partial |  |
| `pillow` | yes | 0 | image | partial |  |
| `pydantic` | yes | 0 | serde, garde | partial | Validation errors will differ. |
| `httpx` | yes | 0 | reqwest | good |  |
| `jinja2` | yes | 0 | minijinja | good | Same template language, by Jinja's author. |
| `requests` | yes | 0 | reqwest | good |  |
| `rich` | yes | 0 | owo-colors, anstream | good |  |
| `tomli-w` | yes | 0 | toml | good |  |
| `urllib3` | yes | 0 | reqwest | good |  |

### Pieces, in port order

- wave 0: `hermes_constants.py` (1188 lines)
- wave 1: `providers/` (1364 lines)
- wave 1: `tools/`, `toolsets.py`, tangled: one piece (2913 lines)
- wave 2: `agent/` (9097 lines)
- wave 3: `cron/` (16622 lines)
- wave 3: `hermes_state_common.py` (1321 lines)
- wave 4: `hermes_state_fts.py` (517 lines)
- wave 5: `hermes_state_search.py` (1412 lines)
- wave 6: `hermes_state.py` (1592 lines)
- wave 7: `acp_adapter/` (5669 lines)
- wave 7: `gateway/` (4770 lines)

### Decisions for the user

**D1** (shape): The app is a cli, entered through hermes (hermes_cli.main:main), hermes-agent (agent.legacy_cli:main), hermes-acp (acp_adapter.entry:main), acp_adapter/entry.py, cron/scheduler.py, gateway/run.py. Port every entry point, in that shape?

> Yes: a clap front door (`app front-door`) takes every command over first

**D2** (blocked gateway/): gateway/ is blocked: Uses dynamic code (eval, new Function or computed imports). Rework that part in Rust, or keep the unit outside?

> Research the blocker first; then port it with the piece that uses it

**D3** (blocked providers/): providers/ is blocked: Uses dynamic code (eval, new Function or computed imports). Rework that part in Rust, or keep the unit outside?

> Research the blocker first; then port it with the piece that uses it

**D4** (blocked tools/): tools/ is blocked: Uses dynamic code (eval, new Function or computed imports). Rework that part in Rust, or keep the unit outside?

> Research the blocker first; then port it with the piece that uses it

**D5** (dependency fastapi): No crate does what fastapi does (declared, no unit imports it; Web framework: keep the app in Python). Reimplement what the app uses, or drop the feature?

> Reimplement only the part the app uses, proved by its recorded cases

**D6** (dependency hermes-cli): Which crate replaces hermes-cli (used by 9 units)?

> Research it against primary sources into rust/research/hermes-cli.md before porting what uses it

**D7** (dependency hermes-state-errors): Which crate replaces hermes-state-errors (used by 4 units)?

> Research it against primary sources into rust/research/hermes-state-errors.md before porting what uses it

**D8** (dependency hermes-state-registry): Which crate replaces hermes-state-registry (used by 4 units)?

> Research it against primary sources into rust/research/hermes-state-registry.md before porting what uses it

**D9** (dependency pm): Which crate replaces pm (used by 4 units)?

> Research it against primary sources into rust/research/pm.md before porting what uses it

**D10** (dependency run-agent): Which crate replaces run-agent (used by 4 units)?

> Research it against primary sources into rust/research/run-agent.md before porting what uses it

**D11** (dependency tui-gateway): Which crate replaces tui-gateway (used by 4 units)?

> Research it against primary sources into rust/research/tui-gateway.md before porting what uses it

**D12** (dependency utils): Which crate replaces utils (used by 4 units)?

> Research it against primary sources into rust/research/utils.md before porting what uses it

**D13** (dependency hermes-bootstrap): Which crate replaces hermes-bootstrap (used by 3 units)?

> Research it against primary sources into rust/research/hermes-bootstrap.md before porting what uses it

**D14** (dependency hermes-dir-acl): Which crate replaces hermes-dir-acl (used by 3 units)?

> Research it against primary sources into rust/research/hermes-dir-acl.md before porting what uses it

**D15** (dependency hermes-logging): Which crate replaces hermes-logging (used by 3 units)?

> Research it against primary sources into rust/research/hermes-logging.md before porting what uses it

**D16** (dependency hermes-time): Which crate replaces hermes-time (used by 3 units)?

> Research it against primary sources into rust/research/hermes-time.md before porting what uses it

**D17** (dependency model-tools): Which crate replaces model-tools (used by 3 units)?

> Research it against primary sources into rust/research/model-tools.md before porting what uses it

**D18** (dependency plugins): Which crate replaces plugins (used by 3 units)?

> Research it against primary sources into rust/research/plugins.md before porting what uses it

**D19** (dependency hermes-platform): Which crate replaces hermes-platform (used by 2 units)?

> Research it against primary sources into rust/research/hermes-platform.md before porting what uses it

**D20** (dependency hermes-state-pidns): Which crate replaces hermes-state-pidns (used by 2 units)?

> Research it against primary sources into rust/research/hermes-state-pidns.md before porting what uses it

**D21** (dependency acp): Which crate replaces acp (used by 1 unit)?

> Research it against primary sources into rust/research/acp.md before porting what uses it

**D22** (dependency croniter): Which crate replaces croniter (used by 1 unit)?

> Research it against primary sources into rust/research/croniter.md before porting what uses it

**D23** (dependency hermes-constants-scratch): Which crate replaces hermes-constants-scratch (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-constants-scratch.md before porting what uses it

**D24** (dependency hermes-constants-scratch-detach): Which crate replaces hermes-constants-scratch-detach (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-constants-scratch-detach.md before porting what uses it

**D25** (dependency hermes-startup-watchdog): Which crate replaces hermes-startup-watchdog (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-startup-watchdog.md before porting what uses it

**D26** (dependency hermes-state-api-content): Which crate replaces hermes-state-api-content (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-api-content.md before porting what uses it

**D27** (dependency hermes-state-compression): Which crate replaces hermes-state-compression (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-compression.md before porting what uses it

**D28** (dependency hermes-state-coverage): Which crate replaces hermes-state-coverage (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-coverage.md before porting what uses it

**D29** (dependency hermes-state-dbfile): Which crate replaces hermes-state-dbfile (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-dbfile.md before porting what uses it

**D30** (dependency hermes-state-gateway): Which crate replaces hermes-state-gateway (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-gateway.md before porting what uses it

**D31** (dependency hermes-state-guard): Which crate replaces hermes-state-guard (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-guard.md before porting what uses it

**D32** (dependency hermes-state-health): Which crate replaces hermes-state-health (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-health.md before porting what uses it

**D33** (dependency hermes-state-holders): Which crate replaces hermes-state-holders (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-holders.md before porting what uses it

**D34** (dependency hermes-state-keys): Which crate replaces hermes-state-keys (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-keys.md before porting what uses it

**D35** (dependency hermes-state-local): Which crate replaces hermes-state-local (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-local.md before porting what uses it

**D36** (dependency hermes-state-lockguard): Which crate replaces hermes-state-lockguard (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-lockguard.md before porting what uses it

**D37** (dependency hermes-state-lockowners): Which crate replaces hermes-state-lockowners (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-lockowners.md before porting what uses it

**D38** (dependency hermes-state-maintenance): Which crate replaces hermes-state-maintenance (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-maintenance.md before porting what uses it

**D39** (dependency hermes-state-messages): Which crate replaces hermes-state-messages (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-messages.md before porting what uses it

**D40** (dependency hermes-state-portability): Which crate replaces hermes-state-portability (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-portability.md before porting what uses it

**D41** (dependency hermes-state-profile-repair): Which crate replaces hermes-state-profile-repair (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-profile-repair.md before porting what uses it

**D42** (dependency hermes-state-prune-rewrite): Which crate replaces hermes-state-prune-rewrite (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-prune-rewrite.md before porting what uses it

**D43** (dependency hermes-state-readpool): Which crate replaces hermes-state-readpool (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-readpool.md before porting what uses it

**D44** (dependency hermes-state-repair): Which crate replaces hermes-state-repair (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-repair.md before porting what uses it

**D45** (dependency hermes-state-rewind): Which crate replaces hermes-state-rewind (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-rewind.md before porting what uses it

**D46** (dependency hermes-state-schema): Which crate replaces hermes-state-schema (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-schema.md before porting what uses it

**D47** (dependency hermes-state-sessions): Which crate replaces hermes-state-sessions (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-sessions.md before porting what uses it

**D48** (dependency hermes-state-telegram): Which crate replaces hermes-state-telegram (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-telegram.md before porting what uses it

**D49** (dependency hermes-state-titles): Which crate replaces hermes-state-titles (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-titles.md before porting what uses it

**D50** (dependency hermes-state-usage): Which crate replaces hermes-state-usage (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-usage.md before porting what uses it

**D51** (dependency hermes-state-user-copy): Which crate replaces hermes-state-user-copy (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-user-copy.md before porting what uses it

**D52** (dependency hermes-state-wal): Which crate replaces hermes-state-wal (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-state-wal.md before porting what uses it

**D53** (dependency hermes-yaml): Which crate replaces hermes-yaml (used by 1 unit)?

> Research it against primary sources into rust/research/hermes-yaml.md before porting what uses it

**D54** (dependency mutagen): Which crate replaces mutagen (used by 1 unit)?

> Research it against primary sources into rust/research/mutagen.md before porting what uses it

**D55** (dependency psutil): Which crate replaces psutil (used by 1 unit)?

> Research it against primary sources into rust/research/psutil.md before porting what uses it

**D56** (dependency browser-harness): Which crate replaces browser-harness (declared, no unit imports it)?

> Research it against primary sources into rust/research/browser-harness.md before porting what uses it

**D57** (dependency certifi): Which crate replaces certifi (declared, no unit imports it)?

> Research it against primary sources into rust/research/certifi.md before porting what uses it

**D58** (dependency concurrent-log-handler): Which crate replaces concurrent-log-handler (declared, no unit imports it)?

> Research it against primary sources into rust/research/concurrent-log-handler.md before porting what uses it

**D59** (dependency fire): Which crate replaces fire (declared, no unit imports it)?

> Research it against primary sources into rust/research/fire.md before porting what uses it

**D60** (dependency firecrawl-anydoc): Which crate replaces firecrawl-anydoc (declared, no unit imports it)?

> Research it against primary sources into rust/research/firecrawl-anydoc.md before porting what uses it

**D61** (dependency httptools): Which crate replaces httptools (declared, no unit imports it)?

> Research it against primary sources into rust/research/httptools.md before porting what uses it

**D62** (dependency markdown): Which crate replaces markdown (declared, no unit imports it)?

> Research it against primary sources into rust/research/markdown.md before porting what uses it

**D63** (dependency nemo-relay): Which crate replaces nemo-relay (declared, no unit imports it)?

> Research it against primary sources into rust/research/nemo-relay.md before porting what uses it

**D64** (dependency openai): Which crate replaces openai (declared, no unit imports it)?

> Research it against primary sources into rust/research/openai.md before porting what uses it

**D65** (dependency packaging): Which crate replaces packaging (declared, no unit imports it)?

> Research it against primary sources into rust/research/packaging.md before porting what uses it

**D66** (dependency pathspec): Which crate replaces pathspec (declared, no unit imports it)?

> Research it against primary sources into rust/research/pathspec.md before porting what uses it

**D67** (dependency pillow-heif): Which crate replaces pillow-heif (declared, no unit imports it)?

> Research it against primary sources into rust/research/pillow-heif.md before porting what uses it

**D68** (dependency prompt-toolkit): Which crate replaces prompt-toolkit (declared, no unit imports it)?

> Research it against primary sources into rust/research/prompt-toolkit.md before porting what uses it

**D69** (dependency ptyprocess): Which crate replaces ptyprocess (declared, no unit imports it)?

> Research it against primary sources into rust/research/ptyprocess.md before porting what uses it

**D70** (dependency pyjwt): Which crate replaces pyjwt (declared, no unit imports it)?

> Research it against primary sources into rust/research/pyjwt.md before porting what uses it

**D71** (dependency python-dotenv): Which crate replaces python-dotenv (declared, no unit imports it)?

> Research it against primary sources into rust/research/python-dotenv.md before porting what uses it

**D72** (dependency python-multipart): Which crate replaces python-multipart (declared, no unit imports it)?

> Research it against primary sources into rust/research/python-multipart.md before porting what uses it

**D73** (dependency pywin32): Which crate replaces pywin32 (declared, no unit imports it)?

> Research it against primary sources into rust/research/pywin32.md before porting what uses it

**D74** (dependency pywinpty): Which crate replaces pywinpty (declared, no unit imports it)?

> Research it against primary sources into rust/research/pywinpty.md before porting what uses it

**D75** (dependency resvg-py): Which crate replaces resvg-py (declared, no unit imports it)?

> Research it against primary sources into rust/research/resvg-py.md before porting what uses it

**D76** (dependency ruamel.yaml): Which crate replaces ruamel.yaml (declared, no unit imports it)?

> Research it against primary sources into rust/research/ruamel.yaml.md before porting what uses it

**D77** (dependency ruamel.yaml.clib): Which crate replaces ruamel.yaml.clib (declared, no unit imports it)?

> Research it against primary sources into rust/research/ruamel.yaml.clib.md before porting what uses it

**D78** (dependency snowballstemmer): Which crate replaces snowballstemmer (declared, no unit imports it)?

> Research it against primary sources into rust/research/snowballstemmer.md before porting what uses it

**D79** (dependency tenacity): Which crate replaces tenacity (declared, no unit imports it)?

> Research it against primary sources into rust/research/tenacity.md before porting what uses it

**D80** (dependency truststore): Which crate replaces truststore (declared, no unit imports it)?

> Research it against primary sources into rust/research/truststore.md before porting what uses it

**D81** (dependency tzdata): Which crate replaces tzdata (declared, no unit imports it)?

> Research it against primary sources into rust/research/tzdata.md before porting what uses it

**D82** (dependency uvicorn): Which crate replaces uvicorn (declared, no unit imports it)?

> Research it against primary sources into rust/research/uvicorn.md before porting what uses it

**D83** (dependency watchfiles): Which crate replaces watchfiles (declared, no unit imports it)?

> Research it against primary sources into rust/research/watchfiles.md before porting what uses it

**D84** (dependency websockets): Which crate replaces websockets (declared, no unit imports it)?

> Research it against primary sources into rust/research/websockets.md before porting what uses it

**D85** (dependency winrt-windows-applicationmodel): Which crate replaces winrt-windows-applicationmodel (declared, no unit imports it)?

> Research it against primary sources into rust/research/winrt-windows-applicationmodel.md before porting what uses it

**D86** (dependency winrt-windows-foundation): Which crate replaces winrt-windows-foundation (declared, no unit imports it)?

> Research it against primary sources into rust/research/winrt-windows-foundation.md before porting what uses it

**D87** (dependency winrt-windows-foundation-collections): Which crate replaces winrt-windows-foundation-collections (declared, no unit imports it)?

> Research it against primary sources into rust/research/winrt-windows-foundation-collections.md before porting what uses it

**D88** (dependency winrt-windows-services-store): Which crate replaces winrt-windows-services-store (declared, no unit imports it)?

> Research it against primary sources into rust/research/winrt-windows-services-store.md before porting what uses it

**D89** (dependency cryptography): cryptography maps to ring, RustCrypto crates only in part (declared, no unit imports it). Use it?

> Use ring, RustCrypto crates, and record cases for the behavior it does not cover

**D90** (dependency pillow): pillow maps to image only in part (declared, no unit imports it). Use it?

> Use image, and record cases for the behavior it does not cover

**D91** (dependency pydantic): pydantic maps to serde, garde only in part (declared, no unit imports it; Validation errors will differ). Use it?

> Use serde, garde, and record cases for the behavior it does not cover

**D92** (tangled units): tools/, toolsets.py import each other (2913 lines). Port them as one piece?

> Yes: one piece in wave 1, no glue between its parts

