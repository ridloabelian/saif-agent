use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use saif_config::{ProfileScope, SaifConfig, SecretScope};
use saif_core::{run_batch_dataset, AgentRunner};
use saif_gateway::{
    build_gateway_router, builtin_platform_specs, converge_multiplex_migration,
    render_service_unit, GatewayHttpState, GatewayMultiplexer, ServiceKind,
};
use saif_persistence::SqliteStore;
use saif_plugins::HookRegistry;
use saif_providers::{
    is_bedrock_xai_grok_no_sampling, sign_aws_sigv4_headers, ApiMode, BedrockConverseTransport,
    ChatMessage, CodexResponsesTransport, CredStatus, CredentialPool, LlmRequest, LlmTransport,
    PoolStrategy, ResolvedRoute,
};
use saif_rpc_contract::{CronJobView, DashboardTab, KanbanCardView, SkillSummaryView};
use saif_tools::{
    enforce_byte_limit, register_all_builtin_tools_with_cdp, truncate_stdout_bytes,
    wrap_pcm_as_wav, CdpSupervisorRegistry, DaytonaEnv, ExecutionEnvironment, ModalEnv,
    ProcessRegistry, SnapshotLedger, TodoStore, ToolContext, ToolRegistry, VercelSandboxEnv,
    TTS_RESPONSE_BODY_LIMIT_BYTES,
};
use saif_web::{build_web_router, WebState};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message as WsMsg;

fn sample_route(api_mode: ApiMode, base_url: String, provider_id: &str) -> ResolvedRoute {
    ResolvedRoute {
        provider_id: provider_id.to_string(),
        base_url,
        api_mode,
        model: "gpt-5.3-codex".to_string(),
        api_key: Some("test-key".to_string()),
        omit_temperature: true,
        supports_prompt_caching: true,
        multiplex_active: false,
        pooled_credential_id: None,
        saif_home: None,
        aws_region: Some("us-east-1".to_string()),
        aws_access_key_id: Some("AKIAEXAMPLE".to_string()),
        aws_secret_access_key: Some("SECRETEXAMPLE".to_string()),
        aws_session_token: None,
        aws_profile: None,
    }
}

#[tokio::test]
async fn codex_responses_sse_and_tool_alias_roundtrip() {
    let seen_payloads = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let seen_srv = Arc::clone(&seen_payloads);
    let app = Router::new().route(
        "/v1/responses",
        post(move |Json(req): Json<Value>| {
            let seen_srv = Arc::clone(&seen_srv);
            async move {
                seen_srv.lock().await.push(req);
                let sse = concat!(
                    "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"thinking...\"}\n\n",
                    "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"fc_1\",\"name\":\"hermes_tool_search\",\"arguments\":\"{\\\"query\\\":\\\"cron\\\"}\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Done searching.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":7}}}\n\n"
                );
                ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], sse)
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let transport = CodexResponsesTransport::new();
    let route = sample_route(
        ApiMode::CodexResponses,
        format!("http://{addr}/v1"),
        "openai-codex",
    );
    let req = LlmRequest {
        model: "gpt-5.3-codex".to_string(),
        messages: vec![
            ChatMessage::system("sys"),
            ChatMessage::user("search cron tools"),
        ],
        tools: vec![saif_providers::ToolDefinition {
            name: "tool_search".to_string(),
            description: "Search tools".to_string(),
            parameters: json!({ "type": "object" }),
        }],
        temperature: None,
        max_tokens: None,
        reasoning_effort: Some("medium".to_string()),
        stream: true,
        session_id: Some("cron_daily_digest_20261120_090000".to_string()),
    };

    let resp = transport.complete(&route, &req, None).await.unwrap();
    assert_eq!(resp.content, "Done searching.");
    assert_eq!(resp.reasoning.as_deref(), Some("thinking..."));
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "tool_search");

    let recorded = seen_payloads.lock().await;
    assert_eq!(recorded[0]["prompt_cache_key"], "cron_daily_digest");
    assert_eq!(recorded[0]["tools"][0]["name"], "hermes_tool_search");
}

