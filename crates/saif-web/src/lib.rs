use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use saif_config::{ProfileScope, SaifConfig};
use saif_core::{AgentEvent, AgentRunner, TurnOptions};
use saif_persistence::{PersistenceStore, SqliteStore};
use saif_rpc_contract::{
    default_command_catalog, GatewayEvent, PromptSubmitParams, RpcRequest, RpcResponse,
    StatusSnapshot,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;
use uuid::Uuid;

#[derive(Clone)]
pub struct WebState {
    pub session_token: String,
    pub headless: bool,
    pub profile: Arc<ProfileScope>,
    pub config: Arc<SaifConfig>,
    pub store: Option<SqliteStore>,
    pub agent: AgentRunner,
    pub pty_program: String,
    pub pty_args: Vec<String>,
    pub cwd: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

const DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Saif Agent Dashboard</title>
  <style>
    body { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; background: #0d1117; color: #e6edf3; margin: 0; padding: 24px; }
    .card { border: 1px solid #30363d; border-radius: 8px; padding: 16px; margin-bottom: 16px; background: #161b22; }
    h1 { color: #58a6ff; margin-top: 0; }
    code { color: #f0883e; }
  </style>
</head>
<body>
  <div class="card">
    <h1>Saif Agent — 100% Pure Rust Dashboard</h1>
    <p>Backend REST (<code>/api/status</code>, <code>/api/sessions</code>), JSON-RPC WebSocket (<code>/ws</code>), and POSIX PTY bridge (<code>/api/pty</code>) are active.</p>
    <p>Frontend WASM bundle: <code>apps/saif-web-ui</code> (<code>wasm32-unknown-unknown</code>).</p>
  </div>
</body>
</html>"#;

fn check_auth(headers: &HeaderMap, query: &TokenQuery, expected: &str) -> bool {
    if let Some(q) = &query.token {
        if q == expected {
            return true;
        }
    }
    if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        if let Some(bearer) = auth.strip_prefix("Bearer ") {
            return bearer.trim() == expected;
        }
    }
    false
}

pub fn build_web_router(state: WebState) -> Router {
    let shared = Arc::new(state);
    Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .route("/api/status", get(status_handler))
        .route("/api/sessions", get(sessions_handler))
        .route("/api/config", get(config_handler))
        .route("/ws", get(rpc_ws_handler))
        .route("/api/pty", get(pty_ws_handler))
        .with_state(shared)
}

pub async fn run_web_server(addr: SocketAddr, state: WebState) -> anyhow::Result<()> {
    let token = state.session_token.clone();
    let headless = state.headless;
    let router = build_web_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("saif-web listening on http://{addr} (headless={headless}, token={token})");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn index_handler(State(state): State<Arc<WebState>>) -> Response {
    if state.headless || std::env::var("SAIF_SERVE_HEADLESS").as_deref() == Ok("1") {
        return (StatusCode::NOT_FOUND, "Headless serve mode: SPA disabled").into_response();
    }
    Html(DASHBOARD_HTML).into_response()
}

async fn health_handler() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn status_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Result<Json<StatusSnapshot>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let session_count = state
        .store
        .as_ref()
        .and_then(|db| db.count_sessions().ok())
        .unwrap_or(0);
    Ok(Json(StatusSnapshot {
        version: env!("CARGO_PKG_VERSION").to_string(),
        profile: state.profile.profile_name.clone(),
        model: state.config.model.clone(),
        provider: state.config.provider.clone(),
        session_count,
        live_sessions: 1,
    }))
}

async fn sessions_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let list = if let Some(db) = &state.store {
        db.list_sessions(25).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(Json(json!({ "sessions": list })))
}

async fn config_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(
        serde_json::to_value(state.config.as_ref()).unwrap_or(Value::Null),
    ))
}

async fn rpc_ws_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(ws.on_upgrade(move |socket| handle_rpc_ws(socket, state)))
}

async fn handle_rpc_ws(socket: WebSocket, state: Arc<WebState>) {
    let (mut sender, mut receiver) = socket.split();
    let ready = json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": GatewayEvent::GatewayReady {
            version: env!("CARGO_PKG_VERSION").to_string(),
            profile: state.profile.profile_name.clone(),
            skin_name: state.config.display.skin.clone(),
        }
    });
    let _ = sender.send(Message::Text(ready.to_string().into())).await;

    while let Some(Ok(msg)) = receiver.next().await {
        let Message::Text(raw) = msg else {
            continue;
        };
        let Ok(req) = serde_json::from_str::<RpcRequest>(&raw) else {
            continue;
        };
        let resp = match req.method.as_str() {
            "commands.catalog" => RpcResponse::ok(
                req.id,
                serde_json::to_value(default_command_catalog()).unwrap_or(Value::Null),
            ),
            "prompt.submit" => {
                let params: PromptSubmitParams =
                    serde_json::from_value(req.params).unwrap_or(PromptSubmitParams {
                        session_id: format!("ws-{}", &Uuid::new_v4().to_string()[..8]),
                        prompt: String::new(),
                        steer: false,
                    });
                let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
                let opts = TurnOptions {
                    session_id: Some(params.session_id.clone()),
                    source: "web".to_string(),
                    cwd: state.cwd.clone(),
                    ..TurnOptions::default()
                };
                let outcome = state
                    .agent
                    .run_conversation(
                        &params.prompt,
                        None,
                        opts,
                        Some(ev_tx),
                        CancellationToken::new(),
                        None,
                    )
                    .await;
                while let Ok(ev) = ev_rx.try_recv() {
                    if let AgentEvent::ContentDelta(delta) = ev {
                        let note = json!({
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": GatewayEvent::MessageDelta {
                                session_id: params.session_id.clone(),
                                delta,
                            }
                        });
                        let _ = sender.send(Message::Text(note.to_string().into())).await;
                    }
                }
                match outcome {
                    Ok(out) => RpcResponse::ok(
                        req.id,
                        json!({
                            "session_id": out.session_id,
                            "final_response": out.final_response
                        }),
                    ),
                    Err(e) => RpcResponse::err(req.id, -32000, e.to_string()),
                }
            }
            other => RpcResponse::err(req.id, -32601, format!("method not found: {other}")),
        };
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&resp).unwrap_or_default().into(),
            ))
            .await;
    }
}

