use crate::registry::{ToolContext, ToolError, ToolRegistry};
use rand::Rng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;

pub const MAX_STDOUT_BYTES: usize = 50_000;
pub const MAX_STDERR_BYTES: usize = 10_000;
pub const MAX_SPILLED_STDOUT_BYTES: usize = 5_000_000;
pub const DEFAULT_MAX_TOOL_CALLS: usize = 50;

pub const SANDBOX_ALLOWED_TOOLS: &[&str] = &[
    "web_search",
    "web_extract",
    "read_file",
    "write_file",
    "search_files",
    "patch",
    "terminal",
];

/// Exact Hermes `tools.tts_tool_providers._config_bool` coercion semantics.
pub fn config_bool(raw: &str, default: bool) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => true,
        "0" | "false" | "no" | "off" | "disabled" => false,
        _ => default,
    }
}

fn floor_char_boundary_up_to(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut idx = max_bytes;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary_from(s: &str, start_bytes: usize) -> usize {
    if start_bytes == 0 {
        return 0;
    }
    let mut idx = start_bytes.min(s.len());
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// 40%/60% byte truncation with optional disk spill (`tools/code_execution_tool.py` parity).
pub fn truncate_stdout_bytes(stdout: &str, spill_root: Option<&Path>) -> (String, Value) {
    let total = stdout.len();
    let captured = total.min(MAX_STDOUT_BYTES);
    let mut meta = json!({
        "stdout_truncated": total > captured,
        "stdout_bytes_captured": captured,
        "stdout_bytes_total": total,
        "stdout_bytes_omitted": total - captured,
    });

    if total <= MAX_STDOUT_BYTES {
        return (stdout.to_string(), meta);
    }

    let head_bytes = (MAX_STDOUT_BYTES * 2) / 5; // 40%
    let tail_bytes = MAX_STDOUT_BYTES - head_bytes; // 60%
    let head_end = floor_char_boundary_up_to(stdout, head_bytes);
    let tail_start = ceil_char_boundary_from(stdout, stdout.len().saturating_sub(tail_bytes));
    let omitted = total - captured;
    let marker = format!("\n\n[... {omitted} bytes truncated of {total} total bytes ...]\n\n");
    let combined = format!("{}{marker}{}", &stdout[..head_end], &stdout[tail_start..]);

    if let Some(root) = spill_root {
        let exec_dir = root.join("cache").join("exec");
        if fs::create_dir_all(&exec_dir).is_ok() {
            let capped = if stdout.len() > MAX_SPILLED_STDOUT_BYTES {
                let end = floor_char_boundary_up_to(stdout, MAX_SPILLED_STDOUT_BYTES);
                &stdout[..end]
            } else {
                stdout
            };
            let digest = hex::encode(Sha256::digest(capped.as_bytes()));
            let spill_path = exec_dir.join(format!("stdout-{}.txt", &digest[..12]));
            if fs::write(&spill_path, capped).is_ok() {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert(
                        "stdout_spill_path".into(),
                        json!(spill_path.display().to_string()),
                    );
                }
            }
        }
    }

    (combined, meta)
}

fn generate_rpc_shim(addr: std::net::SocketAddr, token: &str) -> String {
    format!(
        r#"import json, socket

_ADDR = ("{ip}", {port})
_TOKEN = "{token}"

def _rpc(tool, args):
    payload = json.dumps({{"token": _TOKEN, "tool": tool, "args": args}}) + "\n"
    with socket.create_connection(_ADDR, timeout=30) as s:
        s.sendall(payload.encode("utf-8"))
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(4096)
            if not chunk:
                break
            buf += chunk
    res = json.loads(buf.decode("utf-8").strip() or "{{}}")
    if not res.get("ok"):
        raise RuntimeError(res.get("error", "rpc error"))
    return json.loads(res.get("result", "{{}}"))

def read_file(path, offset=None, limit=None):
    a = {{"path": path}}
    if offset is not None: a["offset"] = offset
    if limit is not None: a["limit"] = limit
    return _rpc("read_file", a)

def write_file(path, content):
    return _rpc("write_file", {{"path": path, "content": content}})

def patch(**kwargs):
    return _rpc("patch", kwargs)

def search_files(pattern, target="content", path=None, limit=None):
    a = {{"pattern": pattern, "target": target}}
    if path is not None: a["path"] = path
    if limit is not None: a["limit"] = limit
    return _rpc("search_files", a)

def web_search(query):
    return _rpc("web_search", {{"query": query}})

def web_extract(url):
    return _rpc("web_extract", {{"url": url}})

def terminal(command, workdir=None, timeout=None):
    a = {{"command": command}}
    if workdir is not None: a["workdir"] = workdir
    if timeout is not None: a["timeout"] = timeout
    return _rpc("terminal", a)
"#,
        ip = addr.ip(),
        port = addr.port(),
        token = token,
    )
}

/// Execute Python script calling allowlisted `saif_tools` via authenticated loopback TCP RPC.
pub async fn handle_execute_code(
    args: &Value,
    ctx: &ToolContext,
    registry: &ToolRegistry,
) -> Result<String, ToolError> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'code'".to_string()))?;
    let max_calls = args
        .get("max_tool_calls")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_TOOL_CALLS as u64) as usize;

    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| ToolError::Execution(format!("failed binding code RPC listener: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    let tmp_dir = std::env::temp_dir().join(format!("saif-exec-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&tmp_dir).map_err(|e| ToolError::Execution(e.to_string()))?;
    let shim = generate_rpc_shim(addr, &token);
    fs::write(tmp_dir.join("saif_tools.py"), &shim)
        .and_then(|_| fs::write(tmp_dir.join("hermes_tools.py"), &shim))
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    let reg = registry.clone();
    let ctx_cloned = ctx.clone();
    let tok_srv = token.clone();
    let call_counter = Arc::new(AtomicUsize::new(0));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let stop_srv = shutdown.clone();

    let srv_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop_srv.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { break; };
                    let reg = reg.clone();
                    let ctx = ctx_cloned.clone();
                    let tok = tok_srv.clone();
                    let cnt = Arc::clone(&call_counter);
                    tokio::spawn(async move {
                        let (r, mut w) = stream.into_split();
                        let mut reader = BufReader::new(r);
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let req: Value = match serde_json::from_str(&line) {
                            Ok(v) => v,
                            Err(e) => {
                                let _ = w.write_all(format!("{{\"ok\":false,\"error\":\"bad json: {e}\"}}\n").as_bytes()).await;
                                return;
                            }
                        };
                        if req.get("token").and_then(|v| v.as_str()) != Some(tok.as_str()) {
                            let _ = w.write_all(b"{\"ok\":false,\"error\":\"unauthorized\"}\n").await;
                            return;
                        }
                        let tool_name = req.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                        if !SANDBOX_ALLOWED_TOOLS.contains(&tool_name) {
                            let _ = w.write_all(format!("{{\"ok\":false,\"error\":\"tool '{tool_name}' not allowed in sandbox\"}}\n").as_bytes()).await;
                            return;
                        }
                        if cnt.fetch_add(1, Ordering::SeqCst) >= max_calls {
                            let _ = w.write_all(b"{\"ok\":false,\"error\":\"max_tool_calls exceeded\"}\n").await;
                            return;
                        }
                        let tool_args = req.get("args").cloned().unwrap_or_else(|| json!({}));
                        let resp = match reg.dispatch(tool_name, tool_args, &ctx).await {
                            Ok(s) => json!({ "ok": true, "result": s }),
                            Err(e) => json!({ "ok": false, "error": e.to_string() }),
                        };
                        let _ = w.write_all((resp.to_string() + "\n").as_bytes()).await;
                    });
                }
            }
        }
    });

    let mut child_env = ctx
        .profile
        .secrets
        .build_child_env(&ctx.profile.home_dir, &ctx.config.terminal.env_passthrough);
    let existing_pypath = child_env.get("PYTHONPATH").cloned().unwrap_or_default();
    let new_pypath = if existing_pypath.is_empty() {
        tmp_dir.display().to_string()
    } else {
        format!("{}:{existing_pypath}", tmp_dir.display())
    };
    child_env.insert("PYTHONPATH".to_string(), new_pypath);

    let out = Command::new("python3")
        .arg("-c")
        .arg(code)
        .current_dir(&ctx.cwd)
        .env_clear()
        .envs(&child_env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    shutdown.cancel();
    let _ = srv_handle.await;
    let _ = fs::remove_dir_all(&tmp_dir);

    let out = out.map_err(|e| ToolError::Execution(format!("python3 execution failed: {e}")))?;
    let stdout_raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr_raw = String::from_utf8_lossy(&out.stderr).into_owned();
    let stderr = if stderr_raw.len() > MAX_STDERR_BYTES {
        let end = floor_char_boundary_up_to(&stderr_raw, MAX_STDERR_BYTES);
        format!("{}...", &stderr_raw[..end])
    } else {
        stderr_raw
    };
    let (stdout, meta) = truncate_stdout_bytes(&stdout_raw, Some(&ctx.profile.home_dir));

    Ok(json!({
        "success": out.status.success(),
        "exit_code": out.status.code().unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
        "metadata": meta,
    })
    .to_string())
}

