use axum::routing::post;
use axum::{Json, Router};
use saif_config::{migrate_from_hermes, ProfileScope, SaifConfig, SecretScope};
use saif_core::{validate_role_alternation, AgentRunner, ContextCompressor, TurnOptions};
use saif_gateway::{
    resolve_identity, BusyDecision, BusyMessageGuard, DeliveryAction, StreamDraftController,
};
use saif_persistence::{KanbanStore, PersistenceStore, SqliteStore, TaskStatus};
use saif_plugins::{validate_catalog_entry, CatalogEntry, HookRegistry, RemovedCatalog};
use saif_providers::{ChatMessage, ToolCall};
use saif_skills::SkillStore;
use saif_tools::{register_all_builtin_tools, ProcessRegistry, TodoStore, ToolRegistry};
use saif_web::parse_pty_resize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[test]
fn secret_scope_fail_closed_under_multiplex() {
    let mut default_map = BTreeMap::new();
    default_map.insert("OPENAI_API_KEY".into(), "sk-default-key".into());

    let default_scope = SecretScope::from_map(default_map, true, true);
    assert_eq!(
        default_scope.get_secret("OPENAI_API_KEY").as_deref(),
        Some("sk-default-key")
    );

    let secondary_scope = SecretScope::empty(false, true);
    // Must NEVER fall back to std::env or default profile when multiplex_active = true!
    std::env::set_var("SAIF_TEST_SECRET_LEAK_GUARD", "host-secret");
    assert_eq!(
        secondary_scope.get_secret("SAIF_TEST_SECRET_LEAK_GUARD"),
        None
    );

    let child_env = secondary_scope.build_child_env(std::path::Path::new("/tmp/p2"), &[]);
    assert!(!child_env.contains_key("OPENAI_API_KEY"));
    assert_eq!(
        child_env.get("SAIF_HOME").map(String::as_str),
        Some("/tmp/p2")
    );
}

#[test]
fn role_alternation_allows_steer_after_tool_forbids_double_user() {
    let valid = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("hi"),
        ChatMessage::assistant(
            "",
            Some(vec![ToolCall {
                id: "c1".into(),
                name: "read_file".into(),
                arguments: json!({"path": "a"}),
            }]),
            None,
        ),
        ChatMessage::tool("c1", "read_file", "ok"),
        ChatMessage::user("steer: focus on line 10"),
    ];
    assert!(validate_role_alternation(&valid).is_ok());

    let invalid = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("one"),
        ChatMessage::user("two"),
    ];
    assert!(validate_role_alternation(&invalid).is_err());
}

#[test]
fn gateway_streaming_four_invariants_and_busy_guard() {
    let mut ctrl = StreamDraftController::new();
    ctrl.set_message_id("42");

    assert_eq!(
        ctrl.push_delta("Hello "),
        Some(DeliveryAction::UpdateStream {
            message_id: "42".into(),
            frame: "Hello ".into()
        })
    );
    // Invariant 1: non-prefix mutation rejected
    assert!(ctrl.push_snapshot("Completely different").is_err());
    // Invariant 3: interim send never seals stream
    assert_eq!(
        ctrl.send_interim("Tool running..."),
        DeliveryAction::InterimSend {
            text: "Tool running...".into()
        }
    );
    assert!(!ctrl.is_sealed());
    // Invariants 2 & 4: explicit finish reconciles via EditMessage when message_id exists
    assert_eq!(
        ctrl.finish("Hello world!"),
        DeliveryAction::EditMessage {
            message_id: "42".into(),
            text: "Hello world!".into()
        }
    );
    assert!(ctrl.is_sealed());

    let mut guard = BusyMessageGuard::default();
    let ident = resolve_identity(
        "telegram",
        "default",
        Some("coder"),
        "999",
        "u1",
        None,
        true,
    );
    assert_eq!(ident.session_key, "agent:coder:telegram:999");
    guard.mark_active(&ident.session_key);
    assert!(matches!(
        guard.admit(&ident.session_key, "/approve"),
        BusyDecision::ControlBypass(_)
    ));
    assert_eq!(
        guard.admit(&ident.session_key, "normal follow-up"),
        BusyDecision::Queued { position: 1 }
    );
}

#[test]
fn plugin_catalog_enforces_sha40_and_removed_kill_list() {
    let removed = RemovedCatalog {
        repos: vec!["https://github.com/evil/bad-plugin.git".into()],
    };
    let short_sha = CatalogEntry {
        name: "good".into(),
        repo: "https://github.com/nous/good".into(),
        commit: "abc123".into(),
        description: "".into(),
        docs_url: None,
    };
    assert!(validate_catalog_entry(&short_sha, &removed, false).is_err());

    let killed = CatalogEntry {
        name: "bad".into(),
        repo: "git@github.com:evil/bad-plugin.git".into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        description: "".into(),
        docs_url: None,
    };
    assert!(validate_catalog_entry(&killed, &removed, false).is_err());
    assert!(validate_catalog_entry(&killed, &removed, true).is_ok());
}

