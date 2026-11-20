use crate::registry::{ToolContext, ToolError};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

static CDP_MSG_ID: AtomicU64 = AtomicU64::new(1);

/// Redaction boundary preventing saved vault secrets from leaking into model tool results.
#[derive(Clone, Default)]
pub struct SecretRedactor {
    secrets: Arc<Mutex<HashSet<String>>>,
}

impl SecretRedactor {
    pub fn register(&self, secret: &str) {
        if secret.len() >= 3 {
            if let Ok(mut guard) = self.secrets.lock() {
                guard.insert(secret.to_string());
            }
        }
    }

    pub fn redact(&self, input: &str) -> String {
        let Ok(guard) = self.secrets.lock() else {
            return input.to_string();
        };
        let mut out = input.to_string();
        for s in guard.iter() {
            if !s.is_empty() && out.contains(s.as_str()) {
                out = out.replace(s.as_str(), "[REDACTED_VAULT_SECRET]");
            }
        }
        out
    }
}

/// Active CDP WebSocket handle tracked per session/task (`tools/browser_supervisor.py` parity).
#[derive(Clone)]
pub struct CdpSupervisorRegistry {
    endpoints: Arc<Mutex<HashMap<String, String>>>,
    pub redactor: SecretRedactor,
}

impl Default for CdpSupervisorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CdpSupervisorRegistry {
    pub fn new() -> Self {
        Self {
            endpoints: Arc::new(Mutex::new(HashMap::new())),
            redactor: SecretRedactor::default(),
        }
    }

    pub fn attach(&self, session_id: &str, cdp_ws_url: impl Into<String>) {
        if let Ok(mut guard) = self.endpoints.lock() {
            guard.insert(session_id.to_string(), cdp_ws_url.into());
        }
    }

    pub fn get(&self, session_id: &str) -> Option<String> {
        self.endpoints
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).cloned())
    }

    pub fn detach(&self, session_id: &str) {
        if let Ok(mut guard) = self.endpoints.lock() {
            guard.remove(session_id);
        }
    }
}

/// Pure-Rust Chrome DevTools Protocol client over WebSocket.
pub struct CdpClient;

