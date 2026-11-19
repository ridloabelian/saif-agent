use crate::frontmatter::{
    extract_metadata, extract_skill_description, parse_frontmatter, validate_skill_description,
    SkillMetadata,
};
use crate::threat::{first_threat_message, ThreatScope};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".github",
    ".hub",
    ".archive",
    ".curator_backups",
    ".locks",
    ".venv",
    "venv",
    "node_modules",
    "__pycache__",
];

const SUPPORT_DIRS: &[&str] = &["references", "templates", "assets", "scripts"];

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("skill '{0}' is pinned and cannot be deleted")]
    PinnedProtected(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDocument {
    pub name: String,
    pub path: PathBuf,
    pub category: Option<String>,
    pub prompt_description: String,
    pub metadata: SkillMetadata,
    pub body: String,
    pub raw: String,
}

#[derive(Clone)]
pub struct SkillStore {
    skills_dir: PathBuf,
    disabled_skills: HashSet<String>,
}

impl SkillStore {
    pub fn new(skills_dir: PathBuf, disabled: &[String]) -> Self {
        Self {
            skills_dir,
            disabled_skills: disabled.iter().cloned().collect(),
        }
    }

    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    pub fn list_skills(&self) -> Vec<SkillDocument> {
        let mut files = Vec::new();
        collect_skill_files(&self.skills_dir, &mut files);
        files.sort();

        let mut docs = Vec::new();
        for path in files {
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let (fm, body) = parse_frontmatter(&raw);
            let meta = extract_metadata(&fm);
            let name = meta.name.clone().unwrap_or_else(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("unnamed")
                    .to_string()
            });
            if self.disabled_skills.contains(&name) {
                continue;
            }
            let prompt_description = extract_skill_description(&fm);
            docs.push(SkillDocument {
                name,
                path,
                category: meta.category.clone(),
                prompt_description,
                metadata: meta,
                body,
                raw,
            });
        }
        docs
    }

    pub fn view_skill(&self, name: &str) -> Result<SkillDocument, SkillError> {
        if self.disabled_skills.contains(name) {
            return Err(SkillError::NotFound(name.to_string()));
        }
        self.list_skills()
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))
    }

    pub fn create_skill(
        &self,
        name: &str,
        description: &str,
        body: &str,
        category: Option<&str>,
    ) -> Result<PathBuf, SkillError> {
        validate_skill_name(name)?;
        validate_skill_description(description).map_err(SkillError::Validation)?;
        let content = format!(
            "---\nname: {name}\ndescription: {description}\ncreated_by: agent\n{}---\n\n{body}\n",
            category
                .map(|c| format!("category: {c}\n"))
                .unwrap_or_default()
        );
        if let Some(threat_msg) = first_threat_message(&content, ThreatScope::Strict) {
            return Err(SkillError::Validation(threat_msg));
        }

        let dir = self.skills_dir.join(name);
        fs::create_dir_all(&dir)?;
        let skill_md = dir.join("SKILL.md");
        fs::write(&skill_md, content)?;
        Ok(skill_md)
    }

    pub fn patch_skill(
        &self,
        name: &str,
        old_text: &str,
        new_text: &str,
    ) -> Result<PathBuf, SkillError> {
        let doc = self.view_skill(name)?;
        if !doc.raw.contains(old_text) {
            return Err(SkillError::Validation(format!(
                "old_text not found in skill '{name}'"
            )));
        }
        let updated = doc.raw.replacen(old_text, new_text, 1);
        let (fm, _) = parse_frontmatter(&updated);
        let new_desc = fm
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if doc.metadata.description.as_deref() != Some(new_desc) {
            validate_skill_description(new_desc).map_err(SkillError::Validation)?;
        }
        if let Some(threat_msg) = first_threat_message(&updated, ThreatScope::Strict) {
            return Err(SkillError::Validation(threat_msg));
        }
        fs::write(&doc.path, updated)?;
        Ok(doc.path)
    }

    pub fn delete_skill(&self, name: &str, pinned: bool) -> Result<(), SkillError> {
        if pinned {
            return Err(SkillError::PinnedProtected(name.to_string()));
        }
        let doc = self.view_skill(name)?;
        if let Some(parent) = doc.path.parent() {
            fs::remove_dir_all(parent)?;
        }
        Ok(())
    }
}

fn validate_skill_name(name: &str) -> Result<(), SkillError> {
    if name.trim().is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(SkillError::Validation(format!(
            "invalid skill name: {name}"
        )));
    }
    Ok(())
}

fn collect_skill_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let skill_md = dir.join("SKILL.md");
    let has_skill_md = skill_md.is_file();
    if has_skill_md {
        out.push(skill_md);
    }
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if EXCLUDED_DIRS.contains(&name) {
            continue;
        }
        if has_skill_md && SUPPORT_DIRS.contains(&name) {
            continue;
        }
        collect_skill_files(&path, out);
    }
}