#[test]
fn kanban_dependencies_and_pty_resize_contract() {
    let tmp = TempDir::new().unwrap();
    let kb = KanbanStore::open(&tmp.path().join("kanban.db")).unwrap();
    let t1 = kb
        .create_task("parent", None, Some("coder"), 5, &[])
        .unwrap();
    let t2 = kb
        .create_task("child", None, Some("qa"), 5, std::slice::from_ref(&t1.id))
        .unwrap();
    assert_eq!(t2.status, TaskStatus::Blocked);

    let claimed = kb
        .claim_next_ready_task(Some("coder"), 1234, Some(999))
        .unwrap();
    assert_eq!(claimed.unwrap().id, t1.id);
    kb.complete_task(&t1.id, "done").unwrap();

    // Downstream child unblocks automatically once parent completes
    let t2_after = kb.show_task(&t2.id).unwrap().unwrap();
    assert_eq!(t2_after.status, TaskStatus::Ready);

    assert_eq!(parse_pty_resize("\x1b[RESIZE:120;40]"), Some((120, 40)));
    assert_eq!(parse_pty_resize("plain text"), None);
}

#[test]
fn hermes_migration_is_non_destructive() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    fs::write(src.path().join("config.yaml"), "model: gpt-4.1\n").unwrap();
    fs::create_dir_all(src.path().join("skills/demo")).unwrap();
    fs::write(
        src.path().join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: Demo skill.\n---\nBody\n",
    )
    .unwrap();

    let dry = migrate_from_hermes(src.path(), dst.path(), true).unwrap();
    assert_eq!(dry.copied_files.len(), 2);
    assert!(!dst.path().join("config.yaml").exists());

    let real = migrate_from_hermes(src.path(), dst.path(), false).unwrap();
    assert_eq!(real.copied_files.len(), 2);
    assert!(dst.path().join("config.yaml").is_file());
    assert!(dst.path().join("skills/demo/SKILL.md").is_file());
    assert!(src.path().join("config.yaml").is_file());
}

#[tokio::test]
async fn end_to_end_agent_loop_with_mock_openai_tool_call_and_prompt_cache_stability() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("hello.txt"), "rust-saif-parity").unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let seen_sys_prompts = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

    let calls_srv = Arc::clone(&calls);
    let prompts_srv = Arc::clone(&seen_sys_prompts);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(req): Json<Value>| {
            let calls_srv = Arc::clone(&calls_srv);
            let prompts_srv = Arc::clone(&prompts_srv);
            async move {
                let sys = req["messages"][0]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                prompts_srv.lock().await.push(sys);

                let n = calls_srv.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Json(json!({
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "",
                                "tool_calls": [{
                                    "id": "call_read_1",
                                    "type": "function",
                                    "function": {
                                        "name": "read_file",
                                        "arguments": "{\"path\":\"hello.txt\"}"
                                    }
                                }]
                            },
                            "finish_reason": "tool_calls"
                        }],
                        "usage": { "prompt_tokens": 100, "completion_tokens": 15 }
                    }))
                } else {
                    Json(json!({
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "File contains: rust-saif-parity"
                            },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 140, "completion_tokens": 10 }
                    }))
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let home = tmp.path().join("saif_home");
    let profile = Arc::new(ProfileScope::for_root(home.clone(), false));
    profile.ensure_layout().unwrap();

    // Seed an agent skill and verify /skill injects as user message without altering system[0]
    let skill_store = SkillStore::new(profile.skills_dir(), &[]);
    skill_store
        .create_skill(
            "inspect",
            "Inspect workspace files safely.",
            "Use read_file first.",
            None,
        )
        .unwrap();

    let cfg = Arc::new(SaifConfig {
        provider: "custom".to_string(),
        model: "mock-model".to_string(),
        base_url: Some(format!("http://{addr}/v1")),
        ..SaifConfig::default()
    });
    let store = SqliteStore::open(&profile.state_db_path()).unwrap();
    let registry = ToolRegistry::new();
    register_all_builtin_tools(&registry, ProcessRegistry::new(), TodoStore::default());

    let runner = AgentRunner::new(
        Arc::clone(&profile),
        cfg,
        registry,
        Some(store.clone()),
        HookRegistry::new(),
    );

    let (steer_tx, mut steer_rx) = mpsc::unbounded_channel();
    steer_tx
        .send("Verify exact file text.".to_string())
        .unwrap();

    let opts = TurnOptions {
        source: "conformance".to_string(),
        cwd: ws,
        ..TurnOptions::default()
    };

    let outcome = runner
        .run_conversation(
            "/skill inspect Check hello.txt",
            None,
            opts,
            None,
            CancellationToken::new(),
            Some(&mut steer_rx),
        )
        .await
        .unwrap();

    assert_eq!(outcome.final_response, "File contains: rust-saif-parity");
    assert_eq!(outcome.api_calls, 2);

    // Sacred Prompt-Caching Invariant: system[0] was 100% byte-identical across API calls!
    let recorded_prompts = seen_sys_prompts.lock().await;
    assert_eq!(recorded_prompts.len(), 2);
    assert_eq!(recorded_prompts[0], recorded_prompts[1]);

    // Verify ContextCompressor leaves system[0] intact
    let comp = ContextCompressor::compress(&outcome.messages, 10, None, None).await;
    assert_eq!(comp.messages[0].content, outcome.messages[0].content);

    // Verify SQLite persistence captured session + messages
    let persisted = store
        .load_active_messages(&outcome.session_id)
        .await
        .unwrap();
    assert!(persisted.len() >= 4);
}
