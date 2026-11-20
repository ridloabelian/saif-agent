use crate::secrets::SecretScope;
use regex::Regex;
use std::env;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use thiserror::Error;

pub const PROFILE_IDENTITY_MARKERS: &[&str] = &[
    "config.yaml",
    ".env",
    "SOUL.md",
    "profile.yaml",
    "auth.json",
    "state.db",
];

pub const DELETED_PROFILES_DIR: &str = ".deleted";

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("invalid profile id '{0}': must match ^[a-z0-9][a-z0-9_-]{{0,63}}$")]
    InvalidProfileId(String),
    #[error("named profile home does not exist or is tombstoned: {0}")]
    MissingOrTombstoned(PathBuf),
    #[error("profile home boundary blocked by non-directory file: {0}")]
    FileAtHomeBoundary(PathBuf),
    #[error("profile I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn profile_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9_-]{0,63}$").unwrap_or_else(|_| unreachable!()))
}

pub fn validate_profile_id(name: &str) -> Result<(), ProfileError> {
    if name == "default" || !profile_id_re().is_match(name) {
        return Err(ProfileError::InvalidProfileId(name.to_string()));
    }
    Ok(())
}

/// Platform default root `~/.saif` (or `$SAIF_HOME` if set).
pub fn default_saif_root() -> PathBuf {
    if let Ok(explicit) = env::var("SAIF_HOME") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return expand_tilde(trimmed);
        }
    }
    user_home_dir().join(".saif")
}

pub fn user_home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" {
        return user_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return user_home_dir().join(rest);
    }
    PathBuf::from(raw)
}

pub fn profile_tombstone_path(profile_home: &Path) -> PathBuf {
    let parent = profile_home.parent().unwrap_or(profile_home);
    let name = profile_home
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    parent.join(DELETED_PROFILES_DIR).join(name)
}

pub fn named_profile_is_deleted(profile_home: &Path) -> bool {
    profile_tombstone_path(profile_home).exists()
}

pub fn mark_named_profile_deleted(profile_home: &Path) -> std::io::Result<()> {
    let marker = profile_tombstone_path(profile_home);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(marker, "deleted\n")
}

pub fn clear_named_profile_deleted(profile_home: &Path) -> std::io::Result<()> {
    let marker = profile_tombstone_path(profile_home);
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    Ok(())
}

/// Matches Hermes `named_profile_has_identity`: returns true if any identity marker
/// file OR symlink (including dangling symlink) exists inside `profile_home`.
pub fn named_profile_has_identity(profile_home: &Path) -> bool {
    PROFILE_IDENTITY_MARKERS.iter().any(|marker| {
        let p = profile_home.join(marker);
        match fs::symlink_metadata(&p) {
            Ok(meta) => meta.file_type().is_symlink() || meta.is_file(),
            Err(_) => false,
        }
    })
}

/// Matches Hermes `named_profile_has_servable_identity`: an empty `.env` alone
/// (left by an aborted create) does NOT flip a host into multiplex mode.
pub fn named_profile_has_servable_identity(profile_home: &Path) -> bool {
    for marker in PROFILE_IDENTITY_MARKERS {
        let p = profile_home.join(marker);
        let Ok(meta) = fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            return true;
        }
        if meta.is_file() && (*marker != ".env" || meta.len() > 0) {
            return true;
        }
    }
    false
}

/// Matches Hermes `named_profile_is_live`.
pub fn named_profile_is_live(profile_home: &Path) -> bool {
    profile_home.is_dir()
        && named_profile_has_identity(profile_home)
        && !named_profile_is_deleted(profile_home)
}

