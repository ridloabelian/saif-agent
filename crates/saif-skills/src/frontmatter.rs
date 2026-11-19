use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const SKILL_PROMPT_DESC_LIMIT: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub platforms: Vec<String>,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub created_by: Option<String>,
}

fn end_fence_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\r?\n---\s*\r?\n").unwrap_or_else(|_| unreachable!()))
}

fn last_whitespace_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s\S*$").unwrap_or_else(|_| unreachable!()))
}

fn clip_trail_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[\s.,;:\-\u{2013}\u{2014}(/]+$").unwrap_or_else(|_| unreachable!())
    })
}

/// Matches Hermes `agent/skill_utils.py::parse_frontmatter`.
pub fn parse_frontmatter(raw: &str) -> (BTreeMap<String, Value>, String) {
    let content = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    if !content.starts_with("---") {
        return (BTreeMap::new(), content.to_string());
    }
    let after_open = &content[3..];
    let Some(m) = end_fence_re().find(after_open) else {
        return (BTreeMap::new(), content.to_string());
    };
    let yaml_content = &after_open[..m.start()];
    let body = &after_open[m.end()..];

    let mut out = BTreeMap::new();
    if let Ok(Value::Mapping(map)) = serde_yaml::from_str::<Value>(yaml_content) {
        for (k, v) in map {
            if let Some(ks) = k.as_str() {
                out.insert(ks.to_string(), v);
            }
        }
    } else {
        for line in yaml_content.trim().lines() {
            if let Some((k, v)) = line.split_once(':') {
                out.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
            }
        }
    }
    (out, body.to_string())
}

pub fn normalize_skill_description(frontmatter: &BTreeMap<String, Value>) -> String {
    let Some(raw) = frontmatter.get("description") else {
        return String::new();
    };
    let text = match raw {
        Value::String(s) => s.as_str(),
        Value::Null => return String::new(),
        other => {
            return serde_yaml::to_string(other)
                .unwrap_or_default()
                .trim()
                .to_string()
        }
    };
    text.trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string()
}

/// Matches Hermes `agent/skill_utils.py::clip_skill_description`.
pub fn clip_skill_description(desc: &str) -> String {
    let chars: Vec<char> = desc.chars().collect();
    if chars.len() <= SKILL_PROMPT_DESC_LIMIT {
        return desc.to_string();
    }
    let head: String = chars[..SKILL_PROMPT_DESC_LIMIT - 2].iter().collect();
    if let Some(boundary) = last_whitespace_re().find(&head) {
        if boundary.start() > 0 {
            let trimmed = clip_trail_re().replace(&head[..boundary.start()], "");
            return format!("{trimmed}...");
        }
    }
    let hard_head: String = chars[..SKILL_PROMPT_DESC_LIMIT - 3].iter().collect();
    format!("{hard_head}...")
}

pub fn extract_skill_description(frontmatter: &BTreeMap<String, Value>) -> String {
    clip_skill_description(&normalize_skill_description(frontmatter))
}

pub fn is_skill_description_truncated_for_prompt(frontmatter: &BTreeMap<String, Value>) -> bool {
    normalize_skill_description(frontmatter).chars().count() > SKILL_PROMPT_DESC_LIMIT
}

pub fn validate_skill_description(desc: &str) -> Result<(), String> {
    let trimmed = desc.trim();
    if trimmed.is_empty() {
        return Err("Skill description must not be empty.".to_string());
    }
    let char_len = trimmed.chars().count();
    if char_len > SKILL_PROMPT_DESC_LIMIT {
        return Err(format!(
            "Skill description exceeds {SKILL_PROMPT_DESC_LIMIT} chars ({char_len} chars)."
        ));
    }
    if !trimmed.ends_with('.') {
        return Err(
            "Skill description must be one concise sentence ending with a period.".to_string(),
        );
    }
    Ok(())
}

pub fn extract_metadata(frontmatter: &BTreeMap<String, Value>) -> SkillMetadata {
    let str_field = |k: &str| -> Option<String> {
        frontmatter
            .get(k)
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
    };
    let str_list = |v: &Value| -> Vec<String> {
        match v {
            Value::Sequence(seq) => seq
                .iter()
                .filter_map(|i| i.as_str().map(ToString::to_string))
                .collect(),
            Value::String(s) => vec![s.clone()],
            _ => Vec::new(),
        }
    };

    let hermes_meta = frontmatter.get("metadata").and_then(|m| m.get("hermes"));

    let tags = frontmatter
        .get("tags")
        .map(str_list)
        .or_else(|| hermes_meta.and_then(|h| h.get("tags")).map(str_list))
        .unwrap_or_default();

    let category = str_field("category").or_else(|| {
        hermes_meta
            .and_then(|h| h.get("category"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
    });

    let platforms = frontmatter
        .get("platforms")
        .map(str_list)
        .unwrap_or_default();

    SkillMetadata {
        name: str_field("name"),
        description: str_field("description"),
        version: str_field("version"),
        author: str_field("author"),
        license: str_field("license"),
        platforms,
        tags,
        category,
        created_by: str_field("created_by"),
    }
}
