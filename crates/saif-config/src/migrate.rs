use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub dry_run: bool,
    pub source_home: PathBuf,
    pub target_home: PathBuf,
    pub copied_files: Vec<PathBuf>,
    pub skipped_existing: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

const MIGRATABLE_FILES: &[&str] = &[
    "config.yaml",
    ".env",
    "SOUL.md",
    "auth.json",
    "state.db",
    "kanban.db",
];

const MIGRATABLE_DIRS: &[&str] = &["memories", "skills", "cron", "skins", "locales"];

/// Non-destructive migration from an existing `~/.hermes` home into `~/.saif`.
/// Never deletes or modifies anything inside `source_home`, and skips already-existing destination files.
pub fn migrate_from_hermes(
    source_home: &Path,
    target_home: &Path,
    dry_run: bool,
) -> std::io::Result<MigrationReport> {
    let mut report = MigrationReport {
        dry_run,
        source_home: source_home.to_path_buf(),
        target_home: target_home.to_path_buf(),
        copied_files: Vec::new(),
        skipped_existing: Vec::new(),
        warnings: Vec::new(),
    };

    if !source_home.exists() {
        report.warnings.push(format!(
            "Source Hermes home does not exist: {}",
            source_home.display()
        ));
        return Ok(report);
    }

    if !dry_run {
        fs::create_dir_all(target_home)?;
    }

    for fname in MIGRATABLE_FILES {
        let src = source_home.join(fname);
        let dst = target_home.join(fname);
        copy_if_missing(&src, &dst, dry_run, &mut report)?;
    }

    for dname in MIGRATABLE_DIRS {
        let src = source_home.join(dname);
        let dst = target_home.join(dname);
        if src.is_dir() {
            copy_dir_non_destructive(&src, &dst, dry_run, &mut report)?;
        }
    }

    Ok(report)
}

fn copy_if_missing(
    src: &Path,
    dst: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> std::io::Result<()> {
    if !src.is_file() {
        return Ok(());
    }
    if dst.exists() {
        report.skipped_existing.push(dst.to_path_buf());
        return Ok(());
    }
    if !dry_run {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
    }
    report.copied_files.push(dst.to_path_buf());
    Ok(())
}

fn copy_dir_non_destructive(
    src: &Path,
    dst: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> std::io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let s = entry.path();
        let d = dst.join(entry.file_name());
        if s.is_dir() {
            copy_dir_non_destructive(&s, &d, dry_run, report)?;
        } else if s.is_file() {
            copy_if_missing(&s, &d, dry_run, report)?;
        }
    }
    Ok(())
}
