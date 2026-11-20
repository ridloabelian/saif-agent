use crate::profile::{named_profile_is_live, validate_profile_id};
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
    #[serde(default)]
    pub migrated_profiles: Vec<String>,
    pub warnings: Vec<String>,
}

const MIGRATABLE_FILES: &[&str] = &[
    "config.yaml",
    ".env",
    "SOUL.md",
    "profile.yaml",
    "auth.json",
    "state.db",
    "kanban.db",
];

const MIGRATABLE_DIRS: &[&str] = &[
    "memories", "skills", "cron", "skins", "locales", "plugins", "scripts",
];

/// Non-destructive migration from an existing `~/.hermes` home (and live `profiles/<name>/`) into `~/.saif`.
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
        migrated_profiles: Vec::new(),
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

    migrate_single_home(source_home, target_home, dry_run, &mut report)?;

    let src_profiles = source_home.join("profiles");
    if src_profiles.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&src_profiles)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let pdir = entry.path();
            let Some(name) = pdir.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('.') || validate_profile_id(name).is_err() {
                continue;
            }
            if !named_profile_is_live(&pdir) {
                continue;
            }
            let dst_profile = target_home.join("profiles").join(name);
            if !dry_run {
                fs::create_dir_all(&dst_profile)?;
            }
            migrate_single_home(&pdir, &dst_profile, dry_run, &mut report)?;
            report.migrated_profiles.push(name.to_string());
        }
    }

    Ok(report)
}

fn migrate_single_home(
    source_home: &Path,
    target_home: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> std::io::Result<()> {
    for fname in MIGRATABLE_FILES {
        let src = source_home.join(fname);
        let dst = target_home.join(fname);
        copy_if_missing(&src, &dst, dry_run, report)?;
    }

    for dname in MIGRATABLE_DIRS {
        let src = source_home.join(dname);
        let dst = target_home.join(dname);
        if src.is_dir() {
            copy_dir_non_destructive(&src, &dst, dry_run, report)?;
        }
    }
    Ok(())
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
