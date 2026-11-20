use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn saif_bin() -> &'static str {
    env!("CARGO_BIN_EXE_saif")
}

#[test]
fn cli_version_and_doctor_run_cleanly() {
    let tmp = TempDir::new().unwrap();
    let ver = Command::new(saif_bin())
        .arg("--version")
        .env("SAIF_HOME", tmp.path())
        .output()
        .unwrap();
    assert!(ver.status.success());
    let stdout = String::from_utf8_lossy(&ver.stdout);
    assert!(stdout.contains("saif"));

    let doc = Command::new(saif_bin())
        .arg("doctor")
        .env("SAIF_HOME", tmp.path())
        .output()
        .unwrap();
    assert!(doc.status.success());
    let doc_out = String::from_utf8_lossy(&doc.stdout);
    assert!(doc_out.contains("36 built-in profiles"));
}

#[test]
fn cli_status_cron_kanban_profile_and_migrate_e2e() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("saif_home");
    let hermes_src = tmp.path().join("hermes_fixture");
    fs::create_dir_all(&hermes_src).unwrap();
    fs::write(hermes_src.join("config.yaml"), "model: claude-sonnet-4\n").unwrap();

    let status = Command::new(saif_bin())
        .arg("status")
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(status.status.success());

    let prof_create = Command::new(saif_bin())
        .args(["profile", "create", "coder"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(prof_create.status.success());

    let prof_list = Command::new(saif_bin())
        .args(["profile", "list"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(prof_list.status.success());
    assert!(String::from_utf8_lossy(&prof_list.stdout).contains("coder"));

    let cron_add = Command::new(saif_bin())
        .args([
            "cron",
            "add",
            "daily-check",
            "every monday 9am",
            "Check logs",
        ])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(cron_add.status.success());
    assert!(String::from_utf8_lossy(&cron_add.stdout).contains("0 9 * * 1"));

    let kb_init = Command::new(saif_bin())
        .args(["kanban", "init"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(kb_init.status.success());

    let kb_create = Command::new(saif_bin())
        .args(["kanban", "create", "Port gateway", "--assignee", "coder"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(kb_create.status.success());
    assert!(String::from_utf8_lossy(&kb_create.stdout).contains("Port gateway"));

    let mig = Command::new(saif_bin())
        .args([
            "migrate",
            "--from-hermes",
            "--source",
            hermes_src.to_str().unwrap(),
            "--dry-run",
        ])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(mig.status.success());
    assert!(String::from_utf8_lossy(&mig.stdout).contains("config.yaml"));

    let platforms = Command::new(saif_bin())
        .args(["gateway", "platforms"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(platforms.status.success());
    let plat_out = String::from_utf8_lossy(&platforms.stdout);
    assert_eq!(plat_out.lines().count(), 30);

    let gw_mig = Command::new(saif_bin())
        .args(["gateway", "migrate", "--multiplex"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(gw_mig.status.success());
    assert!(String::from_utf8_lossy(&gw_mig.stdout).contains("converged"));

    let svc = Command::new(saif_bin())
        .args(["gateway", "install-service", "--kind", "systemd"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(svc.status.success());
    assert!(String::from_utf8_lossy(&svc.stdout).contains("ExecStart="));

    let auth_add = Command::new(saif_bin())
        .args(["auth", "add", "openai", "k1", "sk-pool-1"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(auth_add.status.success());
    let auth_list = Command::new(saif_bin())
        .args(["auth", "list"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(auth_list.status.success());
    assert!(String::from_utf8_lossy(&auth_list.stdout).contains("k1"));

    let auth_strat = Command::new(saif_bin())
        .args(["auth", "strategy", "round_robin"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(auth_strat.status.success());
    assert!(String::from_utf8_lossy(&auth_strat.stdout).contains("RoundRobin"));

    let mcp_list = Command::new(saif_bin())
        .args(["mcp", "list"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(mcp_list.status.success());
}

#[test]
fn cli_upstream_status_diff_ack_with_local_git_fixture() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("fake_hermes");
    fs::create_dir_all(repo.join("agent")).unwrap();
    fs::create_dir_all(repo.join("plugin-catalog")).unwrap();

    let run_git = |args: &[&str]| -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    run_git(&["init", "-b", "main"]);
    fs::write(repo.join("agent/redact_db_url.py"), "# v1\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "initial"]);
    let sha1 = run_git(&["rev-parse", "HEAD"]);

    fs::write(repo.join("agent/redact_db_url.py"), "# v2\n").unwrap();
    fs::write(repo.join("plugin-catalog/item.yaml"), "name: x\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "update redact + catalog"]);
    let sha2 = run_git(&["rev-parse", "HEAD"]);

    let orig_lock_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/UPSTREAM_LOCK.json");
    let mut lock_val: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&orig_lock_path).unwrap()).unwrap();
    lock_val["synced_commit"] = serde_json::Value::String(sha1);
    let lock_copy = tmp.path().join("UPSTREAM_LOCK.json");
    fs::write(&lock_copy, serde_json::to_string_pretty(&lock_val).unwrap()).unwrap();

    let saif_home = tmp.path().join("saif_home");
    let st1 = Command::new(saif_bin())
        .args([
            "upstream",
            "status",
            "--lock",
            lock_copy.to_str().unwrap(),
            "--local-repo",
            repo.to_str().unwrap(),
        ])
        .env("SAIF_HOME", &saif_home)
        .output()
        .unwrap();
    assert!(st1.status.success());
    assert!(String::from_utf8_lossy(&st1.stdout).contains("UPDATE_AVAILABLE"));

    let diff = Command::new(saif_bin())
        .args([
            "upstream",
            "diff",
            "--lock",
            lock_copy.to_str().unwrap(),
            "--local-repo",
            repo.to_str().unwrap(),
            "--port-only",
            "--json",
        ])
        .env("SAIF_HOME", &saif_home)
        .output()
        .unwrap();
    assert!(diff.status.success());
    let report: serde_json::Value = serde_json::from_slice(&diff.stdout).unwrap();
    assert_eq!(report["port_required_count"], 1);
    assert_eq!(report["ignored_noise_count"], 1);

    let ack = Command::new(saif_bin())
        .args([
            "upstream",
            "ack",
            &sha2,
            "--subject",
            "update redact + catalog",
            "--lock",
            lock_copy.to_str().unwrap(),
        ])
        .env("SAIF_HOME", &saif_home)
        .output()
        .unwrap();
    assert!(ack.status.success());

    let st2 = Command::new(saif_bin())
        .args([
            "upstream",
            "status",
            "--lock",
            lock_copy.to_str().unwrap(),
            "--local-repo",
            repo.to_str().unwrap(),
        ])
        .env("SAIF_HOME", &saif_home)
        .output()
        .unwrap();
    assert!(st2.status.success());
    assert!(String::from_utf8_lossy(&st2.stdout).contains("UP_TO_DATE"));
}

#[test]
fn cli_cron_tick_executes_due_job_and_records_output() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"cron-tick-executed"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":3}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("saif_cron_home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(".env"),
        format!("CRON_LLM_URL=http://{addr}/v1\n"),
    )
    .unwrap();
    fs::write(
        home.join("config.yaml"),
        "provider: custom\nmodel: cron-mock\nbase_url: \"${CRON_LLM_URL}\"\n",
    )
    .unwrap();

    let add = Command::new(saif_bin())
        .args(["cron", "add", "smoke-cron", "30m", "Check health"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(add.status.success());
    let created: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    let job_id = created["id"].as_str().unwrap().to_string();

    // Force job due by rewriting next_run_at into the past
    let jobs_path = home.join("cron/jobs.json");
    let mut jobs_doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&jobs_path).unwrap()).unwrap();
    jobs_doc["jobs"][0]["next_run_at"] = serde_json::Value::String("2020-01-01T00:00:00Z".into());
    fs::write(&jobs_path, serde_json::to_string_pretty(&jobs_doc).unwrap()).unwrap();

    let tick = Command::new(saif_bin())
        .args(["cron", "tick"])
        .env("SAIF_HOME", &home)
        .output()
        .unwrap();
    assert!(
        tick.status.success(),
        "cron tick stderr: {}",
        String::from_utf8_lossy(&tick.stderr)
    );
    assert!(String::from_utf8_lossy(&tick.stdout).contains("Due jobs: 1"));
    let _ = srv_handle.join();

    let after_jobs: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&jobs_path).unwrap()).unwrap();
    assert_eq!(after_jobs["jobs"][0]["last_status"], "ok");

    let output_json = fs::read_to_string(home.join(format!("cron/output/{job_id}.json"))).unwrap();
    assert!(output_json.contains("cron-tick-executed"));
}
