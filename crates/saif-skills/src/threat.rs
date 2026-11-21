use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

pub const MAX_SCAN_CHARS: usize = 65_536;

pub const INVISIBLE_CHARS: &[char] = &[
    '\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{2062}', '\u{2063}', '\u{2064}', '\u{feff}',
    '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}',
    '\u{2069}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreatScope {
    All,
    Context,
    Strict,
}

impl ThreatScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "context" => Some(Self::Context),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::All => 0,
            Self::Context => 1,
            Self::Strict => 2,
        }
    }
}

struct CompiledPattern {
    id: &'static str,
    min_scope: ThreatScope,
    regex: Regex,
}

fn compiled_patterns() -> &'static Vec<CompiledPattern> {
    static PATTERNS: OnceLock<Vec<CompiledPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let filler = r"(?:\w+\s+){0,8}";
        let secret_var = r"\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL)S?\b";
        let modify = r"(update|modify|edit|write|change|append|add\s+to)\s+[^\n]{0,2048}";

        let specs: Vec<(String, &'static str, ThreatScope)> = vec![
            (format!(r"ignore\s+{filler}(previous|all|above|prior)\s+{filler}instructions"), "prompt_injection", ThreatScope::All),
            (r"system\s+prompt\s+override".into(), "sys_prompt_override", ThreatScope::All),
            (format!(r"disregard\s+{filler}(your|all|any)\s+{filler}(instructions|rules|guidelines)"), "disregard_rules", ThreatScope::All),
            (format!(r"act\s+as\s+(if|though)\s+{filler}you\s+{filler}(have\s+no|don't\s+have)\s+{filler}(restrictions|limits|rules)"), "bypass_restrictions", ThreatScope::All),
            (r"<!--[^>]{0,512}(?:ignore|override|system|secret|hidden)[^>]{0,512}-->".into(), "html_comment_injection", ThreatScope::All),
            (r#"<\s*div\s+style\s*=\s*["'][^>]{0,2048}display\s*:\s*none"#.into(), "hidden_div", ThreatScope::All),
            (r"translate\s+[^\n]{0,512}\s+into\s+\w+(?:[\s-]+\w+){0,2}\s+and\s+(execute|run|eval)\b".into(), "translate_execute", ThreatScope::All),
            (format!(r"do\s+not\s+{filler}tell\s+{filler}the\s+user"), "deception_hide", ThreatScope::All),
            // Context scope
            (format!(r"you\s+are\s+{filler}now\s+(?:a|an|the)\s+"), "role_hijack", ThreatScope::Context),
            (format!(r"pretend\s+{filler}(you\s+are|to\s+be)\s+"), "role_pretend", ThreatScope::Context),
            (format!(r"output\s+{filler}(system|initial)\s+prompt"), "leak_system_prompt", ThreatScope::Context),
            (format!(r"(respond|answer|reply)\s+without\s+{filler}(restrictions|limitations|filters|safety)"), "remove_filters", ThreatScope::Context),
            (format!(r"you\s+have\s+been\s+{filler}(updated|upgraded|patched)\s+to"), "fake_update", ThreatScope::Context),
            (r"\bname\s+yourself\s+\w+".into(), "identity_override", ThreatScope::Context),
            (r"register\s+(as\s+)?a?\s*node".into(), "c2_node_registration", ThreatScope::Context),
            (r"(heartbeat|beacon|check[\s\-]?in)\s+(to|with)\s+".into(), "c2_heartbeat", ThreatScope::Context),
            (r"pull\s+(down\s+)?(?:new\s+)?task(?:ing|s)?\b".into(), "c2_task_pull", ThreatScope::Context),
            (r"connect\s+to\s+the\s+network\b".into(), "c2_network_connect", ThreatScope::Context),
            (r"you\s+must\s+(?:\w+\s+){0,3}(register|connect|report|beacon)\b".into(), "forced_action", ThreatScope::Context),
            (r"only\s+use\s+one[\s\-]?liners?\b".into(), "anti_forensic_oneliner", ThreatScope::Context),
            (format!(r"never\s+{filler}(?:create|write)\s+{filler}(?:script|file)\s+{filler}disk"), "anti_forensic_disk", ThreatScope::Context),
            (r"unset\s+\w*(?:CLAUDE|CODEX|HERMES|SAIF|AGENT|OPENAI|ANTHROPIC)\w*".into(), "env_var_unset_agent", ThreatScope::Context),
            (r"\b(?:cobalt\s*strike|sliver|havoc|mythic|metasploit|brainworm)\b".into(), "known_c2_framework", ThreatScope::Context),
            (r"\bc2\s+(?:server|channel|infrastructure|beacon)\b".into(), "c2_explicit", ThreatScope::Context),
            (r"\bcommand\s+and\s+control\b".into(), "c2_explicit_long", ThreatScope::Context),
            // Exfil (All / Strict)
            (format!(r"curl\s+[^\n]{{0,2048}}{secret_var}"), "exfil_curl", ThreatScope::All),
            (format!(r"wget\s+[^\n]{{0,2048}}{secret_var}"), "exfil_wget", ThreatScope::All),
            (r"cat\s+[^\n]{0,2048}(\.env|credentials|\.netrc|\.pgpass|\.npmrc|\.pypirc)".into(), "read_secrets", ThreatScope::All),
            (r"(send|post|upload|transmit)\s+[^\n]{0,2048}\s+(to|at)\s+https?://".into(), "send_to_url", ThreatScope::Strict),
            (format!(r"(include|output|print|share)\s+{filler}(conversation|chat\s+history|previous\s+messages|full\s+context|entire\s+context)"), "context_exfil", ThreatScope::Strict),
            // Persistence / backdoor (Strict)
            (r"authorized_keys".into(), "ssh_backdoor", ThreatScope::Strict),
            (r"(?:\b(?:echo|cat|cp|mv|dd|tee|install|printf|rsync|scp|ln|append|add|write|sed|chmod|chown|truncate|rm|touch|curl|wget|git)\b|\bopen\s*\(|>>?)[^\n]{0,512}(?:\$HOME/\.ssh|~/\.ssh)".into(), "ssh_access", ThreatScope::Strict),
            (r"\$HOME/\.(?:hermes|saif)/\.env|~/\.(?:hermes|saif)/\.env".into(), "hermes_env", ThreatScope::Strict),
            (format!(r"{modify}(?:AGENTS\.md|CLAUDE\.md|\.cursorrules|\.clinerules)"), "agent_config_mod", ThreatScope::Strict),
            (format!(r"{modify}\.(?:hermes|saif)/(config\.yaml|SOUL\.md)"), "hermes_config_mod", ThreatScope::Strict),
        ];

        specs
            .into_iter()
            .filter_map(|(pat, id, min_scope)| {
                RegexBuilder::new(&pat)
                    .case_insensitive(true)
                    .build()
                    .ok()
                    .map(|regex| CompiledPattern {
                        id,
                        min_scope,
                        regex,
                    })
            })
            .collect()
    })
}

