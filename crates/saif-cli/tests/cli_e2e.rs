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
    assert!(doc_out.contains("24 built-in profiles"));
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
}
