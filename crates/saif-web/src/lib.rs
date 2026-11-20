use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use saif_config::{atomic_yaml_write, load_config, ProfileScope, SaifConfig};
use saif_core::{AgentEvent, AgentRunner, TurnOptions};
use saif_cron::CronStore;
use saif_persistence::{KanbanStore, PersistenceStore, SqliteStore};
use saif_rpc_contract::{
    default_command_catalog, CronJobView, GatewayEvent, KanbanCardView, PromptSubmitParams,
    RpcRequest, RpcResponse, SkillSummaryView, StatusSnapshot,
};
use saif_skills::SkillStore;
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
    :root { --bg:#0d1117; --card:#161b22; --border:#30363d; --fg:#e6edf3; --accent:#58a6ff; --warn:#f0883e; }
    * { box-sizing: border-box; }
    body { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; background: var(--bg); color: var(--fg); margin: 0; padding: 16px; }
    header { display:flex; align-items:center; justify-content:space-between; border-bottom:1px solid var(--border); padding-bottom:12px; margin-bottom:16px; }
    nav { display:flex; gap:8px; flex-wrap:wrap; }
    button { background: var(--card); color: var(--fg); border:1px solid var(--border); padding:6px 12px; border-radius:6px; cursor:pointer; font:inherit; }
    button.active { border-color: var(--accent); color: var(--accent); }
    .panel { display:none; border:1px solid var(--border); border-radius:8px; padding:16px; background:var(--card); }
    .panel.active { display:block; }
    pre { background:#090c10; border:1px solid var(--border); padding:12px; border-radius:6px; overflow:auto; max-height:360px; }
    input, textarea { width:100%; background:#090c10; color:var(--fg); border:1px solid var(--border); border-radius:6px; padding:8px; font:inherit; margin:6px 0; }
    .grid { display:grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap:12px; }
    .card { border:1px solid var(--border); padding:10px; border-radius:6px; background:#0d1117; }
  </style>
</head>
<body>
  <header>
    <div><strong>Saif Agent</strong> <span style="color:var(--warn)">Pure-Rust WASM Control Plane</span></div>
    <nav>
      <button class="active" onclick="showTab('chat',this)">Chat</button>
      <button onclick="showTab('terminal',this)">Terminal (PTY)</button>
      <button onclick="showTab('kanban',this)">Kanban</button>
      <button onclick="showTab('cron',this)">Cron</button>
      <button onclick="showTab('skills',this)">Skills</button>
      <button onclick="showTab('config',this)">Config</button>
    </nav>
  </header>
  <section id="tab-chat" class="panel active">
    <pre id="chat-log">Connecting to /ws...</pre>
    <input id="chat-input" placeholder="Send prompt to Saif (/skill, /steer)..." />
    <button onclick="sendChat()">Send Prompt</button>
  </section>
  <section id="tab-terminal" class="panel">
    <pre id="pty-screen"></pre>
    <input id="pty-cmd" placeholder="Type shell input + Enter..." onkeydown="if(event.key==='Enter')sendPty()" />
    <button onclick="connectPty()">Reconnect PTY (80x24 RESIZE)</button>
  </section>
  <section id="tab-kanban" class="panel">
    <div style="display:flex;gap:8px;"><input id="kb-title" placeholder="New task title..." /><button onclick="createKanban()">Create Task</button></div>
    <div id="kb-list" class="grid"></div>
  </section>
  <section id="tab-cron" class="panel"><pre id="cron-list"></pre></section>
  <section id="tab-skills" class="panel"><pre id="skills-list"></pre></section>
  <section id="tab-config" class="panel">
    <input id="cfg-model" placeholder="Model override" />
    <input id="cfg-skin" placeholder="Skin (default, ares, mono, slate)" />
    <button onclick="saveConfig()">Save Config</button>
    <pre id="cfg-json"></pre>
  </section>
  <script>
    const q = new URLSearchParams(location.search);
    const tok = q.get('token') || '';
    const hdr = () => tok ? { 'Authorization': 'Bearer ' + tok, 'Content-Type': 'application/json' } : { 'Content-Type': 'application/json' };
    function showTab(name, btn) {
      document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
      document.querySelectorAll('nav button').forEach(b => b.classList.remove('active'));
      document.getElementById('tab-' + name).classList.add('active');
      if (btn) btn.classList.add('active');
      if (name === 'kanban') loadKanban();
      if (name === 'cron') loadCron();
      if (name === 'skills') loadSkills();
      if (name === 'config') loadConfig();
    }
    let ws, pty;
    function connectWs() {
      const proto = location.protocol === 'https:' ? 'wss://' : 'ws://';
      ws = new WebSocket(proto + location.host + '/ws?token=' + encodeURIComponent(tok));
      ws.onmessage = e => { document.getElementById('chat-log').textContent += '\n' + e.data; };
    }
    function sendChat() {
      const el = document.getElementById('chat-input');
      if (ws && el.value) ws.send(JSON.stringify({ jsonrpc:'2.0', id: Date.now(), method:'prompt.submit', params:{ session_id:'web-main', prompt:el.value, steer:false } }));
      el.value = '';
    }
    function connectPty() {
      const proto = location.protocol === 'https:' ? 'wss://' : 'ws://';
      pty = new WebSocket(proto + location.host + '/api/pty?token=' + encodeURIComponent(tok));
      pty.onopen = () => pty.send('\x1b[RESIZE:100;30]');
      pty.onmessage = async e => { const t = e.data instanceof Blob ? await e.data.text() : e.data; document.getElementById('pty-screen').textContent += t; };
    }
    function sendPty() { const el = document.getElementById('pty-cmd'); if (pty) pty.send(el.value + '\n'); el.value = ''; }
    async function loadKanban() {
      const r = await fetch('/api/kanban?token=' + encodeURIComponent(tok), { headers: hdr() }).then(r => r.json());
      document.getElementById('kb-list').innerHTML = (r.tasks||[]).map(t => `<div class="card"><b>${t.title}</b><div>${t.status}</div></div>`).join('');
    }
    async function createKanban() {
      const title = document.getElementById('kb-title').value;
      if (!title) return;
      await fetch('/api/kanban?token=' + encodeURIComponent(tok), { method:'POST', headers: hdr(), body: JSON.stringify({ action:'create', title }) });
      document.getElementById('kb-title').value = '';
      loadKanban();
    }
    async function loadCron() { document.getElementById('cron-list').textContent = JSON.stringify(await fetch('/api/cron?token=' + encodeURIComponent(tok), { headers: hdr() }).then(r => r.json()), null, 2); }
    async function loadSkills() { document.getElementById('skills-list').textContent = JSON.stringify(await fetch('/api/skills?token=' + encodeURIComponent(tok), { headers: hdr() }).then(r => r.json()), null, 2); }
    async function loadConfig() { document.getElementById('cfg-json').textContent = JSON.stringify(await fetch('/api/config?token=' + encodeURIComponent(tok), { headers: hdr() }).then(r => r.json()), null, 2); }
    async function saveConfig() {
      const body = { model: document.getElementById('cfg-model').value || undefined, skin: document.getElementById('cfg-skin').value || undefined };
      await fetch('/api/config?token=' + encodeURIComponent(tok), { method:'PUT', headers: hdr(), body: JSON.stringify(body) });
      loadConfig();
    }
    connectWs();
  </script>
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
        .route(
            "/api/config",
            get(config_handler).put(update_config_handler),
        )
        .route(
            "/api/kanban",
            get(kanban_list_handler).post(kanban_action_handler),
        )
        .route("/api/cron", get(cron_list_handler))
        .route("/api/skills", get(skills_list_handler))
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
    let live_cfg = load_config(&state.profile.home_dir).unwrap_or_else(|_| (*state.config).clone());
    Ok(Json(serde_json::to_value(live_cfg).unwrap_or(Value::Null)))
}

#[derive(Debug, Deserialize)]
struct ConfigPatchRequest {
    model: Option<String>,
    provider: Option<String>,
    skin: Option<String>,
    language: Option<String>,
    terminal_backend: Option<String>,
}

async fn update_config_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(patch): Json<ConfigPatchRequest>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut cfg = load_config(&state.profile.home_dir).unwrap_or_else(|_| (*state.config).clone());
    if let Some(m) = patch.model.filter(|s| !s.trim().is_empty()) {
        cfg.model = m;
    }
    if let Some(p) = patch.provider.filter(|s| !s.trim().is_empty()) {
        cfg.provider = p;
    }
    if let Some(s) = patch.skin.filter(|s| !s.trim().is_empty()) {
        cfg.display.skin = s;
    }
    if let Some(l) = patch.language.filter(|s| !s.trim().is_empty()) {
        cfg.display.language = l;
    }
    if let Some(b) = patch.terminal_backend.filter(|s| !s.trim().is_empty()) {
        cfg.terminal.backend = b;
    }
    atomic_yaml_write(&state.profile.config_path(), &cfg)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "updated": true, "config": cfg })))
}

