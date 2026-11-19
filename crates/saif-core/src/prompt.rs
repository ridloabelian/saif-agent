use saif_config::{ProfileScope, SaifConfig};
use saif_skills::{scan_for_threats, SkillStore, ThreatScope};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::warn;

/// Default identity kept neutral/identical to upstream `NousResearch/hermes-agent` (`agent/prompt_builder.py`).
pub const DEFAULT_AGENT_IDENTITY: &str = "You are Hermes Agent, built by Nous Research. Be direct: match the length of your reply to the weight of the ask — a one-line question gets a one-line answer, and finished work gets a short report of what changed, what's verified, and what's left, never a replay of the process. No filler (\"Great question,\" \"I'd be happy to\"), no restating the request back, no re-summarizing what you already said, no narrating tool calls the user can see. Plain claims over adjectives; when unsure, say so plainly. Agree because it's right, not because the user said it. Depth is earned — give it when the user asks for detail, teaches, or the stakes demand it, not by default.";

pub const MAX_SUBDIR_HINT_CHARS: usize = 32_000;

/// Matches `agent/prompt_builder.py::_scan_context_content`.
pub fn scan_context_content(content: &str, filename: &str, user_authored: bool) -> String {
    let cleaned = content.strip_prefix('\u{feff}').unwrap_or(content);
    let findings = scan_for_threats(cleaned, ThreatScope::Context);
    if findings.is_empty() {
        return cleaned.to_string();
    }
    let joined = findings.join(", ");
    if user_authored {
        warn!(
            "Context file {filename} matched injection pattern(s) {joined}; loaded anyway because it is user-authored"
        );
        return cleaned.to_string();
    }
    warn!("Context file {filename} blocked: {joined}");
    format!("[BLOCKED: {filename} contained potential prompt injection ({joined}). Content not loaded.]")
}

pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let resolved = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    for ancestor in resolved.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

async fn read_text_with_timeout(path: PathBuf, timeout: Duration) -> Option<String> {
    let handle = tokio::task::spawn_blocking(move || fs::read_to_string(path).ok());
    tokio::time::timeout(timeout, handle)
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten()
}

pub fn head_tail_truncate(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let half = max_chars / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(total - half).collect();
    format!(
        "{head}\n\n[... truncated {} chars ...]\n\n{tail}",
        total - max_chars
    )
}

pub struct PromptBuilder;

impl PromptBuilder {
    /// Assemble the byte-stable `system[0]` prompt once per conversation.
    pub async fn build_system_prompt(
        profile: &ProfileScope,
        cfg: &SaifConfig,
        cwd: &Path,
        extra_system: Option<&str>,
    ) -> String {
        let mut sections: Vec<String> = Vec::new();
        let timeout = Duration::from_secs_f64(cfg.agent.context_file_read_timeout.max(0.5));

        let soul_path = profile.home_dir.join("SOUL.md");
        if soul_path.is_file() {
            if let Some(raw) = read_text_with_timeout(soul_path, timeout).await {
                sections.push(scan_context_content(&raw, "SOUL.md", true));
            }
        } else {
            sections.push(DEFAULT_AGENT_IDENTITY.to_string());
        }

        if let Some(extra) = extra_system.filter(|s| !s.trim().is_empty()) {
            sections.push(extra.to_string());
        }

        if cfg.memory.enabled {
            let mem_path = profile.memories_dir().join("MEMORY.md");
            if let Some(raw) = read_text_with_timeout(mem_path, timeout).await {
                if !raw.trim().is_empty() {
                    sections.push(format!(
                        "## Persistent Memory\n{}",
                        head_tail_truncate(&raw, cfg.memory.max_chars)
                    ));
                }
            }
        }
        if cfg.memory.user_profile_enabled {
            let user_path = profile.memories_dir().join("USER.md");
            if let Some(raw) = read_text_with_timeout(user_path, timeout).await {
                if !raw.trim().is_empty() {
                    sections.push(format!(
                        "## User Profile\n{}",
                        head_tail_truncate(&raw, cfg.memory.max_chars)
                    ));
                }
            }
        }

        // Load project context files walking up from cwd bounded strictly to git root.
        let context_files =
            Self::discover_project_context_files(cwd, timeout, cfg.agent.context_file_max_chars)
                .await;
        if !context_files.is_empty() {
            sections.push(context_files);
        }

        // Skills index (<= 60 chars per entry)
        let skill_store = SkillStore::new(profile.skills_dir(), &cfg.skills.disabled);
        let skills = skill_store.list_skills();
        if !skills.is_empty() {
            let mut idx_lines = vec!["## Available Skills".to_string()];
            for s in skills {
                idx_lines.push(format!("- `{}`: {}", s.name, s.prompt_description));
            }
            sections.push(idx_lines.join("\n"));
        }

        sections.join("\n\n")
    }

    async fn discover_project_context_files(
        cwd: &Path,
        timeout: Duration,
        max_chars: usize,
    ) -> String {
        let stop_at = find_git_root(cwd);
        let resolved = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let dirs: Vec<PathBuf> = if let Some(root) = stop_at {
            let mut chain: Vec<PathBuf> = resolved
                .ancestors()
                .take_while(|p| *p != root)
                .map(Path::to_path_buf)
                .collect();
            chain.push(root);
            chain
        } else {
            vec![resolved]
        };

        let candidate_names = [
            ".saif.md",
            "SAIF.md",
            ".hermes.md",
            "HERMES.md",
            "AGENTS.md",
        ];
        let mut chunks = Vec::new();
        for dir in dirs.into_iter().rev() {
            for name in candidate_names {
                let p = dir.join(name);
                if p.is_file() {
                    if let Some(raw) = read_text_with_timeout(p, timeout).await {
                        let safe = scan_context_content(&raw, name, false);
                        chunks.push(format!(
                            "## Context ({name})\n{}",
                            head_tail_truncate(&safe, max_chars)
                        ));
                    }
                    break;
                }
            }
        }
        chunks.join("\n\n")
    }
}
