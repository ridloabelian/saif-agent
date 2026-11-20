# Panduan Sinkronisasi Upstream `NousResearch/hermes-agent`

Dokumen ini menjelaskan cara melacak, menyaring, mem-porting, dan memverifikasi pembaruan dari [`NousResearch/hermes-agent`](https://github.com/NousResearch/hermes-agent) ke `saif-agent` (100% Pure Rust) secara rutin, cepat, dan aman.

---

## Ringkasan Alur 4 Langkah

Setiap kali ingin memeriksa atau mengikuti update terbaru `hermes-agent`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
TMP_HOME=$(mktemp -d)

# 1. Cek apakah lockfile sudah tertinggal dari upstream HEAD
SAIF_HOME="$TMP_HOME" ./target/release/saif upstream status

# 2. Lihat perubahan bersih yang relevan untuk di-port ke Rust (noise otomatis tersaring)
SAIF_HOME="$TMP_HOME" ./target/release/saif upstream diff --port-only
```

Setelah perubahan di-port ke crate Rust terkait + ditambah test & golden case `.jsonl` + lolos quality gate:

```bash
# 3. Kunci SHA commit upstream terbaru ke conformance/UPSTREAM_LOCK.json
SAIF_HOME="$TMP_HOME" ./target/release/saif upstream ack <HEAD_SHA> --subject "<ringkasan_commit>"
rm -rf "$TMP_HOME"
```

Lalu commit & push ke `origin main`.

---

## Cara Kerja `conformance/UPSTREAM_LOCK.json`

`saif upstream diff` tidak membaca ribuan commit satu per satu secara mentah, melainkan menghitung **net diff** antara `synced_commit` dan `HEAD` (via `git clone --bare --filter=blob:none --depth=1` di folder sementara `/tmp` yang otomatis terhapus selesai perintah), lalu mengklasifikasikan setiap file ke 4 kategori:

1. **`IGNORED_NOISE`** (`ignore_globs`):
   - `plugin-catalog/**`, `.github/**`, `tests/**`, `docs/**`, `README.md`, `*.lock`, `uv.lock`, `pyproject.toml`, `.mailmap`, `__pycache__/**`, dll.
   - Otomatis disembunyikan saat flag `--port-only` dipakai.
2. **`SKIP_RUNTIME_SPECIFIC`** (`runtime_specific_prefixes`):
   - Wrapper Python/Node murni yang tidak dibutuhkan di binary statis Rust (`hermes_cli/hermes_bootstrap*`, `hermes_cli/old_updater*`, `hermes_cli/npm_engine*`, `setup.py`, `apps/desktop/electron*`).
3. **`PORT_REQUIRED`** (`subsystems`):
   - File inti perilaku Hermes yang langsung dipetakan ke 14 subsistem `saif-agent`, lengkap dengan daftar `rust_targets` dan `conformance_suites` yang perlu diupdate/diuji.
4. **`UNMAPPED`**:
   - File baru di root/direktori baru upstream yang belum punya aturan di `UPSTREAM_LOCK.json`, sehingga tidak ada fitur baru upstream yang terlewat diam-diam.

---

## Filter Per-Subsistem & Output JSON

```bash
# Fokus pada satu subsistem saja (mis. gateway_platforms atau profile_secrets)
SAIF_HOME="$TMP_HOME" ./target/release/saif upstream diff --port-only --subsystem gateway_platforms

# Output JSON terstruktur untuk otomasi atau agen AI
SAIF_HOME="$TMP_HOME" ./target/release/saif upstream diff --port-only --json

# Bandingkan rentang commit spesifik atau terhadap clone lokal read-only
SAIF_HOME="$TMP_HOME" ./target/release/saif upstream diff --from <SHA_A> --to <SHA_B> --port-only
```

---

## Aturan Wajib Saat Sinkronisasi

1. **Jangan pernah memodifikasi `~/.hermes` atau `~/.hermes/hermes-agent`** — direktori tersebut adalah state produksi live dan hanya boleh dibaca sebagai referensi oracle.
2. **Jangan pernah mem-port ketergantungan Python/Node ke runtime utama** — terjemahkan kontrak perilakunya ke pure Rust (`saif-*`) dengan `rustls`, SQLite bundled, dan error propagation eksplisit.
3. **Wajib menjaga 8 invarian inti** di [HERMES_ANATOMY.md](HERMES_ANATOMY.md) (terutama *Byte-Stable `system[0]`*, *Strict Role Alternation*, *Fail-Closed `SecretScope`*, *4-Invariant Draft Streaming*, dan *At-Most-Once Cron `advance_before_dispatch`*).
4. **Wajib lolos Quality Gate penuh** sebelum menjalankan `saif upstream ack`:
   - `grep -rn --include="*.rs" "todo!\|unimplemented!" crates/ apps/` (0 match)
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace` + test & build WASM `apps/saif-web-ui`
