use saif_config::{ProfileScope, SaifConfig};
use saif_core::{AgentEvent, AgentRunner, TurnOptions};
use saif_persistence::{PersistenceStore, SqliteStore};
use saif_rpc_contract::{
    default_command_catalog, GatewayEvent, PromptSubmitParams, RpcRequest, RpcResponse,
    SessionCreateParams, StatusSnapshot,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// NDJSON JSON-RPC 2.0 backend (`tui_gateway/server.py` wire parity) over stdio.
pub async fn run_tui_gateway(
    profile: Arc<ProfileScope>,
    cfg: Arc<SaifConfig>,
    store: Option<SqliteStore>,
    agent: AgentRunner,
    cwd: PathBuf,
) -> anyhow::Result<()> {
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let emit_line = |writer: Arc<Mutex<tokio::io::Stdout>>, line: String| async move {
        let mut out = writer.lock().await;
        let _ = out.write_all(line.as_bytes()).await;
        let _ = out.write_all(b"\n").await;
        let _ = out.flush().await;
    };

    let ready = json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": GatewayEvent::GatewayReady {
            version: env!("CARGO_PKG_VERSION").to_string(),
            profile: profile.profile_name.clone(),
            skin_name: cfg.display.skin.clone(),
        }
    });
    emit_line(Arc::clone(&stdout), ready.to_string()).await;

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = RpcResponse::err(serde_json::Value::Null, -32700, e.to_string());
                emit_line(Arc::clone(&stdout), serde_json::to_string(&err_resp)?).await;
                continue;
            }
        };

        let resp = match req.method.as_str() {
            "commands.catalog" => {
                RpcResponse::ok(req.id, serde_json::to_value(default_command_catalog())?)
            }
            "complete.slash" => {
                let prefix = req
                    .params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim_start_matches('/');
                let matches: Vec<_> = default_command_catalog()
                    .into_iter()
                    .filter(|c| c.name.starts_with(prefix))
                    .collect();
                RpcResponse::ok(req.id, serde_json::to_value(matches)?)
            }
            "session.create" => {
                let _p: SessionCreateParams =
                    serde_json::from_value(req.params).unwrap_or(SessionCreateParams {
                        profile: None,
                        cwd: None,
                        model: None,
                    });
                let sid = format!("sess-{}", &Uuid::new_v4().to_string()[..12]);
                if let Some(db) = &store {
                    let _ = db
                        .create_session(
                            &sid,
                            "tui_gateway",
                            (!cfg.model.is_empty()).then_some(cfg.model.as_str()),
                            None,
                            Some(&cwd.to_string_lossy()),
                            Some(&profile.profile_name),
                        )
                        .await;
                }
                RpcResponse::ok(req.id, json!({ "session_id": sid }))
            }
            "session.list" => {
                let limit = req
                    .params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20) as usize;
                let sessions = if let Some(db) = &store {
                    db.list_sessions(limit).await.unwrap_or_default()
                } else {
                    Vec::new()
                };
                RpcResponse::ok(req.id, json!({ "sessions": sessions }))
            }
            "status.get" => {
                let session_count = store
                    .as_ref()
                    .and_then(|db| db.count_sessions().ok())
                    .unwrap_or(0);
                let snap = StatusSnapshot {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    profile: profile.profile_name.clone(),
                    model: cfg.model.clone(),
                    provider: cfg.provider.clone(),
                    session_count,
                    live_sessions: 1,
                };
                RpcResponse::ok(req.id, serde_json::to_value(snap)?)
            }
            "config.get" => RpcResponse::ok(req.id, serde_json::to_value(cfg.as_ref())?),
            "prompt.submit" => {
                let params: PromptSubmitParams = match serde_json::from_value(req.params) {
                    Ok(p) => p,
                    Err(e) => {
                        let r = RpcResponse::err(req.id, 4000, format!("invalid params: {e}"));
                        emit_line(Arc::clone(&stdout), serde_json::to_string(&r)?).await;
                        continue;
                    }
                };
                let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
                let out_clone = Arc::clone(&stdout);
                let sid_ev = params.session_id.clone();
                let pump = tokio::spawn(async move {
                    while let Some(ev) = ev_rx.recv().await {
                        let mapped = match ev {
                            AgentEvent::ContentDelta(delta) => Some(GatewayEvent::MessageDelta {
                                session_id: sid_ev.clone(),
                                delta,
                            }),
                            AgentEvent::ThinkingDelta(delta) => Some(GatewayEvent::ThinkingDelta {
                                session_id: sid_ev.clone(),
                                delta,
                            }),
                            AgentEvent::ToolStart { name, arguments } => {
                                Some(GatewayEvent::ToolStart {
                                    session_id: sid_ev.clone(),
                                    name,
                                    arguments,
                                })
                            }
                            AgentEvent::ToolComplete {
                                name,
                                output_preview,
                                ok,
                            } => Some(GatewayEvent::ToolComplete {
                                session_id: sid_ev.clone(),
                                name,
                                ok,
                                preview: output_preview,
                            }),
                            AgentEvent::FinalResponse(content) => {
                                Some(GatewayEvent::MessageComplete {
                                    session_id: sid_ev.clone(),
                                    content,
                                })
                            }
                            _ => None,
                        };
                        if let Some(ge) = mapped {
                            let note = json!({
                                "jsonrpc": "2.0",
                                "method": "event",
                                "params": ge
                            });
                            let mut out = out_clone.lock().await;
                            let _ = out.write_all(note.to_string().as_bytes()).await;
                            let _ = out.write_all(b"\n").await;
                            let _ = out.flush().await;
                        }
                    }
                });

                let opts = TurnOptions {
                    session_id: Some(params.session_id.clone()),
                    source: "tui_gateway".to_string(),
                    cwd: cwd.clone(),
                    ..TurnOptions::default()
                };
                let res = agent
                    .run_conversation(
                        &params.prompt,
                        None,
                        opts,
                        Some(ev_tx),
                        CancellationToken::new(),
                        None,
                    )
                    .await;
                let _ = pump.await;
                match res {
                    Ok(outcome) => RpcResponse::ok(
                        req.id,
                        json!({
                            "session_id": outcome.session_id,
                            "final_response": outcome.final_response
                        }),
                    ),
                    Err(e) => RpcResponse::err(req.id, -32000, e.to_string()),
                }
            }
            other => RpcResponse::err(req.id, -32601, format!("method not found: {other}")),
        };
        emit_line(Arc::clone(&stdout), serde_json::to_string(&resp)?).await;
    }

    Ok(())
}