/// Returns matched pattern IDs (`invisible_unicode_U+XXXX` + pattern ids) for `scope`.
pub fn scan_for_threats(content: &str, scope: ThreatScope) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let bounded: String = content.chars().take(MAX_SCAN_CHARS).collect();
    let mut findings = Vec::new();
    let mut seen_invisible = HashSet::new();
    for ch in bounded.chars() {
        if INVISIBLE_CHARS.contains(&ch) && seen_invisible.insert(ch) {
            findings.push(format!("invisible_unicode_U+{:04X}", ch as u32));
        }
    }

    let normalized: String = bounded.nfkc().collect();
    let max_rank = scope.rank();
    for pat in compiled_patterns() {
        if pat.min_scope.rank() <= max_rank && pat.regex.is_match(&normalized) {
            findings.push(pat.id.to_string());
        }
    }
    findings
}

/// Scan raw file bytes so invalid UTF-8 padding or embedded NUL bytes cannot hide threat payloads.
pub fn scan_bytes_for_threats(raw: &[u8], scope: ThreatScope) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    let slice = &raw[..raw.len().min(MAX_SCAN_CHARS * 4)];
    let mut findings = Vec::new();
    if slice.contains(&0u8) {
        findings.push("nul_byte".to_string());
    }
    if std::str::from_utf8(slice).is_err() {
        findings.push("invalid_utf8_bytes".to_string());
    }
    let text = String::from_utf8_lossy(slice);
    for finding in scan_for_threats(&text, scope) {
        if !findings.contains(&finding) {
            findings.push(finding);
        }
    }
    findings
}

pub fn first_threat_message(content: &str, scope: ThreatScope) -> Option<String> {
    let findings = scan_for_threats(content, scope);
    let pid = findings.first()?;
    if let Some(cp) = pid.strip_prefix("invisible_unicode_") {
        return Some(format!(
            "Blocked: content contains invisible unicode character {cp} (possible injection)."
        ));
    }
    Some(format!(
        "Blocked: content matches threat pattern '{pid}'. Content is injected into the system prompt and must not contain injection or exfiltration payloads."
    ))
}
