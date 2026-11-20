use axum::extract::Query;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use saif_config::{
    clear_named_profile_deleted, load_effective_saif_config, mark_named_profile_deleted,
    migrate_from_hermes, parse_git_name_status, ProfileScope, SaifConfig, SecretScope,
    UpstreamLock,
};
use saif_core::{AgentEvent, AgentRunner, TurnOptions};
use saif_gateway::{build_gateway_router, GatewayHttpState, GatewayMultiplexer, TelegramAdapter};
use saif_persistence::{KanbanStore, PersistenceStore, SqliteStore};
use saif_plugins::{HookRegistry, PluginManager};
use saif_providers::{
    builtin_providers, read_auth_json_secret, resolve_provider_route_for_home,
    AnthropicMessagesTransport, ApiMode, ChatMessage, CredentialPool, LlmRequest, LlmTransport,
    PoolStrategy, StreamChunk,
};
use saif_tools::{
    register_all_builtin_tools, register_trusted_mcp_tools, McpServerConfig, McpTrustStore,
    ProcessRegistry, TodoStore, ToolRegistry,
};
use saif_web::{build_web_router, WebState};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn phase1_session_hydration_and_durable_compaction_across_restarts() {
    let seen_histories: Arc<tokio::sync::Mutex<Vec<Vec<Value>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let seen_srv = Arc::clone(&seen_histories);
    let call_counter = Arc::new(AtomicUsize::new(0));
    let call_srv = Arc::clone(&call_counter);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(req): Json<Value>| {
            let seen_srv = Arc::clone(&seen_srv);
            let call_srv = Arc::clone(&call_srv);
            async move {
                let msgs = req["messages"].as_array().cloned().unwrap_or_default();
                seen_srv.lock().await.push(msgs.clone());
                let n = call_srv.fetch_add(1, Ordering::SeqCst);
                let reply = format!("turn-{n}-ack");
                Json(json!({
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": reply },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 20, "completion_tokens": 5 }
                }))
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("saif_home");
    let profile = Arc::new(ProfileScope::for_root(home.clone(), false));
    profile.ensure_layout().unwrap();

    let mut cfg = SaifConfig {
        provider: "custom".into(),
        model: "mock-1".into(),
        base_url: Some(format!("http://{addr}/v1")),
        ..SaifConfig::default()
    };
    cfg.compression.enabled = false;
    let cfg_arc = Arc::new(cfg);

    // Turn 1 with first runner instance
    let store1 = SqliteStore::open(&profile.state_db_path()).unwrap();
    let runner1 = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&cfg_arc),
        ToolRegistry::new(),
        Some(store1),
        HookRegistry::new(),
    );
    let out1 = runner1
        .run_conversation(
            "Remember keyword: ALBATROSS",
            None,
            TurnOptions {
                session_id: Some("sess-hydrate-1".into()),
                source: "cli".into(),
                cwd: tmp.path().to_path_buf(),
                ..TurnOptions::default()
            },
            None,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out1.final_response, "turn-0-ack");

    // Turn 2 with a FRESH SqliteStore + AgentRunner and prior_messages = None
    let store2 = SqliteStore::open(&profile.state_db_path()).unwrap();
    let runner2 = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&cfg_arc),
        ToolRegistry::new(),
        Some(store2.clone()),
        HookRegistry::new(),
    );
    let out2 = runner2
        .run_conversation(
            "What keyword did I mention?",
            None,
            TurnOptions {
                session_id: Some("sess-hydrate-1".into()),
                source: "cli".into(),
                cwd: tmp.path().to_path_buf(),
                ..TurnOptions::default()
            },
            None,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out2.final_response, "turn-1-ack");

    {
        let calls = seen_histories.lock().await;
        assert_eq!(calls.len(), 2);
        // Byte-identical system[0] preserved across restart
        assert_eq!(calls[0][0]["content"], calls[1][0]["content"]);
        // Turn 2 carries hydrated Turn 1 user + assistant + Turn 2 user
        assert_eq!(calls[1].len(), 4);
        assert!(calls[1][1]["content"]
            .as_str()
            .unwrap()
            .contains("ALBATROSS"));
        assert_eq!(calls[1][2]["content"], "turn-0-ack");
    }

    // Turn 3 with low compression threshold persists compaction to SQLite
    let mut cfg_compact = (*cfg_arc).clone();
    cfg_compact.compression.enabled = true;
    cfg_compact.compression.threshold_ratio = 0.0002; // ~25 tokens
    cfg_compact.compression.keep_tail_tokens = 12;
    let runner3 = AgentRunner::new(
        Arc::clone(&profile),
        Arc::new(cfg_compact),
        ToolRegistry::new(),
        Some(store2.clone()),
        HookRegistry::new(),
    );
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let long_prompt = format!("Third turn details: {}", "x".repeat(260));
    let _ = runner3
        .run_conversation(
            &long_prompt,
            None,
            TurnOptions {
                session_id: Some("sess-hydrate-1".into()),
                source: "cli".into(),
                cwd: tmp.path().to_path_buf(),
                ..TurnOptions::default()
            },
            Some(ev_tx),
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    let mut saw_compacted = false;
    while let Ok(ev) = ev_rx.try_recv() {
        if matches!(ev, AgentEvent::Compacted { summarized, .. } if summarized > 0) {
            saw_compacted = true;
        }
    }
    assert!(
        saw_compacted,
        "expected ContextCompressor to emit Compacted"
    );

    let active_after = store2.load_active_messages("sess-hydrate-1").await.unwrap();
    assert!(
        active_after
            .iter()
            .any(|m| m.display_kind.as_deref() == Some("compaction_summary")),
        "expected durable compaction_summary row in SQLite"
    );

    // Turn 4 resumes cleanly from compacted DB without role alternation error
    let out4 = runner3
        .run_conversation(
            "Continue after compaction",
            None,
            TurnOptions {
                session_id: Some("sess-hydrate-1".into()),
                source: "cli".into(),
                cwd: tmp.path().to_path_buf(),
                ..TurnOptions::default()
            },
            None,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert!(out4.final_response.starts_with("turn-"));
    assert_eq!(out4.messages[0].content, out1.messages[0].content);
}

#[tokio::test]
async fn phase2_delegate_task_kanban_mcp_and_plugin_hooks_wiring() {
    let step = Arc::new(AtomicUsize::new(0));
    let step_srv = Arc::clone(&step);
    let saw_delegate_schema = Arc::new(AtomicUsize::new(0));
    let saw_del_srv = Arc::clone(&saw_delegate_schema);
    let child_saw_delegate_schema = Arc::new(AtomicUsize::new(0));
    let child_del_srv = Arc::clone(&child_saw_delegate_schema);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(req): Json<Value>| {
            let step_srv = Arc::clone(&step_srv);
            let saw_del_srv = Arc::clone(&saw_del_srv);
            let child_del_srv = Arc::clone(&child_del_srv);
            async move {
                let tool_names: Vec<String> = req["tools"]
                    .as_array()
                    .unwrap_or(&Vec::new())
                    .iter()
                    .filter_map(|t| t.pointer("/function/name").and_then(|v| v.as_str()).map(ToString::to_string))
                    .collect();
                let last_user = req["messages"]
                    .as_array()
                    .and_then(|arr| arr.iter().rev().find(|m| m["role"] == "user"))
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("");

                if last_user.contains("Sub-goal ping") {
                    // Delegated child call MUST NOT receive delegate_task when orchestrator_enabled=false
                    if tool_names.iter().any(|n| n == "delegate_task") {
                        child_del_srv.fetch_add(1, Ordering::SeqCst);
                    }
                    return Json(json!({
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "child-done-ok" },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
                    }));
                }

                if tool_names.iter().any(|n| n == "delegate_task") {
                    saw_del_srv.fetch_add(1, Ordering::SeqCst);
                }

                let n = step_srv.fetch_add(1, Ordering::SeqCst);
                match n {
                    0 => Json(json!({
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "",
                                "tool_calls": [
                                    {
                                        "id": "tc_kb",
                                        "type": "function",
                                        "function": { "name": "kanban_create", "arguments": "{\"title\":\"Phase2 Board Item\"}" }
                                    },
                                    {
                                        "id": "tc_mcp",
                                        "type": "function",
                                        "function": { "name": "mcp__demo__echo_tool", "arguments": "{\"msg\":\"hi-mcp\"}" }
                                    },
                                    {
                                        "id": "tc_del",
                                        "type": "function",
                                        "function": { "name": "delegate_task", "arguments": "{\"goal\":\"Sub-goal ping\",\"context\":\"ctx-alpha\"}" }
                                    }
                                ]
                            },
                            "finish_reason": "tool_calls"
                        }],
                        "usage": { "prompt_tokens": 20, "completion_tokens": 10 }
                    })),
                    _ => Json(json!({
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "All Phase2 tools wired!" },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 30, "completion_tokens": 8 }
                    })),
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("saif_home");
    fs::create_dir_all(&home).unwrap();

    // Seed effective config with ${MOCK_BASE_URL} interpolation in .env before loading ProfileScope
    fs::write(
        home.join(".env"),
        format!("MOCK_BASE_URL=http://{addr}/v1\n"),
    )
    .unwrap();
    fs::write(
        home.join("config.yaml"),
        "provider: custom\nmodel: mock-p2\nbase_url: \"${MOCK_BASE_URL}\"\n",
    )
    .unwrap();
    let profile = Arc::new(ProfileScope::for_root(home.clone(), false));
    profile.ensure_layout().unwrap();
    let eff_cfg = Arc::new(load_effective_saif_config(&home, Some(&profile.secrets)).unwrap());
    assert_eq!(
        eff_cfg.base_url.as_deref(),
        Some(format!("http://{addr}/v1").as_str())
    );

    // Seed an enabled plugin and verify PluginHookBridge logs lifecycle hooks
    let pdir = profile.plugins_dir().join("telemetry-plugin");
    fs::create_dir_all(&pdir).unwrap();
    fs::write(
        pdir.join("plugin.yaml"),
        "name: telemetry-plugin\nversion: 0.1.0\nkind: general\n",
    )
    .unwrap();
    let child_env = profile.secrets.build_child_env(&home, &[]);
    let hooks = PluginManager::new(profile.plugins_dir(), &[]).build_hook_registry(
        &eff_cfg.plugins.isolation,
        &profile.logs_dir(),
        child_env.clone(),
    );

    // Seed a POSIX shell MCP stdio server & grant trust
    let mcp_script = tmp.path().join("mock_mcp.sh");
    fs::write(
        &mcp_script,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{}}}\n'
      ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo_tool","description":"Echo via MCP","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}}}}]}}\n'
      ;;
    *'"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"mcp-pong"}]}}\n'
      ;;
  esac