#[tokio::test]
async fn bedrock_sigv4_fail_closed_multiplex_and_converse() {
    assert!(is_bedrock_xai_grok_no_sampling("us.xai.grok-3"));
    assert!(!is_bedrock_xai_grok_no_sampling(
        "us.anthropic.claude-sonnet-4"
    ));

    // SigV4 deterministic signing check
    let s1 = sign_aws_sigv4_headers(
        "POST",
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/claude-3/converse",
        "us-east-1",
        "bedrock",
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        b"{}",
        "20261120T090000Z",
    )
    .unwrap();
    assert!(s1.authorization.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20261120/us-east-1/bedrock/aws4_request"
    ));

    // Fail-closed under multiplex when AWS creds absent
    let bedrock = BedrockConverseTransport::new();
    let mut multiplex_route = sample_route(
        ApiMode::Bedrock,
        "http://127.0.0.1:1".to_string(),
        "bedrock",
    );
    multiplex_route.multiplex_active = true;
    multiplex_route.aws_access_key_id = None;
    multiplex_route.aws_secret_access_key = None;
    let err = bedrock
        .complete(&multiplex_route, &LlmRequest::default(), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("authentication error"));

    // Mock Converse endpoint round-trip
    let app = Router::new().route(
        "/model/{model}/converse",
        post(|| async {
            Json(json!({
                "output": { "message": { "role": "assistant", "content": [{ "text": "Bedrock ok" }] } },
                "stopReason": "end_turn",
                "usage": { "inputTokens": 20, "outputTokens": 5 }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let route = sample_route(ApiMode::Bedrock, format!("http://{addr}"), "bedrock");
    let req = LlmRequest {
        model: "us.xai.grok-3".to_string(),
        messages: vec![ChatMessage::user("hi")],
        ..LlmRequest::default()
    };
    let res = bedrock.complete(&route, &req, None).await.unwrap();
    assert_eq!(res.content, "Bedrock ok");
}

#[test]
fn credential_pool_strategies_cooldowns_and_terminal_dead() {
    let tmp = TempDir::new().unwrap();
    let mut pool = CredentialPool::open(tmp.path());
    pool.set_strategy(PoolStrategy::RoundRobin);
    pool.add_credential("openai", "k1", "sk-1", "manual");
    pool.add_credential("openai", "k2", "sk-2", "manual");

    let c1 = pool.select("openai", 1000).unwrap();
    let c2 = pool.select("openai", 1000).unwrap();
    assert_ne!(c1.id, c2.id);

    // Exhaust k1 (429 -> 3600s cooldown with 2 creds), k2 terminal dead
    pool.record_failure("openai", &c1.id, Some(429), None, 1000);
    pool.record_failure("openai", &c2.id, Some(401), Some("token_revoked"), 1000);
    pool.save().unwrap();

    let reloaded = CredentialPool::open(tmp.path());
    let entries = reloaded.list(Some("openai"));
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.status == CredStatus::Dead));
    assert!(entries.iter().any(|e| e.status == CredStatus::Exhausted));
}

#[tokio::test]
async fn batch_dataset_parallel_resume_and_sharegpt_stats() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|Json(req): Json<Value>| async move {
            let msg = req["messages"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or("");
            Json(json!({
                "choices": [{ "index": 0, "message": { "role": "assistant", "content": format!("ans:{msg}") }, "finish_reason": "stop" }],
                "usage": { "prompt_tokens": 5, "completion_tokens": 2 }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let tmp = TempDir::new().unwrap();
    let profile = Arc::new(ProfileScope::for_root(tmp.path().to_path_buf(), false));
    profile.ensure_layout().unwrap();
    let cfg = Arc::new(SaifConfig {
        provider: "custom".to_string(),
        model: "m".to_string(),
        base_url: Some(format!("http://{addr}/v1")),
        ..SaifConfig::default()
    });
    let reg = ToolRegistry::new();
    let runner = AgentRunner::new(profile, cfg, reg, None, HookRegistry::new());

    let in_path = tmp.path().join("in.jsonl");
    let out_path = tmp.path().join("out.jsonl");
    fs::write(
        &in_path,
        "{\"id\":\"r1\",\"prompt\":\"first\"}\n{\"id\":\"r2\",\"prompt\":\"second\"}\n",
    )
    .unwrap();

    let recs = run_batch_dataset(
        runner.clone(),
        &in_path,
        &out_path,
        2,
        false,
        tmp.path().to_path_buf(),
    )
    .await
    .unwrap();
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].conversations[1].from, "human");
    assert_eq!(recs[0].conversations[2].from, "gpt");

    // Append r3 and verify --resume preserves r1/r2 and adds r3
    fs::write(
        &in_path,
        "{\"id\":\"r1\",\"prompt\":\"first\"}\n{\"id\":\"r2\",\"prompt\":\"second\"}\n{\"id\":\"r3\",\"prompt\":\"third\"}\n",
    )
    .unwrap();
    let resumed = run_batch_dataset(
        runner,
        &in_path,
        &out_path,
        2,
        true,
        tmp.path().to_path_buf(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.len(), 3);
}

#[tokio::test]
async fn cloud_sandboxes_cdp_vault_and_execute_code_e2e() {
    let vercel_attempts = Arc::new(AtomicUsize::new(0));
    let v_srv = Arc::clone(&vercel_attempts);

    let mock_cloud = Router::new()
        .route(
            "/v1/sandboxes/exec",
            post(|| async {
                Json(json!({ "exit_code": 0, "stdout": "modal-ok", "stderr": "", "snapshot_id": "sn_1" }))
            }),
        )
        .route(
            "/v1/toolbox/exec",
            post(|| async { Json(json!({ "exit_code": 0, "stdout": "daytona-ok", "stderr": "" })) }),
        )
        .route(
            "/v1/sandboxes/run",
            post(move || {
                let v_srv = Arc::clone(&v_srv);
                async move {
                    if v_srv.fetch_add(1, Ordering::SeqCst) == 0 {
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(json!({ "error": "retry me" })),
                        )
                    } else {
                        (
                            StatusCode::OK,
                            Json(json!({ "exit_code": 0, "stdout": "vercel-ok", "snapshot_id": "vs_1" })),
                        )
                    }
                }
            }),
        );

    let cloud_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(cloud_listener, mock_cloud).await;
    });

    let tmp = TempDir::new().unwrap();
    let modal = ModalEnv::new(
        format!("http://{cloud_addr}"),
        None,
        None,
        "img",
        "t1",
        tmp.path().to_path_buf(),
    );
    assert_eq!(
        modal
            .execute(
                "echo ok",
                tmp.path(),
                &BTreeMap::new(),
                Duration::from_secs(5)
            )
            .await
            .unwrap()
            .stdout,
        "modal-ok"
    );
    assert_eq!(
        SnapshotLedger::modal(tmp.path()).get_restore_candidate("t1"),
        Some(("sn_1".to_string(), false))
    );

    let daytona = DaytonaEnv::new(format!("http://{cloud_addr}"), None, "img", "t1", 99_999);
    assert_eq!(daytona.disk_gib, 10);
    assert_eq!(
        daytona
            .execute(
                "echo ok",
                tmp.path(),
                &BTreeMap::new(),
                Duration::from_secs(5)
            )
            .await
            .unwrap()
            .stdout,
        "daytona-ok"
    );

    let vercel = VercelSandboxEnv::new(
        format!("http://{cloud_addr}"),
        None,
        "t1",
        tmp.path().to_path_buf(),
    );
    assert_eq!(
        vercel
            .execute(
                "echo ok",
                tmp.path(),
                &BTreeMap::new(),
                Duration::from_secs(5)
            )
            .await
            .unwrap()
            .stdout,
        "vercel-ok"
    );

    // Mock CDP WebSocket server verifying origin binding & secret redaction
    let cdp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cdp_addr = cdp_listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = cdp_listener.accept().await {
            tokio::spawn(async move {
                let mut ws = accept_async(stream).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    if let WsMsg::Text(txt) = msg {
                        let v: Value = serde_json::from_str(&txt).unwrap();
                        let id = v["id"].as_u64().unwrap();
                        let expr = v["params"]["expression"].as_str().unwrap_or("");
                        let val = if expr == "window.location.origin" {
                            json!("https://example.com")
                        } else {
                            json!(true)
                        };
                        let reply = json!({ "id": id, "result": { "result": { "value": val } } });
                        let _ = ws.send(WsMsg::Text(reply.to_string().into())).await;
                    }
                }
            });
        }
    });

    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();
    fs::write(ws_dir.join("note.txt"), "rpc-read-success").unwrap();
    let profile = Arc::new(ProfileScope::for_root(tmp.path().to_path_buf(), false));
    profile.ensure_layout().unwrap();
    let cfg = Arc::new(SaifConfig::default());
    let reg = ToolRegistry::new();
    let cdp_reg = CdpSupervisorRegistry::new();
    register_all_builtin_tools_with_cdp(
        &reg,
        ProcessRegistry::new(),
        TodoStore::default(),
        cdp_reg.clone(),
    );

    let ctx = ToolContext {
        session_id: "sess-cdp-1".to_string(),
        cwd: ws_dir,
        profile,
        config: cfg,
        store: None,
        kanban: None,
        task_id: None,
        spawn_depth: 0,
        is_delegated_child: false,
    };

    // Save credential + test missing supervisor fail-closed error
    reg.dispatch(
        "browser_vault",
        json!({ "action": "save_login", "handle": "gh", "origin": "https://example.com/login", "username": "alice", "password": "SuperSecretPassword123!" }),
        &ctx,
    )
    .await
    .unwrap();
    let listed = reg
        .dispatch("browser_vault", json!({ "action": "list" }), &ctx)
        .await
        .unwrap();
    assert!(!listed.contains("SuperSecretPassword123!"));

    let no_sup = reg
        .dispatch(
            "browser_vault",
            json!({ "action": "fill", "handle": "gh" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(no_sup.contains("supervisor_required"));

    // Attach CDP and fill successfully
    cdp_reg.attach("sess-cdp-1", format!("ws://{cdp_addr}"));
    let ok_fill = reg
        .dispatch(
            "browser_vault",
            json!({ "action": "fill", "handle": "gh" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(ok_fill.contains("\"success\":true"));
    assert!(!ok_fill.contains("SuperSecretPassword123!"));

    // execute_code running Python RPC into saif_tools.read_file
    let exec_res = reg
        .dispatch(
            "execute_code",
            json!({ "code": "import saif_tools\nprint(saif_tools.read_file('note.txt')['content'])" }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(exec_res.contains("rpc-read-success"));

    // Stdout > 50KB truncation writes cache/exec spill file
    let big = "Z".repeat(75_000);
    let (_trunc, meta) = truncate_stdout_bytes(&big, Some(tmp.path()));
    let spill = meta["stdout_spill_path"].as_str().unwrap();
    assert!(PathBuf::from(spill).is_file());
}

#[tokio::test]
async fn gateway_30_platforms_multiplex_web_and_media_e2e() {
    assert_eq!(builtin_platform_specs().len(), 30);

    let llm_srv = Router::new().route(
        "/v1/chat/completions",
        post(|Json(req): Json<Value>| async move {
            let txt = req["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_str()
                .unwrap_or("");
            Json(json!({
                "choices": [{ "index": 0, "message": { "role": "assistant", "content": format!("gw:{txt}") }, "finish_reason": "stop" }],
                "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
            }))
        }),
    );
    let llm_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let llm_addr = llm_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(llm_listener, llm_srv).await;
    });

    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let p_default = Arc::new(ProfileScope::for_root(root.clone(), true));
    let mut sec_coder = BTreeMap::new();
    sec_coder.insert("DISCORD_BOT_TOKEN".into(), "tok-coder".into());
    let p_coder = Arc::new(ProfileScope {
        profile_name: "coder".into(),
        home_dir: root.join("profiles/coder"),
        root_dir: root.clone(),
        is_default: false,
        secrets: SecretScope::from_map(sec_coder, false, true),
    });
    p_default.ensure_layout().unwrap();
    p_coder.ensure_layout().unwrap();

    let cfg = Arc::new(SaifConfig {
        provider: "custom".into(),
        model: "m".into(),
        base_url: Some(format!("http://{llm_addr}/v1")),
        ..SaifConfig::default()
    });
    let runner_d = AgentRunner::new(
        Arc::clone(&p_default),
        Arc::clone(&cfg),
        ToolRegistry::new(),
        None,
        HookRegistry::new(),
    );
    let runner_c = AgentRunner::new(
        Arc::clone(&p_coder),
        Arc::clone(&cfg),
        ToolRegistry::new(),
        None,
        HookRegistry::new(),
    );

    let mut mux = GatewayMultiplexer::new("default");
    mux.add_slot(Arc::clone(&p_default), Arc::clone(&cfg), runner_d.clone());
    mux.add_slot(Arc::clone(&p_coder), Arc::clone(&cfg), runner_c);
    assert!(mux.preflight_credential_conflicts().is_ok());

    let gw_app = build_gateway_router(GatewayHttpState {
        agent: runner_d.clone(),
        cwd: root.clone(),
        multiplexer: Some(Arc::new(mux)),
    });
    let gw_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(gw_listener, gw_app).await;
    });

    let client = reqwest::Client::new();
    let plat_resp: Value = client
        .get(format!("http://{gw_addr}/v1/platforms"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(plat_resp["platforms"].as_array().unwrap().len(), 30);

    let discord_ack: Value = client
        .post(format!("http://{gw_addr}/webhooks/discord"))
        .json(&json!({ "profile": "coder", "channel_id": "ch_1", "author": { "id": "u_9" }, "content": "hello discord" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        discord_ack["identity"]["session_key"],
        "agent:coder:discord:ch_1"
    );

    let a2a_ack: Value = client
        .post(format!("http://{gw_addr}/webhooks/a2a"))
        .json(&json!({ "id": "task-1", "method": "message/send", "params": { "message": { "parts": [{ "text": "ping a2a" }] } } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a2a_ack["chunks"][0], "gw:ping a2a");

    let manifest = converge_multiplex_migration(&root).unwrap();
    assert_eq!(manifest.status, "converged");
    let unit = render_service_unit(
        ServiceKind::Systemd,
        &PathBuf::from("/usr/local/bin/saif"),
        &root,
        "127.0.0.1:8788".parse::<SocketAddr>().unwrap(),
    );
    assert!(unit.contains("SAIF_HOME="));

    // Wave 6: Web REST routes + WAV header + byte cap + RPC views
    let store = SqliteStore::open(&p_default.state_db_path()).unwrap();
    let web_app = build_web_router(WebState {
        session_token: "tok-web".into(),
        headless: false,
        profile: Arc::clone(&p_default),
        config: Arc::clone(&cfg),
        store: Some(store),
        agent: runner_d,
        pty_program: "sh".into(),
        pty_args: vec![],
        cwd: root.clone(),
    });
    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let web_addr = web_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(web_listener, web_app).await;
    });

    client
        .post(format!("http://{web_addr}/api/kanban?token=tok-web"))
        .json(&json!({ "action": "create", "title": "Ship Wave 6" }))
        .send()
        .await
        .unwrap();
    let kb_list: Value = client
        .get(format!("http://{web_addr}/api/kanban?token=tok-web"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(kb_list["tasks"][0]["title"], "Ship Wave 6");

    client
        .put(format!("http://{web_addr}/api/config?token=tok-web"))
        .json(&json!({ "skin": "ares" }))
        .send()
        .await
        .unwrap();
    let cfg_get: Value = client
        .get(format!("http://{web_addr}/api/config?token=tok-web"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_get["display"]["skin"], "ares");

    let wav = wrap_pcm_as_wav(&[0u8; 32], 24_000, 1, 16);
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(wav.len(), 76);
    assert!(enforce_byte_limit(
        &vec![0u8; TTS_RESPONSE_BODY_LIMIT_BYTES + 1],
        TTS_RESPONSE_BODY_LIMIT_BYTES,
        "tts"
    )
    .is_err());

    let _views = (
        DashboardTab::Kanban,
        KanbanCardView {
            id: "1".into(),
            title: "t".into(),
            status: "ready".into(),
            assignee: None,
            priority: 0,
        },
        CronJobView {
            id: "1".into(),
            name: "c".into(),
            enabled: true,
            next_run_at: None,
            last_status: None,
        },
        SkillSummaryView {
            name: "s".into(),
            category: None,
            description: "d".into(),
        },
    );
}
