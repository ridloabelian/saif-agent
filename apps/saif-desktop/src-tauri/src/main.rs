//! Pure-Rust Tauri v2 Desktop shell (`apps/desktop` parity):
//! - Spawns `saif serve` as a child process bound to the app lifetime (`SAIF_SERVE_HEADLESS=1`).
//! - Spawns `saif gateway run` DETACHED so messaging bots survive Desktop quit (`gateway/AGENTS.md`).
//! - Verifies updater release payloads via SHA-256 digests cleanly.

use sha2::{Digest, Sha256};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

struct ServeHandle(Mutex<Option<Child>>);

#[derive(Debug, Clone)]
pub struct ServeSidecarConfig {
    pub listen_addr: String,
    pub session_token: String,
}

impl ServeSidecarConfig {
    pub fn args(&self) -> Vec<String> {
        vec![
            "serve".to_string(),
            "--listen".to_string(),
            self.listen_addr.clone(),
        ]
    }
}

/// Verify downloaded updater bundle checksum before executing self-update.
pub fn verify_release_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    let actual = hex::encode(Sha256::digest(bytes));
    actual.eq_ignore_ascii_case(expected_hex.trim())
}

#[tauri::command]
fn start_detached_gateway(saif_bin: String, listen_addr: String) -> Result<u32, String> {
    let mut cmd = Command::new(&saif_bin);
    cmd.args(["gateway", "run", "--listen", &listen_addr])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc_setsid();
                Ok(())
            });
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed spawning detached gateway: {e}"))?;
    Ok(child.id())
}

#[tauri::command]
fn verify_update_digest(payload: Vec<u8>, expected_sha256: String) -> bool {
    verify_release_sha256(&payload, &expected_sha256)
}

#[cfg(unix)]
fn libc_setsid() {
    extern "C" {
        fn setsid() -> i32;
    }
    unsafe {
        let _ = setsid();
    }
}

fn main() {
    let cfg = ServeSidecarConfig {
        listen_addr: "127.0.0.1:8788".to_string(),
        session_token: "desktop-local".to_string(),
    };
    let serve_child = Command::new("saif")
        .args(cfg.args())
        .env("SAIF_SERVE_HEADLESS", "1")
        .env("SAIF_SERVE_TOKEN", &cfg.session_token)
        .spawn()
        .ok();

    tauri::Builder::default()
        .manage(ServeHandle(Mutex::new(serve_child)))
        .invoke_handler(tauri::generate_handler![
            start_detached_gateway,
            verify_update_digest
        ])
        .build(tauri::generate_context!())
        .expect("error building Saif Desktop")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app_handle.state::<ServeHandle>();
                if let Ok(mut guard) = state.0.lock() {
                    if let Some(mut child) = guard.take() {
                        let _ = child.kill();
                    }
                }
            }
        });
}