pub async fn handle_computer_use(args: &Value, _ctx: &ToolContext) -> Result<String, ToolError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;
    let valid = matches!(
        action,
        "screenshot"
            | "mouse_move"
            | "left_click"
            | "right_click"
            | "type_text"
            | "key_press"
            | "cursor_position"
    );
    if !valid {
        return Err(ToolError::InvalidArgs(format!(
            "unsupported computer_use action '{action}'"
        )));
    }
    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_display =
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();

    if dry_run || !has_display {
        return Ok(json!({
            "success": dry_run,
            "action": action,
            "has_display": has_display,
            "dry_run": dry_run,
            "note": if has_display { "dry_run" } else { "no active X11/Wayland display" },
            "x": args.get("x").cloned().unwrap_or(json!(null)),
            "y": args.get("y").cloned().unwrap_or(json!(null)),
            "text": args.get("text").cloned().unwrap_or(json!(null)),
        })
        .to_string());
    }

    let xdotool = saif_platform::ExecutableResolver::which("xdotool");
    let Some(tool_bin) = xdotool else {
        return Ok(json!({
            "success": false,
            "action": action,
            "error": "xdotool not installed on PATH"
        })
        .to_string());
    };

    let mut cmd = Command::new(tool_bin);
    match action {
        "mouse_move" => {
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            cmd.args(["mousemove", &x.to_string(), &y.to_string()]);
        }
        "left_click" => {
            cmd.args(["click", "1"]);
        }
        "right_click" => {
            cmd.args(["click", "3"]);
        }
        "type_text" => {
            let txt = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            cmd.args(["type", "--", txt]);
        }
        "key_press" => {
            let key = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("Return");
            cmd.args(["key", key]);
        }
        "cursor_position" => {
            cmd.args(["getmouselocation"]);
        }
        "screenshot" => {
            return Ok(json!({
                "success": false,
                "action": "screenshot",
                "error": "install scrot or imagemagick import for screen capture"
            })
            .to_string());
        }
        _ => {}
    }

    let out = cmd
        .output()
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(json!({
        "success": out.status.success(),
        "action": action,
        "stdout": String::from_utf8_lossy(&out.stdout),
        "stderr": String::from_utf8_lossy(&out.stderr),
    })
    .to_string())
}