async fn kanban_list_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let kb = KanbanStore::open(&state.profile.kanban_db_path())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tasks = kb
        .list_tasks(None)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let cards: Vec<KanbanCardView> = tasks
        .into_iter()
        .map(|t| KanbanCardView {
            id: t.id,
            title: t.title,
            status: format!("{:?}", t.status).to_ascii_lowercase(),
            assignee: t.assignee,
            priority: t.priority,
        })
        .collect();
    Ok(Json(json!({ "tasks": cards })))
}

#[derive(Debug, Deserialize)]
struct KanbanPostRequest {
    action: String,
    title: Option<String>,
    body: Option<String>,
    assignee: Option<String>,
    task_id: Option<String>,
    summary: Option<String>,
}

async fn kanban_action_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(req): Json<KanbanPostRequest>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let kb = KanbanStore::open(&state.profile.kanban_db_path())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match req.action.as_str() {
        "create" => {
            let title = req.title.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
            let task = kb
                .create_task(title, req.body.as_deref(), req.assignee.as_deref(), 0, &[])
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(json!({ "success": true, "task": task })))
        }
        "complete" => {
            let tid = req.task_id.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
            kb.complete_task(tid, req.summary.as_deref().unwrap_or("done"))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(json!({ "success": true, "task_id": tid })))
        }
        "unblock" => {
            let tid = req.task_id.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
            kb.unblock_task(tid)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(json!({ "success": true, "task_id": tid })))
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

async fn cron_list_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let store = CronStore::new(state.profile.cron_dir());
    let jobs = store.load_jobs().unwrap_or_default();
    let views: Vec<CronJobView> = jobs
        .into_iter()
        .map(|j| CronJobView {
            id: j.id,
            name: j.name,
            enabled: j.enabled,
            next_run_at: j.next_run_at,
            last_status: j.last_status,
        })
        .collect();
    Ok(Json(json!({ "jobs": views })))
}

async fn skills_list_handler(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&headers, &q, &state.session_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let store = SkillStore::new(state.profile.skills_dir(), &state.config.skills.disabled);
    let views: Vec<SkillSummaryView> = store
        .list_skills()
        .into_iter()
        .map(|s| SkillSummaryView {
            name: s.name,
            category: s.category,
            description: s.prompt_description,
        })
        .collect();
    Ok(Json(json!({ "skills": views })))
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