done
"#,
    )
    .unwrap();
    fs::set_permissions(&mcp_script, fs::Permissions::from_mode(0o755)).unwrap();

    let trust = McpTrustStore::new(&home);
    trust.grant_trust("demo").unwrap();

    let registry = ToolRegistry::new();
    register_all_builtin_tools(&registry, ProcessRegistry::new(), TodoStore::default());
    let registered_mcp = register_trusted_mcp_tools(
        &registry,
        &[McpServerConfig {
            name: "demo".into(),
            command: mcp_script.to_string_lossy().into_owned(),
            args: vec![],
            env: BTreeMap::new(),
        }],
        &trust,
        &child_env,
    )
    .await;
    assert_eq!(registered_mcp, 1);

    let store = SqliteStore::open(&profile.state_db_path()).unwrap();
    let runner = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&eff_cfg),
        registry,
        Some(store),
        hooks,
    );

    let out = runner
        .run_conversation(
            "Run kanban + mcp + delegate",
            None,
            TurnOptions {
                source: "cli".into(),
                cwd: tmp.path().to_path_buf(),
                ..TurnOptions::default()
            },
            None,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();

    assert_eq!(out.final_response, "All Phase2 tools wired!");
    assert!(saw_delegate_schema.load(Ordering::SeqCst) >= 1);
    assert_eq!(child_saw_delegate_schema.load(Ordering::SeqCst), 0);

    // Check Kanban DB actually received the created task
    let kb = KanbanStore::open(&profile.kanban_db_path()).unwrap();
    let tasks = kb.list_tasks(None).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Phase2 Board Item");

    // Check Tool messages contained MCP + Delegation results
    let tool_msgs: Vec<String> = out
        .messages
        .iter()
        .filter(|m| m.role == saif_providers::MessageRole::Tool)
        .map(|m| m.content.clone())
        .collect();
    assert!(tool_msgs.iter().any(|c| c.contains("mcp-pong")));
    assert!(tool_msgs
        .iter()
        .any(|c| c.contains("child-done-ok") && c.contains("completed")));

    // Check PluginHookBridge recorded lifecycle events
    let hook_log = fs::read_to_string(profile.logs_dir().join("plugin_hooks.jsonl")).unwrap();
    assert!(hook_log.contains("on_session_start"));
    assert!(hook_log.contains("pre_tool_call"));
    assert!(hook_log.contains("on_session_end"));
}

