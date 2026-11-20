use chrono::Utc;
use saif_config::{
    named_profile_has_identity, named_profile_has_servable_identity, named_profile_is_deleted,
    named_profile_is_live,
};
use saif_core::{scan_context_content, tool_call_succeeded};
use saif_cron::{
    compute_grace_seconds, normalize_repeat_value, parse_duration, parse_schedule, CronSchedule,
    RepeatInput,
};
use saif_persistence::{
    shape_preview, PersistenceStore, SqliteStore, FTS_TOOL_CONTENT_PREFIX_CHARS,
};
use saif_providers::{
    alias_reserved_tools, bounded_prompt_cache_key, cache_scope_from_session_id, ToolDefinition,
};
use saif_skills::{
    clip_skill_description, extract_skill_description, is_skill_description_truncated_for_prompt,
    parse_frontmatter, scan_for_threats, ThreatScope,
};
use saif_tools::{
    config_bool, fuzzy_find_and_replace, parse_v4a_patch, truncate_stdout_bytes,
    tts_response_format_from_path,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/cases")
}

fn load_jsonl(filename: &str) -> Vec<Value> {
    let path = cases_dir().join(filename);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading oracle file {}: {e}", path.display()));
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid JSONL oracle line"))
        .collect()
}

#[test]
fn replay_cron_schedules_oracle() {
    let cases = load_jsonl("cron_schedules.jsonl");
    assert_eq!(cases.len(), 31);
    let now = Utc::now();

    for case in cases {
        let op = case["op"].as_str().unwrap();
        match op {
            "parse_duration" => {
                let input = case["input"].as_str().unwrap();
                let expect_ok = case["ok"].as_bool().unwrap();
                let res = parse_duration(input);
                assert_eq!(
                    res.is_ok(),
                    expect_ok,
                    "parse_duration mismatch for {input:?}"
                );
                if expect_ok {
                    assert_eq!(res.unwrap(), case["minutes"].as_i64().unwrap());
                }
            }
            "normalize_repeat" => {
                let expect_ok = case["ok"].as_bool().unwrap();
                let input_val = &case["input"];
                let res = match input_val {
                    Value::Null => normalize_repeat_value(RepeatInput::None),
                    Value::Number(n) => {
                        normalize_repeat_value(RepeatInput::Int(n.as_i64().unwrap()))
                    }
                    Value::String(s) => normalize_repeat_value(RepeatInput::Str(s)),
                    other => panic!("unexpected repeat input {other:?}"),
                };
                assert_eq!(
                    res.is_ok(),
                    expect_ok,
                    "normalize_repeat mismatch for {input_val}"
                );
                if expect_ok {
                    assert_eq!(res.unwrap(), case["value"].as_i64());
                }
            }
            "parse_schedule" => {
                let input = case["input"].as_str().unwrap();
                let expect_ok = case["ok"].as_bool().unwrap();
                let res = parse_schedule(input);
                assert_eq!(
                    res.is_ok(),
                    expect_ok,
                    "parse_schedule mismatch for {input:?}"
                );
                if expect_ok {
                    let parsed = res.unwrap();
                    let expected_kind = case["kind"].as_str().unwrap();
                    match (&parsed, expected_kind) {
                        (CronSchedule::Interval { minutes, .. }, "interval") => {
                            assert_eq!(*minutes, case["minutes"].as_i64().unwrap());
                        }
                        (CronSchedule::Cron { expr, .. }, "cron") => {
                            assert_eq!(expr, case["expr"].as_str().unwrap());
                        }
                        (CronSchedule::Once { .. }, "once") => {}
                        other => panic!("kind mismatch: {other:?} vs {expected_kind}"),
                    }
                    let grace = compute_grace_seconds(&parsed, now);
                    assert_eq!(
                        grace,
                        case["grace_seconds"].as_i64().unwrap(),
                        "grace mismatch for {input:?}"
                    );
                }
            }
            other => panic!("unknown cron op: {other}"),
        }
    }
}

