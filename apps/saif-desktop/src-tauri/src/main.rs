//! Pure-Rust Tauri v2 Desktop shell (`apps/desktop` parity):
//! - Spawns `saif serve` as a child process bound to the app lifetime.
//! - Spawns `saif gateway run` DETACHED so messaging bots survive Desktop quit (`gateway/AGENTS.md`).

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

struct ServeHandle(Mutex<Option<Child>>);

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

    let child = cmd.spawn().map_err(|e| format!("failed spawning detached gateway: {e}"))?;
    Ok(child.id())
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
    let serve_child = Command::new("saif")
        .args(["serve", "--listen", "127.0.0.1:8787"])
        .env("SAIF_SERVE_HEADLESS", "1")
        .spawn()
        .ok();

    tauri::Builder::default()
        .manage(ServeHandle(Mutex::new(serve_child)))
        .invoke_handler(tauri::generate_handler![start_detached_gateway])
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
