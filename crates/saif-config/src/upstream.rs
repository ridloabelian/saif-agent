use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemRule {
    pub id: String,
    pub description: String,
    pub hermes_prefixes: Vec<String>,
    pub rust_targets: Vec<String>,
    #[serde(default)]
    pub conformance_suites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamLock {
    pub schema_version: u32,
    pub upstream_repo: String,
    pub tracked_branch: String,
    pub synced_commit: String,
    pub synced_at: String,
    #[serde(default)]
    pub synced_subject: String,
    #[serde(default)]
    pub ignore_globs: Vec<String>,
    #[serde(default)]
    pub runtime_specific_prefixes: Vec<String>,
    #[serde(default)]
    pub subsystems: Vec<SubsystemRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangedFileEntry {
    pub status: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "classification", rename_all = "snake_case")]
pub enum FileClassification {
    IgnoredNoise {
        path: String,
        reason: String,
    },
    SkipRuntimeSpecific {
        path: String,
        reason: String,
    },
    PortRequired {
        path: String,
        status: String,
        subsystem_id: String,
        rust_targets: Vec<String>,
        conformance_suites: Vec<String>,
    },
    Unmapped {
        path: String,
        status: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemDelta {
    pub subsystem_id: String,
    pub description: String,
    pub rust_targets: Vec<String>,
    pub conformance_suites: Vec<String>,
    pub files: Vec<ChangedFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamDeltaReport {
    pub from_commit: String,
    pub to_commit: String,
    pub total_changed_files: usize,
    pub port_required_count: usize,
    pub skipped_runtime_count: usize,
    pub ignored_noise_count: usize,
    pub unmapped_count: usize,
    pub by_subsystem: BTreeMap<String, SubsystemDelta>,
    pub unmapped: Vec<ChangedFileEntry>,
    pub skipped_runtime: Vec<String>,
    pub ignored_noise: Vec<String>,
}

impl UpstreamLock {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("failed reading upstream lock {}: {e}", path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("failed parsing upstream lock {}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let body = serde_json::to_string_pretty(self).map_err(|e| e.to_string())? + "\n";
        let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        fs::rename(tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn ack(&mut self, sha: &str, subject: Option<&str>) -> Result<(), String> {
        let trimmed = sha.trim();
        if trimmed.len() != 40 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid commit SHA '{trimmed}': expected 40 hex characters"
            ));
        }
        self.synced_commit = trimmed.to_ascii_lowercase();
        self.synced_at = Utc::now().to_rfc3339();
        if let Some(subj) = subject {
            self.synced_subject = subj.to_string();
        }
        Ok(())
    }

    pub fn classify_path(&self, path: &str, status: &str) -> FileClassification {
        let norm = path.trim().trim_start_matches("./");
        for pat in &self.ignore_globs {
            if glob_matches(pat, norm) {
                return FileClassification::IgnoredNoise {
                    path: norm.to_string(),
                    reason: format!("matched ignore glob '{pat}'"),
                };
            }
        }
        for prefix in &self.runtime_specific_prefixes {
            if norm.starts_with(prefix.as_str()) {
                return FileClassification::SkipRuntimeSpecific {
                    path: norm.to_string(),
                    reason: format!("matched runtime-specific prefix '{prefix}'"),
                };
            }
        }

        let mut best_match: Option<(usize, &SubsystemRule)> = None;
        for rule in &self.subsystems {
            for pref in &rule.hermes_prefixes {
                let matched = if pref.ends_with('/') {
                    norm.starts_with(pref.as_str())
                } else {
                    norm == pref.as_str() || norm.starts_with(pref.as_str())
                };
                if matched {
                    let score = pref.len();
                    if best_match.is_none_or(|(best_len, _)| score > best_len) {
                        best_match = Some((score, rule));
                    }
                }
            }
        }

        if let Some((_, rule)) = best_match {
            FileClassification::PortRequired {
                path: norm.to_string(),
                status: status.to_string(),
                subsystem_id: rule.id.clone(),
                rust_targets: rule.rust_targets.clone(),
                conformance_suites: rule.conformance_suites.clone(),
            }
        } else {
            FileClassification::Unmapped {
                path: norm.to_string(),
                status: status.to_string(),
            }
        }
    }

    pub fn build_report(
        &self,
        from_commit: &str,
        to_commit: &str,
        entries: Vec<ChangedFileEntry>,
    ) -> UpstreamDeltaReport {
        let total_changed_files = entries.len();
        let mut by_subsystem: BTreeMap<String, SubsystemDelta> = BTreeMap::new();
        let mut unmapped = Vec::new();
        let mut skipped_runtime = Vec::new();
        let mut ignored_noise = Vec::new();
        let mut port_required_count = 0usize;

        for entry in entries {
            match self.classify_path(&entry.path, &entry.status) {
                FileClassification::IgnoredNoise { path, .. } => ignored_noise.push(path),
                FileClassification::SkipRuntimeSpecific { path, .. } => skipped_runtime.push(path),
                FileClassification::Unmapped { .. } => unmapped.push(entry),
                FileClassification::PortRequired { subsystem_id, .. } => {
                    port_required_count += 1;
                    let rule = self.subsystems.iter().find(|r| r.id == subsystem_id);
                    let delta = by_subsystem.entry(subsystem_id.clone()).or_insert_with(|| {
                        SubsystemDelta {
                            subsystem_id,
                            description: rule.map(|r| r.description.clone()).unwrap_or_default(),
                            rust_targets: rule.map(|r| r.rust_targets.clone()).unwrap_or_default(),
                            conformance_suites: rule
                                .map(|r| r.conformance_suites.clone())
                                .unwrap_or_default(),
                            files: Vec::new(),
                        }
                    });
                    delta.files.push(entry);
                }
            }
        }

        UpstreamDeltaReport {
            from_commit: from_commit.to_string(),
            to_commit: to_commit.to_string(),
            total_changed_files,
            port_required_count,
            skipped_runtime_count: skipped_runtime.len(),
            ignored_noise_count: ignored_noise.len(),
            unmapped_count: unmapped.len(),
            by_subsystem,
            unmapped,
            skipped_runtime,
            ignored_noise,
        }
    }
}

impl UpstreamDeltaReport {
    pub fn render_markdown(&self, port_only: bool, subsystem_filter: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Upstream Delta (`{}..{}`)\n\n",
            &self.from_commit[..self.from_commit.len().min(12)],
            &self.to_commit[..self.to_commit.len().min(12)]
        ));
        out.push_str(&format!(
            "- Total changed files: {}\n- PORT_REQUIRED: {}\n- SKIP_RUNTIME_SPECIFIC: {}\n- IGNORED_NOISE: {}\n- UNMAPPED: {}\n\n",
            self.total_changed_files,
            self.port_required_count,
            self.skipped_runtime_count,
            self.ignored_noise_count,
            self.unmapped_count
        ));

        for (id, sub) in &self.by_subsystem {
            if let Some(flt) = subsystem_filter {
                if id != flt {
                    continue;
                }
            }
            out.push_str(&format!(
                "## Subsystem `{}` ({} files)\n",
                sub.subsystem_id,
                sub.files.len()
            ));
            if !sub.description.is_empty() {
                out.push_str(&format!("- Scope: {}\n", sub.description));
            }
            if !sub.rust_targets.is_empty() {
                out.push_str(&format!(
                    "- Rust Targets: {}\n",
                    sub.rust_targets.join(", ")
                ));
            }
            if !sub.conformance_suites.is_empty() {
                out.push_str(&format!(
                    "- Conformance: {}\n",
                    sub.conformance_suites.join(", ")
                ));
            }
            for f in &sub.files {
                out.push_str(&format!("  - [{}] {}\n", f.status, f.path));
            }
            out.push('\n');
        }

        if !port_only {
            if !self.unmapped.is_empty() {
                out.push_str("## Unmapped\n");
                for f in &self.unmapped {
                    out.push_str(&format!("  - [{}] {}\n", f.status, f.path));
                }
                out.push('\n');
            }
            if !self.skipped_runtime.is_empty() {
                out.push_str(&format!(
                    "## Skipped Runtime-Specific ({})\n",
                    self.skipped_runtime.len()
                ));
                for p in self.skipped_runtime.iter().take(40) {
                    out.push_str(&format!("  - {p}\n"));
                }
                out.push('\n');
            }
        }
        out
    }
}

/// Parse `git diff --name-status` lines (`M\tpath`, `A\tpath`, `D\tpath`, `R100\told\tnew`).
pub fn parse_git_name_status(raw: &str) -> Vec<ChangedFileEntry> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('\t').collect();
        match parts.as_slice() {
            [status, path] => out.push(ChangedFileEntry {
                status: (*status).to_string(),
                path: (*path).to_string(),
                old_path: None,
            }),
            [status, old_path, new_path] => out.push(ChangedFileEntry {
                status: (*status).to_string(),
                path: (*new_path).to_string(),
                old_path: Some((*old_path).to_string()),
            }),
            _ => {}
        }
    }
    out
}

pub fn glob_matches(pattern: &str, path: &str) -> bool {
    if let Some(dir_prefix) = pattern.strip_suffix("/**") {
        return path == dir_prefix || path.starts_with(&format!("{dir_prefix}/"));
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        return path.ends_with(&format!(".{ext}"));
    }
    if let Some(sub) = pattern.strip_prefix("**/") {
        if path == sub || path.ends_with(&format!("/{sub}")) {
            return true;
        }
        if let Some(dir_sub) = sub.strip_suffix("/**") {
            return path.starts_with(&format!("{dir_sub}/"))
                || path.contains(&format!("/{dir_sub}/"));
        }
        if let Some(ext) = sub.strip_prefix("*.") {
            return path.ends_with(&format!(".{ext}"));
        }
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut cursor = path;
        for (idx, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if idx == 0 {
                let Some(rest) = cursor.strip_prefix(part) else {
                    return false;
                };
                cursor = rest;
            } else if idx + 1 == parts.len() {
                return cursor.ends_with(part);
            } else {
                let Some(pos) = cursor.find(part) else {
                    return false;
                };
                cursor = &cursor[pos + part.len()..];
            }
        }
        return true;
    }
    path == pattern
}