async fn pty_ws_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(ws.on_upgrade(move |socket| handle_pty_ws(socket, state)))
}

async fn handle_pty_ws(socket: WebSocket, state: Arc<WebState>) {
    let pty_system = native_pty_system();
    let Ok(pair) = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) else {
        return;
    };

    let mut cmd = CommandBuilder::new(&state.pty_program);
    for arg in &state.pty_args {
        cmd.arg(arg);
    }
    cmd.cwd(&state.cwd);
    let Ok(mut child) = pair.slave.spawn_command(cmd) else {
        return;
    };
    drop(pair.slave);

    let Ok(mut reader) = pair.master.try_clone_reader() else {
        return;
    };
    let Ok(mut writer) = pair.master.take_writer() else {
        return;
    };

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            if out_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let write_task = tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            if ws_tx.send(Message::Binary(chunk.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(txt) => {
                if let Some(dims) = parse_pty_resize(&txt) {
                    let _ = pair.master.resize(PtySize {
                        rows: dims.1,
                        cols: dims.0,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                } else {
                    let _ = writer.write_all(txt.as_bytes());
                    let _ = writer.flush();
                }
            }
            Message::Binary(bytes) => {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let _ = child.kill();
    write_task.abort();
}

/// Parses Hermes `\x1b[RESIZE:<cols>;<rows>]` control frames (`web/AGENTS.md`).
pub fn parse_pty_resize(frame: &str) -> Option<(u16, u16)> {
    let body = frame.strip_prefix("\x1b[RESIZE:")?.strip_suffix(']')?;
    let (cols_s, rows_s) = body.split_once(';')?;
    let cols = cols_s.trim().parse::<u16>().ok()?;
    let rows = rows_s.trim().parse::<u16>().ok()?;
    Some((cols, rows))
}
