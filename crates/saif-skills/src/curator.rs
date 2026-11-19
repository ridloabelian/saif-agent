use crate::store::{SkillError, SkillStore};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillLifecycleState {
    Active,
    Stale,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageRecord {
    pub use_count: u64,
    pub view_count: u64,
    pub patch_count: u64,
    pub last_activity_at: u64,
    pub state: SkillLifecycleState,
    pub pinned: bool,
}

impl Default for SkillUsageRecord {
    fn default() -> Self {
        Self {
            use_count: 0,
            view_count: 0,
            patch_count: 0,
            last_activity_at: now_secs(),
            state: SkillLifecycleState::Active,
            pinned: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorReport {
    pub backup_archive: Option<PathBuf>,
    pub marked_stale: Vec<String>,
    pub archived: Vec<String>,
    pub skipped_pinned: Vec<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct Curator {
    skills_dir: PathBuf,
    usage_file: PathBuf,
}

impl Curator {
    pub fn new(skills_dir: PathBuf) -> Self {
        let usage_file = skills_dir.join(".usage.json");
        Self {
            skills_dir,
            usage_file,
        }
    }

    pub fn load_usage(&self) -> BTreeMap<String, SkillUsageRecord> {
        let Ok(raw) = fs::read_to_string(&self.usage_file) else {
            return BTreeMap::new();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save_usage(&self, map: &BTreeMap<String, SkillUsageRecord>) -> Result<(), SkillError> {
        fs::create_dir_all(&self.skills_dir)?;
        let raw = serde_json::to_string_pretty(map).unwrap_or_else(|_| "{}".to_string());
        fs::write(&self.usage_file, raw)?;
        Ok(())
    }

    pub fn record_view(&self, skill_name: &str) -> Result<(), SkillError> {
        let mut usage = self.load_usage();
        let entry = usage.entry(skill_name.to_string()).or_default();
        entry.view_count += 1;
        entry.use_count += 1;
        entry.last_activity_at = now_secs();
        entry.state = SkillLifecycleState::Active;
        self.save_usage(&usage)
    }

    pub fn record_patch(&self, skill_name: &str) -> Result<(), SkillError> {
        let mut usage = self.load_usage();
        let entry = usage.entry(skill_name.to_string()).or_default();
        entry.patch_count += 1;
        entry.last_activity_at = now_secs();
        entry.state = SkillLifecycleState::Active;
        self.save_usage(&usage)
    }

    pub fn set_pinned(&self, skill_name: &str, pinned: bool) -> Result<(), SkillError> {
        let mut usage = self.load_usage();
        let entry = usage.entry(skill_name.to_string()).or_default();
        entry.pinned = pinned;
        self.save_usage(&usage)
    }

    pub fn is_pinned(&self, skill_name: &str) -> bool {
        self.load_usage().get(skill_name).is_some_and(|r| r.pinned)
    }

    pub fn create_backup_snapshot(&self) -> Result<Option<PathBuf>, SkillError> {
        if !self.skills_dir.exists() {
            return Ok(None);
        }
        let backup_dir = self.skills_dir.join(".curator_backups");
        fs::create_dir_all(&backup_dir)?;
        let out_path = backup_dir.join(format!("skills-backup-{}.tar.gz", now_secs()));
        let tar_gz = fs::File::create(&out_path)?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut builder = tar::Builder::new(enc);
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                builder.append_dir_all(name, &path)?;
            } else if path.is_file() {
                builder.append_path_with_name(&path, name)?;
            }
        }
        builder.finish()?;
        Ok(Some(out_path))
    }

    /// Run curator maintenance: never deletes skills, respects `pinned` & `created_by == "agent"`,
    /// archives stale skills into `<skills>/.archive/` after creating a `.tar.gz` snapshot.
    pub fn run_cycle(
        &self,
        store: &SkillStore,
        stale_after_days: u64,
        archive_after_days: u64,
        backup_enabled: bool,
    ) -> Result<CuratorReport, SkillError> {
        let now = now_secs();
        let stale_secs = stale_after_days.saturating_mul(86_400);
        let archive_secs = archive_after_days.saturating_mul(86_400);
        let mut usage = self.load_usage();

        let backup_archive = if backup_enabled {
            self.create_backup_snapshot()?
        } else {
            None
        };

        let mut report = CuratorReport {
            backup_archive,
            marked_stale: Vec::new(),
            archived: Vec::new(),
            skipped_pinned: Vec::new(),
        };

        for doc in store.list_skills() {
            if doc.metadata.created_by.as_deref() != Some("agent") {
                continue;
            }
            let rec = usage.entry(doc.name.clone()).or_default();
            if rec.pinned {
                report.skipped_pinned.push(doc.name.clone());
                continue;
            }
            let idle = now.saturating_sub(rec.last_activity_at);
            if idle >= archive_secs && rec.state != SkillLifecycleState::Archived {
                if let Some(skill_dir) = doc.path.parent() {
                    let archive_root = self.skills_dir.join(".archive");
                    fs::create_dir_all(&archive_root)?;
                    let target = archive_root.join(&doc.name);
                    if !target.exists() {
                        move_dir_copy_remove(skill_dir, &target)?;
                    }
                }
                rec.state = SkillLifecycleState::Archived;
                report.archived.push(doc.name.clone());
            } else if idle >= stale_secs && rec.state == SkillLifecycleState::Active {
                rec.state = SkillLifecycleState::Stale;
                report.marked_stale.push(doc.name.clone());
            }
        }

        self.save_usage(&usage)?;
        Ok(report)
    }
}

fn move_dir_copy_remove(src: &Path, dst: &Path) -> std::io::Result<()> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let s = entry.path();
        let d = dst.join(entry.file_name());
        if s.is_dir() {
            move_dir_copy_remove(&s, &d)?;
        } else {
            fs::copy(&s, &d)?;
        }
    }
    fs::remove_dir_all(src)
}