#[test]
fn replay_patch_fuzzy_oracle() {
    let cases = load_jsonl("patch_fuzzy.jsonl");
    assert_eq!(cases.len(), 11);

    for case in cases {
        let op = case["op"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        match op {
            "fuzzy_find_and_replace" => {
                let res = fuzzy_find_and_replace(
                    case["content"].as_str().unwrap(),
                    case["old_string"].as_str().unwrap(),
                    case["new_string"].as_str().unwrap(),
                    case["replace_all"].as_bool().unwrap(),
                );
                assert_eq!(
                    res.content,
                    case["expected_content"].as_str().unwrap(),
                    "content mismatch for {name}"
                );
                assert_eq!(
                    res.match_count as u64,
                    case["expected_count"].as_u64().unwrap(),
                    "count mismatch for {name}"
                );
                assert_eq!(
                    res.strategy.as_deref(),
                    case["expected_strategy"].as_str(),
                    "strategy mismatch for {name}"
                );
                assert_eq!(
                    res.error.is_some(),
                    case["expected_error"].as_bool().unwrap(),
                    "error flag mismatch for {name}"
                );
            }
            "parse_v4a_patch" => {
                let res = parse_v4a_patch(case["patch"].as_str().unwrap());
                let expect_ok = case["ok"].as_bool().unwrap();
                assert_eq!(res.is_ok(), expect_ok, "v4a ok mismatch for {name}");
                if expect_ok {
                    let ops = res.unwrap();
                    let expected_ops = case["ops"].as_array().unwrap();
                    assert_eq!(ops.len(), expected_ops.len(), "op len mismatch for {name}");
                    for (got, exp) in ops.iter().zip(expected_ops.iter()) {
                        let got_kind = serde_json::to_value(got.operation).unwrap();
                        assert_eq!(got_kind, exp["operation"], "op kind mismatch for {name}");
                        assert_eq!(got.file_path, exp["file_path"].as_str().unwrap());
                        assert_eq!(got.new_path.as_deref(), exp["new_path"].as_str());
                    }
                }
            }
            other => panic!("unknown patch op: {other}"),
        }
    }
}

#[test]
fn replay_skills_frontmatter_oracle() {
    let cases = load_jsonl("skills_frontmatter.jsonl");
    assert_eq!(cases.len(), 7);

    for case in cases {
        match case["op"].as_str().unwrap() {
            "parse_skill" => {
                let raw = case["raw"].as_str().unwrap();
                let (fm, body) = parse_frontmatter(raw);
                let mut got_keys: Vec<String> = fm.keys().cloned().collect();
                got_keys.sort();
                let exp_keys: Vec<String> = case["frontmatter_keys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                assert_eq!(got_keys, exp_keys);
                assert_eq!(
                    extract_skill_description(&fm),
                    case["extracted_desc"].as_str().unwrap()
                );
                assert_eq!(
                    is_skill_description_truncated_for_prompt(&fm),
                    case["truncated"].as_bool().unwrap()
                );
                assert_eq!(body, case["body"].as_str().unwrap());
            }
            "clip_skill_description" => {
                let input = case["input"].as_str().unwrap();
                let expected = case["expected"].as_str().unwrap();
                assert_eq!(clip_skill_description(input), expected);
            }
            other => panic!("unknown skills op: {other}"),
        }
    }
}

#[test]
fn replay_prompt_injection_scan_oracle() {
    let cases = load_jsonl("prompt_injection_scan.jsonl");
    assert_eq!(cases.len(), 7);

    for case in cases {
        let name = case["name"].as_str().unwrap();
        let scope = ThreatScope::parse(case["scope"].as_str().unwrap()).unwrap();
        let user_authored = case["user_authored"].as_bool().unwrap();
        let content = case["content"].as_str().unwrap();

        let got_findings = scan_for_threats(content, scope);
        let exp_findings: Vec<String> = case["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(got_findings, exp_findings, "findings mismatch for {name}");

        let filtered = scan_context_content(content, &format!("{name}.md"), user_authored);
        assert_eq!(
            filtered.starts_with("[BLOCKED:"),
            case["blocked"].as_bool().unwrap(),
            "blocked flag mismatch for {name}"
        );
    }
}

#[tokio::test]
async fn replay_sqlite_fts_preview_oracle() {
    let cases = load_jsonl("sqlite_fts_preview.jsonl");
    assert_eq!(cases.len(), 5);

    for case in cases {
        match case["op"].as_str().unwrap() {
            "session_db_fts" => {
                assert_eq!(
                    FTS_TOOL_CONTENT_PREFIX_CHARS as u64,
                    case["fts_tool_prefix_chars"].as_u64().unwrap()
                );
                let tmp = TempDir::new().unwrap();
                let db = SqliteStore::open(&tmp.path().join("state.db")).unwrap();
                let sid = db
                    .create_session(
                        "oracle-sess-1",
                        "cli",
                        Some("claude-sonnet-4"),
                        Some("You are Saif."),
                        None,
                        None,
                    )
                    .await
                    .unwrap();
                db.append_message(
                    &sid,
                    "user",
                    Some("How do we configure Rust workspace caching in CI?"),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
                db.append_message(
                    &sid,
                    "assistant",
                    Some("Let me inspect Cargo.toml."),
                    None,
                    None,
                    None,
                    Some("Checking workspace layout."),
                    None,
                    None,
                )
                .await
                .unwrap();
                let big_tool_body = format!(
                    "{} TAIL_ONLY_NEEDLE",
                    "A".repeat(FTS_TOOL_CONTENT_PREFIX_CHARS + 500)
                );
                db.append_message(
                    &sid,
                    "tool",
                    Some(&big_tool_body),
                    Some("read_file"),
                    Some("call_1"),
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
                db.append_message(
                    &sid,
                    "assistant",
                    Some("Workspace caching uses Swatinem/rust-cache."),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();

                let hits_user = db.search_messages("workspace caching", 10).await.unwrap();
                let hits_tail = db.search_messages("TAIL_ONLY_NEEDLE", 10).await.unwrap();
                assert_eq!(
                    hits_user.len() as u64,
                    case["hits_user_count"].as_u64().unwrap()
                );
                assert_eq!(
                    hits_tail.len() as u64,
                    case["hits_tail_count"].as_u64().unwrap()
                );
            }
            "shape_preview" => {
                let input = case["input"].as_str().unwrap();
                let expected = case["expected"].as_str().unwrap();
                assert_eq!(shape_preview(input), expected);
            }
            other => panic!("unknown sqlite op: {other}"),
        }
    }
}

#[test]
fn replay_profile_paths_oracle() {
    let cases = load_jsonl("profile_paths.jsonl");
    assert_eq!(cases.len(), 6);

    let tmp = TempDir::new().unwrap();
    let profiles_root = tmp.path().join("profiles");
    fs::create_dir_all(&profiles_root).unwrap();

    for case in cases {
        let name = case["name"].as_str().unwrap();
        let pdir = profiles_root.join(name);
        fs::create_dir_all(&pdir).unwrap();
        match name {
            "empty_dir" => {}
            "empty_env_only" => fs::write(pdir.join(".env"), "").unwrap(),
            "non_empty_env" => fs::write(pdir.join(".env"), "OPENAI_API_KEY=sk-test\n").unwrap(),
            "config_yaml" => fs::write(pdir.join("config.yaml"), "model: gpt-5\n").unwrap(),
            "state_db_only" => fs::write(pdir.join("state.db"), "").unwrap(),
            "tombstoned" => {
                fs::write(pdir.join("config.yaml"), "model: gpt-5\n").unwrap();
                let del_dir = profiles_root.join(".deleted");
                fs::create_dir_all(&del_dir).unwrap();
                fs::write(del_dir.join(name), "deleted\n").unwrap();
            }
            other => panic!("unexpected profile oracle case {other}"),
        }

        assert_eq!(
            named_profile_has_identity(&pdir),
            case["has_identity"].as_bool().unwrap()
        );
        assert_eq!(
            named_profile_has_servable_identity(&pdir),
            case["has_servable_identity"].as_bool().unwrap()
        );
        assert_eq!(
            named_profile_is_deleted(&pdir),
            case["is_deleted"].as_bool().unwrap()
        );
        assert_eq!(
            named_profile_is_live(&pdir),
            case["is_live"].as_bool().unwrap()
        );
    }
}

#[test]
fn replay_codex_transport_oracle() {
    let cases = load_jsonl("codex_transport.jsonl");
    assert_eq!(cases.len(), 14);

    for case in cases {
        match case["op"].as_str().unwrap() {
            "bounded_prompt_cache_key" => {
                let got = bounded_prompt_cache_key(case["input"].as_str());
                assert_eq!(got.as_deref(), case["expected"].as_str());
            }
            "cache_scope_from_session_id" => {
                let got = cache_scope_from_session_id(case["session_id"].as_str().unwrap());
                assert_eq!(got, case["expected"].as_str().unwrap());
            }
            "alias_reserved_tools" => {
                let tools: Vec<ToolDefinition> = case["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| ToolDefinition {
                        name: n.as_str().unwrap().to_string(),
                        description: "d".to_string(),
                        parameters: json!({ "type": "object" }),
                    })
                    .collect();
                let reserved: Vec<&str> = case["reserved"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_str().unwrap())
                    .collect();
                let (rewritten, alias_map) = alias_reserved_tools(&tools, &reserved);
                let got_names: Vec<String> = rewritten.into_iter().map(|t| t.name).collect();
                let exp_names: Vec<String> = case["expected_tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_str().unwrap().to_string())
                    .collect();
                assert_eq!(got_names, exp_names);
                let exp_map: BTreeMap<String, String> =
                    serde_json::from_value(case["expected_alias_map"].clone()).unwrap();
                assert_eq!(alias_map, exp_map);
            }
            other => panic!("unexpected codex oracle op {other}"),
        }
    }
}

#[test]
fn replay_batch_sharegpt_oracle() {
    let cases = load_jsonl("batch_sharegpt.jsonl");
    assert_eq!(cases.len(), 9);

    for case in cases {
        assert_eq!(case["op"].as_str().unwrap(), "tool_call_succeeded");
        let name = case["name"].as_str().unwrap();
        let content = case["content"].as_str().unwrap();
        let expected = case["expected"].as_bool().unwrap();
        assert_eq!(
            tool_call_succeeded(content),
            expected,
            "mismatch for {name}"
        );
    }
}

#[test]
fn replay_media_exec_helpers_oracle() {
    let cases = load_jsonl("media_exec_helpers.jsonl");
    assert_eq!(cases.len(), 18);

    for case in cases {
        match case["op"].as_str().unwrap() {
            "tts_response_format_from_path" => {
                let got = tts_response_format_from_path(case["path"].as_str().unwrap());
                assert_eq!(got, case["expected"].as_str().unwrap());
            }
            "config_bool" => {
                let got = config_bool(
                    case["raw"].as_str().unwrap(),
                    case["default"].as_bool().unwrap(),
                );
                assert_eq!(got, case["expected"].as_bool().unwrap());
            }
            "truncate_stdout_text" => {
                let n = case["input_bytes"].as_u64().unwrap() as usize;
                let text = "A".repeat(n);
                let (out, meta) = truncate_stdout_bytes(&text, None);
                assert_eq!(
                    meta["stdout_truncated"].as_bool().unwrap(),
                    case["expected_truncated"].as_bool().unwrap()
                );
                assert_eq!(
                    meta["stdout_bytes_captured"].as_u64().unwrap(),
                    case["expected_bytes_captured"].as_u64().unwrap()
                );
                assert_eq!(
                    meta["stdout_bytes_total"].as_u64().unwrap(),
                    case["expected_bytes_total"].as_u64().unwrap()
                );
                assert_eq!(
                    meta["stdout_bytes_omitted"].as_u64().unwrap(),
                    case["expected_bytes_omitted"].as_u64().unwrap()
                );
                assert_eq!(
                    out.to_ascii_lowercase().contains("truncated"),
                    case["has_truncation_marker"].as_bool().unwrap()
                );
            }
            other => panic!("unexpected media/exec oracle op {other}"),
        }
    }
}