pub fn list_live_profiles(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let profiles_dir = root.join("profiles");
    let Ok(entries) = fs::read_dir(profiles_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if named_profile_is_live(&path) {
            out.push((name.to_string(), path));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn resolve_saif_home(profile: Option<&str>) -> Result<PathBuf, ProfileError> {
    let root = default_saif_root();
    match profile {
        None | Some("default") | Some("") => Ok(root),
        Some(name) => {
            validate_profile_id(name)?;
            Ok(root.join("profiles").join(name))
        }
    }
}

/// Explicit per-activity scope carrying profile home, profile name, and fail-closed `SecretScope`.
#[derive(Debug, Clone)]
pub struct ProfileScope {
    pub profile_name: String,
    pub home_dir: PathBuf,
    pub root_dir: PathBuf,
    pub is_default: bool,
    pub secrets: SecretScope,
}

tokio::task_local! {
    static ACTIVE_PROFILE_SCOPE: Arc<ProfileScope>;
}

impl ProfileScope {
    pub fn for_root(root_dir: PathBuf, multiplex_active: bool) -> Self {
        let secrets = SecretScope::load_from_home(&root_dir, true, multiplex_active);
        Self {
            profile_name: "default".to_string(),
            home_dir: root_dir.clone(),
            root_dir,
            is_default: true,
            secrets,
        }
    }

    pub fn for_profile(
        root_dir: PathBuf,
        profile_name: Option<&str>,
        multiplex_active: bool,
    ) -> Result<Self, ProfileError> {
        match profile_name {
            None | Some("default") | Some("") => Ok(Self::for_root(root_dir, multiplex_active)),
            Some(name) => {
                validate_profile_id(name)?;
                let home_dir = root_dir.join("profiles").join(name);
                if named_profile_is_deleted(&home_dir) {
                    return Err(ProfileError::MissingOrTombstoned(home_dir));
                }
                if fs::symlink_metadata(&home_dir).is_ok() && !home_dir.is_dir() {
                    return Err(ProfileError::FileAtHomeBoundary(home_dir));
                }
                let secrets = SecretScope::load_from_home(&home_dir, false, multiplex_active);
                Ok(Self {
                    profile_name: name.to_string(),
                    home_dir,
                    root_dir,
                    is_default: false,
                    secrets,
                })
            }
        }
    }

    pub fn detect_multiplex_active(root_dir: &Path) -> bool {
        list_live_profiles(root_dir)
            .iter()
            .any(|(_, p)| named_profile_has_servable_identity(p))
    }

    pub async fn run_scoped<F, T>(scope: Arc<ProfileScope>, fut: F) -> T
    where
        F: Future<Output = T>,
    {
        ACTIVE_PROFILE_SCOPE.scope(scope, fut).await
    }

    pub fn current() -> Option<Arc<ProfileScope>> {
        ACTIVE_PROFILE_SCOPE.try_with(Arc::clone).ok()
    }

    pub fn config_path(&self) -> PathBuf {
        self.home_dir.join("config.yaml")
    }

    pub fn env_path(&self) -> PathBuf {
        self.home_dir.join(".env")
    }

    pub fn state_db_path(&self) -> PathBuf {
        self.home_dir.join("state.db")
    }

    pub fn kanban_db_path(&self) -> PathBuf {
        self.root_dir.join("kanban.db")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.home_dir.join("skills")
    }

    pub fn cron_dir(&self) -> PathBuf {
        self.home_dir.join("cron")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.home_dir.join("plugins")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.home_dir.join("logs")
    }

    pub fn memories_dir(&self) -> PathBuf {
        self.home_dir.join("memories")
    }

    pub fn ensure_layout(&self) -> std::io::Result<()> {
        if !self.is_default && named_profile_is_deleted(&self.home_dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to recreate tombstoned profile home: {}",
                    self.home_dir.display()
                ),
            ));
        }
        if fs::symlink_metadata(&self.home_dir).is_ok() && !self.home_dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "home boundary exists as non-directory: {}",
                    self.home_dir.display()
                ),
            ));
        }
        for dir in [
            &self.home_dir,
            &self.skills_dir(),
            &self.cron_dir(),
            &self.plugins_dir(),
            &self.logs_dir(),
            &self.memories_dir(),
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}