impl CdpClient {
    pub async fn call(ws_url: &str, method: &str, params: Value) -> Result<Value, ToolError> {
        let (mut ws, _) = connect_async(ws_url)
            .await
            .map_err(|e| ToolError::Execution(format!("CDP WebSocket connect failed: {e}")))?;

        let id = CDP_MSG_ID.fetch_add(1, Ordering::Relaxed);
        let cmd = json!({ "id": id, "method": method, "params": params });
        ws.send(Message::Text(cmd.to_string().into()))
            .await
            .map_err(|e| ToolError::Execution(format!("CDP send failed: {e}")))?;

        while let Some(frame) = ws.next().await {
            let msg = frame.map_err(|e| ToolError::Execution(format!("CDP recv failed: {e}")))?;
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                Message::Close(_) => break,
                _ => continue,
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                let _ = ws.close(None).await;
                if let Some(err) = v.get("error") {
                    return Err(ToolError::Execution(format!("CDP error: {err}")));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        Err(ToolError::Execution(
            "CDP connection closed before response".to_string(),
        ))
    }

    pub async fn eval_value(ws_url: &str, expression: &str) -> Result<Value, ToolError> {
        let res = Self::call(
            ws_url,
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )
        .await?;
        Ok(res.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultLoginItem {
    pub handle: String,
    pub origin: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct VaultFile {
    logins: HashMap<String, VaultLoginItem>,
}

/// Origin-bound, model-blind browser vault (`tools/browser_vault_tool.py` parity).
#[derive(Debug, Clone)]
pub struct BrowserVault {
    path: PathBuf,
}

impl BrowserVault {
    pub fn open(saif_home: &Path) -> Self {
        Self {
            path: saif_home.join("browser_vault.json"),
        }
    }

    fn load(&self) -> VaultFile {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, file: &VaultFile) -> Result<(), ToolError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolError::Execution(e.to_string()))?;
        }
        let body =
            serde_json::to_string_pretty(file).map_err(|e| ToolError::Execution(e.to_string()))?;
        let tmp = self
            .path
            .with_extension(format!("json.tmp.{}", std::process::id()));
        fs::write(&tmp, &body).map_err(|e| ToolError::Execution(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(tmp, &self.path).map_err(|e| ToolError::Execution(e.to_string()))
    }

    pub fn save_login(
        &self,
        handle: &str,
        origin: &str,
        username: &str,
        password: &str,
    ) -> Result<(), ToolError> {
        let normalized_origin = normalize_origin(origin)?;
        let mut vf = self.load();
        vf.logins.insert(
            handle.to_string(),
            VaultLoginItem {
                handle: handle.to_string(),
                origin: normalized_origin,
                username: username.to_string(),
                password: password.to_string(),
            },
        );
        self.save(&vf)
    }

    /// Returns handles + non-secret metadata only; passwords are NEVER returned to the model.
    pub fn list_redacted(&self) -> Vec<Value> {
        let vf = self.load();
        let mut out: Vec<Value> = vf
            .logins
            .values()
            .map(|item| {
                json!({
                    "handle": item.handle,
                    "origin": item.origin,
                    "username": item.username,
                    "kind": "login"
                })
            })
            .collect();
        out.sort_by(|a, b| a["handle"].as_str().cmp(&b["handle"].as_str()));
        out
    }

    pub fn get_item(&self, handle: &str) -> Option<VaultLoginItem> {
        self.load().logins.get(handle).cloned()
    }
}

pub fn normalize_origin(raw: &str) -> Result<String, ToolError> {
    let parsed = url::Url::parse(raw.trim())
        .map_err(|e| ToolError::InvalidArgs(format!("invalid origin URL '{raw}': {e}")))?;
    Ok(parsed.origin().ascii_serialization())
}

pub async fn handle_browser_exec(
    args: &Value,
    ctx: &ToolContext,
    supervisors: &CdpSupervisorRegistry,
) -> Result<String, ToolError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;

    if action == "attach" {
        let ws_url = args
            .get("cdp_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'cdp_url'".to_string()))?;
        supervisors.attach(&ctx.session_id, ws_url);
        return Ok(json!({ "success": true, "attached": true, "cdp_url": ws_url }).to_string());
    }

    let ws_url = args
        .get("cdp_url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| supervisors.get(&ctx.session_id))
        .ok_or_else(|| {
            ToolError::Execution(
                "no active CDP browser session; pass action='attach' with cdp_url first"
                    .to_string(),
            )
        })?;
    supervisors.attach(&ctx.session_id, ws_url.clone());

    let raw_out = match action {
        "navigate" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'url'".to_string()))?;
            let res = CdpClient::call(&ws_url, "Page.navigate", json!({ "url": url })).await?;
            json!({ "success": true, "result": res }).to_string()
        }
        "eval" => {
            let expr = args
                .get("expression")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'expression'".to_string()))?;
            let val = CdpClient::eval_value(&ws_url, expr).await?;
            json!({ "success": true, "result": val }).to_string()
        }
        "click" => {
            let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let _ = CdpClient::call(
                &ws_url,
                "Input.dispatchMouseEvent",
                json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
            let _ = CdpClient::call(
                &ws_url,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
            json!({ "success": true, "clicked": [x, y] }).to_string()
        }
        "screenshot" => {
            let res = CdpClient::call(
                &ws_url,
                "Page.captureScreenshot",
                json!({ "format": "png" }),
            )
            .await?;
            let b64 = res.get("data").and_then(|v| v.as_str()).unwrap_or_default();
            json!({ "success": true, "bytes": b64.len(), "data": b64 }).to_string()
        }
        other => {
            return Err(ToolError::InvalidArgs(format!(
                "unknown browser_exec action '{other}'"
            )))
        }
    };

    Ok(supervisors.redactor.redact(&raw_out))
}

pub async fn handle_browser_vault(
    args: &Value,
    ctx: &ToolContext,
    supervisors: &CdpSupervisorRegistry,
) -> Result<String, ToolError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'action'".to_string()))?;
    let vault = BrowserVault::open(&ctx.profile.home_dir);

    match action {
        "list" => Ok(json!({ "success": true, "items": vault.list_redacted() }).to_string()),
        "save_login" => {
            let handle = args
                .get("handle")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'handle'".to_string()))?;
            let origin = args
                .get("origin")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'origin'".to_string()))?;
            let username = args
                .get("username")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'username'".to_string()))?;
            let password = args
                .get("password")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'password'".to_string()))?;
            vault.save_login(handle, origin, username, password)?;
            supervisors.redactor.register(password);
            Ok(json!({ "success": true, "handle": handle }).to_string())
        }
        "fill" => {
            let Some(ws_url) = supervisors.get(&ctx.session_id) else {
                // Fail-closed: NEVER invoke subprocess argv with credentials when supervisor WS is absent.
                return Ok(json!({
                    "success": false,
                    "error_type": "supervisor_required",
                    "error": "No supervised CDP WebSocket attached for this session; browser_vault fill refuses subprocess argv paths."
                })
                .to_string());
            };
            let handle = args
                .get("handle")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'handle'".to_string()))?;
            let Some(item) = vault.get_item(handle) else {
                return Ok(json!({
                    "success": false,
                    "error_type": "not_found",
                    "error": format!("vault handle '{handle}' not found")
                })
                .to_string());
            };
            let page_origin_val = CdpClient::eval_value(&ws_url, "window.location.origin").await?;
            let page_origin_raw = page_origin_val.as_str().unwrap_or("");
            let page_origin = normalize_origin(page_origin_raw).unwrap_or_default();
            if page_origin != item.origin {
                return Ok(json!({
                    "success": false,
                    "error_type": "origin_mismatch",
                    "error": format!(
                        "page origin '{page_origin}' does not match vault handle origin '{}'",
                        item.origin
                    )
                })
                .to_string());
            }

            // Inject exclusively over WebSocket Runtime.evaluate, re-asserting origin synchronously in JS.
            supervisors.redactor.register(&item.password);
            let pw_json =
                serde_json::to_string(&item.password).unwrap_or_else(|_| "\"\"".to_string());
            let expected_orig_json =
                serde_json::to_string(&item.origin).unwrap_or_else(|_| "\"\"".to_string());
            let js = format!(
                "(() => {{ if (window.location.origin !== {expected_orig_json}) return false; \
                 const el = document.querySelector('input[type=\"password\"], input[name*=\"pass\" i]'); \
                 if (!el) return false; el.value = {pw_json}; return true; }})()"
            );
            let _ = CdpClient::eval_value(&ws_url, &js).await?;
            Ok(json!({
                "success": true,
                "filled_fields": ["password"],
                "kind": "login",
                "origin": item.origin
            })
            .to_string())
        }
        other => Err(ToolError::InvalidArgs(format!(
            "unknown browser_vault action '{other}'"
        ))),
    }
}