#[tokio::test]
async fn phase3_36_providers_credential_pool_failover_auth_json_and_anthropic_sse() {
    assert_eq!(builtin_providers().len(), 36);
    assert!(builtin_providers().iter().any(|p| p.id == "zai"));
    assert!(builtin_providers().iter().any(|p| p.id == "cerebras"));

    // Mock OpenAI server rejecting k1 with 429 and succeeding on k2, plus mock Anthropic SSE
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(|headers: axum::http::HeaderMap, Json(_): Json<Value>| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if auth == "Bearer sk-bad-429" {
                    return (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({ "error": "rate_limit" })),
                    )
                        .into_response();
                }
                if auth == "Bearer sk-good-200" {
                    return Json(json!({
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "failover-ok" },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 5, "completion_tokens": 3 }
                    }))
                    .into_response();
                }
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "bad key" })),
                )
                    .into_response()
            }),
        )
        .route(
            "/v1/messages",
            post(|| async move {
                let sse_body = concat!(
                    "event: message_start\n",
                    "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":14,\"cache_read_input_tokens\":4}}}\n\n",
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"ponder \"}}\n\n",
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello SSE\"}}\n\n",
                    "event: content_block_start\n",
                    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"read_file\"}}\n\n",
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}\n\n",
                    "event: message_delta\n",
                    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\n\n",
                    "event: message_stop\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                );
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    sse_body,
                )
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("saif_home");
    let profile = Arc::new(ProfileScope::for_root(home.clone(), false));
    profile.ensure_layout().unwrap();

    // Test auth.json fallback
    fs::write(
        home.join("auth.json"),
        r#"{ "openai": { "api_key": "sk-from-auth-json" } }"#,
    )
    .unwrap();
    assert_eq!(
        read_auth_json_secret(&home, "openai").as_deref(),
        Some("sk-from-auth-json")
    );
    let route_auth = resolve_provider_route_for_home(
        Some(&home),
        &SaifConfig {
            provider: "openai".into(),
            ..SaifConfig::default()
        },
        &SecretScope::empty(false, true),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(route_auth.api_key.as_deref(), Some("sk-from-auth-json"));

    // Test live CredentialPool automatic failover 429 -> 200
    let mut pool = CredentialPool::open(&home);
    pool.set_strategy(PoolStrategy::FillFirst);
    pool.add_credential("custom", "bad_k1", "sk-bad-429", "manual");
    pool.add_credential("custom", "good_k2", "sk-good-200", "manual");
    pool.save().unwrap();

    let cfg = Arc::new(SaifConfig {
        provider: "custom".into(),
        model: "mock-m".into(),
        base_url: Some(format!("http://{addr}/v1")),
        ..SaifConfig::default()
    });
    let runner = AgentRunner::new(
        Arc::clone(&profile),
        cfg,
        ToolRegistry::new(),
        None,
        HookRegistry::new(),
    );
    let out = runner
        .run_conversation(
            "Trigger failover",
            None,
            TurnOptions {
                skip_persistence: true,
                cwd: tmp.path().to_path_buf(),
                ..TurnOptions::default()
            },
            None,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out.final_response, "failover-ok");

    let reloaded_pool = CredentialPool::open(&home);
    let k1 = reloaded_pool
        .list(Some("custom"))
        .into_iter()
        .find(|c| c.id == "bad_k1")
        .unwrap();
    assert_eq!(k1.status, saif_providers::CredStatus::Exhausted);

    // Test Anthropic SSE streaming
    let ant = AnthropicMessagesTransport::new();
    let route_ant = saif_providers::ResolvedRoute {
        provider_id: "anthropic".into(),
        base_url: format!("http://{addr}"),
        api_mode: ApiMode::AnthropicMessages,
        model: "claude-sonnet-4".into(),
        api_key: Some("sk-ant-test".into()),
        omit_temperature: false,
        supports_prompt_caching: true,
        multiplex_active: false,
        pooled_credential_id: None,
        saif_home: None,
        aws_region: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_profile: None,
    };
    let req_ant = LlmRequest {
        model: "claude-sonnet-4".into(),
        messages: vec![ChatMessage::user("hi")],
        stream: true,
        ..LlmRequest::default()
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let resp = ant.complete(&route_ant, &req_ant, Some(tx)).await.unwrap();
    assert_eq!(resp.content, "Hello SSE");
    assert_eq!(resp.reasoning.as_deref(), Some("ponder "));
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "read_file");
    assert_eq!(resp.tool_calls[0].arguments["path"], "README.md");
    assert_eq!(resp.usage.input_tokens, 14);
    assert_eq!(resp.usage.output_tokens, 9);

    let mut got_text_delta = false;
    let mut got_think_delta = false;
    while let Ok(ch) = rx.try_recv() {
        match ch {
            StreamChunk::ContentDelta(t) if t == "Hello SSE" => got_text_delta = true,
            StreamChunk::ReasoningDelta(r) if r == "ponder " => got_think_delta = true,
            _ => {}
        }
    }
    assert!(got_text_delta && got_think_delta);
}

#[test]
fn phase4_multi_profile_hermes_migration_and_boundary_safety() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("hermes_src");
    let dst = tmp.path().join("saif_dst");

    // Seed root + live named profile + tombstoned named profile + plugins
    fs::create_dir_all(src.join("plugins/my-plug")).unwrap();
    fs::write(src.join("config.yaml"), "model: gpt-4.1\n").unwrap();
    fs::write(
        src.join("plugins/my-plug/plugin.yaml"),
        "name: my-plug\nkind: general\n",
    )
    .unwrap();

    let coder_src = src.join("profiles/coder");
    fs::create_dir_all(coder_src.join("skills/rust-skill")).unwrap();
    fs::write(coder_src.join("config.yaml"), "model: claude-sonnet-4\n").unwrap();
    fs::write(
        coder_src.join("skills/rust-skill/SKILL.md"),
        "---\nname: rust-skill\ndescription: Rust tips.\n---\nBody\n",
    )
    .unwrap();

    let del_src = src.join("profiles/retired");
    fs::create_dir_all(&del_src).unwrap();
    fs::write(del_src.join("config.yaml"), "model: old\n").unwrap();
    mark_named_profile_deleted(&del_src).unwrap();

    let report = migrate_from_hermes(&src, &dst, false).unwrap();
    assert_eq!(report.migrated_profiles, vec!["coder".to_string()]);
    assert!(dst.join("plugins/my-plug/plugin.yaml").is_file());
    assert!(dst.join("profiles/coder/config.yaml").is_file());
    assert!(dst
        .join("profiles/coder/skills/rust-skill/SKILL.md")
        .is_file());
    assert!(!dst.join("profiles/retired").exists());

    // Verify tombstoned boundary cannot be resurrected unless cleared
    let retired_dst = dst.join("profiles/retired");
    fs::create_dir_all(&retired_dst).unwrap();
    mark_named_profile_deleted(&retired_dst).unwrap();
    assert!(ProfileScope::for_profile(dst.clone(), Some("retired"), false).is_err());
    clear_named_profile_deleted(&retired_dst).unwrap();
    assert!(ProfileScope::for_profile(dst, Some("retired"), false).is_ok());
}

#[derive(Default)]
struct MockTgState {
    next_msg_id: i64,
    pending_updates: Vec<Value>,
    sent_messages: Vec<Value>,
    edited_messages: Vec<Value>,
}

#[tokio::test]
async fn phase5_gateway_stop_steer_queue_drain_and_telegram_streaming() {
    let llm_calls = Arc::new(AtomicUsize::new(0));
    let llm_calls_srv = Arc::clone(&llm_calls);

    let llm_app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(req): Json<Value>| {
            let llm_calls_srv = Arc::clone(&llm_calls_srv);
            async move {
                let msgs = req["messages"].as_array().cloned().unwrap_or_default();
                let last_txt = msgs
                    .last()
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("")
                    .to_string();
                let n = llm_calls_srv.fetch_add(1, Ordering::SeqCst);

                if last_txt.contains("slow-cancel") {
                    tokio::time::sleep(Duration::from_millis(450)).await;
                } else if last_txt.contains("stream-me") {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
                let is_stream = req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
                if is_stream {
                    let body = if last_txt.contains("stream-me") {
                        concat!(
                            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Part1-streaming-frame-long \"},\"finish_reason\":null}]}\n\n",
                            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Part2\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
                            "data: [DONE]\n\n"
                        )
                        .to_string()
                    } else {
                        format!(
                            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"ack#{n}:{last_txt}\"}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":3,\"completion_tokens\":2}}}}\n\ndata: [DONE]\n\n"
                        )
                    };
                    return ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response();
                }
                Json(json!({
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": format!("ack#{n}:{last_txt}") },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 2, "completion_tokens": 2 }
                }))
                .into_response()
            }
        }),
    );

    let llm_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let llm_addr = llm_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(llm_listener, llm_app).await;
    });

    // Outbound webhook sink + Mock Telegram Bot API
    let webhook_hits: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let wh_srv = Arc::clone(&webhook_hits);

    let tg_state = Arc::new(Mutex::new(MockTgState {
        next_msg_id: 100,
        pending_updates: vec![
            json!({
                "update_id": 1,
                "message": {
                    "chat": { "id": 555, "type": "private" },
                    "from": { "id": 777 },
                    "text": "stream-me"
                }
            }),
            json!({
                "update_id": 2,
                "message": {
                    "chat": { "id": 555, "type": "private" },
                    "from": { "id": 777 },
                    "text": "queued-after-stream"
                }
            }),
        ],
        ..MockTgState::default()
    }));

    let tg_get = Arc::clone(&tg_state);
    let tg_send = Arc::clone(&tg_state);
    let tg_edit = Arc::clone(&tg_state);

    let mock_srv = Router::new()
        .route(
            "/sink",
            post(move |Json(body): Json<Value>| {
                let wh_srv = Arc::clone(&wh_srv);
                async move {
                    wh_srv.lock().unwrap().push(body);
                    Json(json!({ "ok": true }))
                }
            }),
        )
        .route(
            "/getUpdates",
            get(move |Query(q): Query<HashMap<String, String>>| {
                let tg_get = Arc::clone(&tg_get);
                async move {
                    let offset = q
                        .get("offset")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(0);
                    let batch: Vec<Value> = {
                        let mut st = tg_get.lock().unwrap();
                        st.pending_updates
                            .drain(..)
                            .filter(|u| u["update_id"].as_i64().unwrap_or(0) >= offset)
                            .collect()
                    };
                    if batch.is_empty() {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                    Json(json!({ "ok": true, "result": batch }))
                }
            }),
        )
        .route(
            "/sendMessage",
            post(move |Json(body): Json<Value>| {
                let tg_send = Arc::clone(&tg_send);
                async move {
                    let mut st = tg_send.lock().unwrap();
                    st.next_msg_id += 1;
                    let mid = st.next_msg_id;
                    st.sent_messages.push(body);
                    Json(json!({ "ok": true, "result": { "message_id": mid } }))
                }
            }),
        )
        .route(
            "/editMessageText",
            post(move |Json(body): Json<Value>| {
                let tg_edit = Arc::clone(&tg_edit);
                async move {
                    tg_edit.lock().unwrap().edited_messages.push(body);
                    Json(json!({ "ok": true }))
                }
            }),
        );

    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(mock_listener, mock_srv).await;
    });

    let tmp = TempDir::new().unwrap();
    let profile = Arc::new(ProfileScope::for_root(tmp.path().to_path_buf(), false));
    profile.ensure_layout().unwrap();
    let mut cfg = SaifConfig {
        provider: "custom".into(),
        model: "m".into(),
        base_url: Some(format!("http://{llm_addr}/v1")),
        ..SaifConfig::default()
    };
    cfg.security.allow_private_urls = true;
    let cfg_arc = Arc::new(cfg);

    let agent = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&cfg_arc),
        ToolRegistry::new(),
        None,
        HookRegistry::new(),
    );

    // Run mock Telegram polling with cancellation token
    let cancel_poll = CancellationToken::new();
    let poll_stop = cancel_poll.clone();
    let tg_agent = agent.clone();
    let tg_cwd = tmp.path().to_path_buf();
    let tg_task = tokio::spawn(async move {
        let mut adapter =
            TelegramAdapter::new("tok".into(), "default".into(), false, vec!["777".into()])
                .with_api_base(format!("http://{mock_addr}"));
        let _ = adapter
            .run_polling_with_cancel(tg_agent, tg_cwd, poll_stop)
            .await;
    });

    // Wait for both the streamed turn + drained queued message to deliver
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let st = tg_state.lock().unwrap();
        let has_final = st.edited_messages.iter().any(|m| {
            m["text"]
                .as_str()
                .is_some_and(|t| t.contains("Part1-streaming-frame-long Part2"))
        }) || st.sent_messages.iter().any(|m| {
            m["text"]
                .as_str()
                .is_some_and(|t| t.contains("Part1-streaming-frame-long Part2"))
        });
        let has_queued_drained = st.sent_messages.iter().any(|m| {
            m["text"]
                .as_str()
                .is_some_and(|t| t.contains("queued-after-stream"))
        });
        if has_final && has_queued_drained {
            break;
        }
    }
    cancel_poll.cancel();
    let _ = tg_task.await;

    {
        let st = tg_state.lock().unwrap();
        assert!(
            st.sent_messages
                .iter()
                .any(|m| m["text"].as_str().is_some_and(|t| t.contains("Queued"))),
            "expected queued interim ack"
        );
        assert!(
            st.sent_messages
                .iter()
                .chain(st.edited_messages.iter())
                .any(|m| m["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("Part1-streaming-frame-long Part2"))),
            "expected streamed reply"
        );
        assert!(
            st.sent_messages
                .iter()
                .chain(st.edited_messages.iter())
                .any(|m| m["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("queued-after-stream"))),
            "expected busy queue to auto-drain"
        );
    }

    // Test GatewayMultiplexer /stop + outbound webhook dispatch
    let mut mux = GatewayMultiplexer::new("default");
    mux.add_slot(profile, cfg_arc, agent.clone());
    let mux_arc = Arc::new(mux);
    let gw_app = build_gateway_router(GatewayHttpState {
        agent,
        cwd: tmp.path().to_path_buf(),
        multiplexer: Some(Arc::clone(&mux_arc)),
    });
    let gw_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(gw_listener, gw_app).await;
    });

    let client = reqwest::Client::new();
    let slow_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .post(format!("http://{gw_addr}/webhooks/discord"))
                .json(&json!({
                    "channel_id": "ch_stop",
                    "author": { "id": "u1" },
                    "content": "slow-cancel"
                }))
                .send()
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    let stop_ack: Value = client
        .post(format!("http://{gw_addr}/webhooks/discord"))
        .json(&json!({
            "channel_id": "ch_stop",
            "author": { "id": "u1" },
            "content": "/stop"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stop_ack["delivery_action_kind"], "control_bypass");
    assert_eq!(stop_ack["chunks"][0], "Stopped active turn.");
    let slow_res = slow_task.await.unwrap().unwrap();
    assert_eq!(slow_res.status(), reqwest::StatusCode::BAD_REQUEST);

    // Verify outbound HTTP dispatch
    let out_ack: Value = client
        .post(format!("http://{gw_addr}/webhooks/webhook"))
        .json(&json!({
            "chat_id": "ch_out",
            "text": "ping-outbound",
            "deliver_outbound": true,
            "outbound_url": format!("http://{mock_addr}/sink")
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(out_ack["outbound_dispatched"], true);
    assert_eq!(webhook_hits.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn phase6_wasm_asset_csp_404_and_realtime_ws_streaming() {
    let llm_app = Router::new().route(
        "/v1/chat/completions",
        post(|| async move {
            let stream = async_stream_sse();
            (
                [(header::CONTENT_TYPE, "text/event-stream")],
                axum::body::Body::from_stream(stream),
            )
        }),
    );
    let llm_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let llm_addr = llm_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(llm_listener, llm_app).await;
    });

    let tmp = TempDir::new().unwrap();
    let profile = Arc::new(ProfileScope::for_root(tmp.path().to_path_buf(), false));
    profile.ensure_layout().unwrap();
    let cfg = Arc::new(SaifConfig {
        provider: "custom".into(),
        model: "m".into(),
        base_url: Some(format!("http://{llm_addr}/v1")),
        ..SaifConfig::default()
    });
    let store = SqliteStore::open(&profile.state_db_path()).unwrap();
    let runner = AgentRunner::new(
        Arc::clone(&profile),
        Arc::clone(&cfg),
        ToolRegistry::new(),
        Some(store.clone()),
        HookRegistry::new(),
    );
    let web_app = build_web_router(WebState {
        session_token: "tok-ws".into(),
        headless: false,
        profile,
        config: cfg,
        store: Some(store),
        agent: runner,
        pty_program: "sh".into(),
        pty_args: vec![],
        cwd: tmp.path().to_path_buf(),
    });
    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let web_addr = web_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(web_listener, web_app).await;
    });

    let client = reqwest::Client::new();
    let root_resp = client
        .get(format!("http://{web_addr}/"))
        .send()
        .await
        .unwrap();
    assert_eq!(root_resp.status(), reqwest::StatusCode::OK);
    let csp = root_resp
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(csp.contains("wasm-unsafe-eval"));

    let wasm_resp = client
        .get(format!("http://{web_addr}/assets/saif_web_ui.wasm"))
        .send()
        .await
        .unwrap();
    assert_eq!(wasm_resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        wasm_resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/wasm")
    );
    let wasm_bytes = wasm_resp.bytes().await.unwrap();
    assert!(wasm_bytes.starts_with(b"\0asm"));

    let ico_resp = client
        .get(format!("http://{web_addr}/favicon.ico"))
        .send()
        .await
        .unwrap();
    assert_eq!(ico_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // Real-time WebSocket streaming: MessageDelta frames must arrive BEFORE final RPC response!
    let (mut ws, _) = connect_async(format!("ws://{web_addr}/ws?token=tok-ws"))
        .await
        .unwrap();
    let _ready = ws.next().await.unwrap().unwrap();
    ws.send(WsMessage::Text(
        json!({
            "jsonrpc": "2.0",
            "id": 77,
            "method": "prompt.submit",
            "params": { "session_id": "ws-live", "prompt": "stream", "steer": false }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let mut frame_order = Vec::new();
    while let Some(Ok(msg)) = ws.next().await {
        let WsMessage::Text(txt) = msg else { continue };
        let v: Value = serde_json::from_str(&txt).unwrap();
        if v.pointer("/params/type").and_then(|t| t.as_str()) == Some("message_delta") {
            frame_order.push(format!("delta:{}", v["params"]["delta"].as_str().unwrap()));
        } else if v.get("id") == Some(&json!(77)) {
            frame_order.push("rpc_ok".to_string());
            break;
        }
    }
    assert_eq!(frame_order, vec!["delta:chunkA ", "delta:chunkB", "rpc_ok"]);
}

fn async_stream_sse(
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>> {
    futures_util::stream::unfold(0u8, |step| async move {
        match step {
            0 => Some((
                Ok(axum::body::Bytes::from(
                    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"chunkA \"},\"finish_reason\":null}]}\n\n",
                )),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(35)).await;
                Some((
                    Ok(axum::body::Bytes::from(
                        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"chunkB\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n",
                    )),
                    2,
                ))
            }
            _ => None,
        }
    })
}

#[test]
fn phase7_upstream_lock_diff_classification_report() {
    let lock_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/UPSTREAM_LOCK.json");
    let lock = UpstreamLock::load(&lock_path).unwrap();

    let diff_raw = "\
M\tplugin-catalog/iacg-board.yaml
M\thermes_cli/old_updater_main.py
A\tagent/redact_db_url.py
M\tgateway/run.py
M\tweb/src/App.tsx
";
    let entries = parse_git_name_status(diff_raw);
    let report = lock.build_report("aaa111", "bbb222", entries);
    assert_eq!(report.total_changed_files, 5);
    assert_eq!(report.ignored_noise_count, 1);
    assert_eq!(report.skipped_runtime_count, 1);
    assert_eq!(report.port_required_count, 3);
    assert!(report.by_subsystem.contains_key("profile_secrets"));
    assert!(report.by_subsystem.contains_key("gateway_platforms"));
    assert!(report.by_subsystem.contains_key("web_dashboard"));
    let md = report.render_markdown(true, None);
    assert!(md.contains("crates/saif-config/src/redact.rs"));
    assert!(!md.contains("plugin-catalog/iacg-board.yaml"));
}
