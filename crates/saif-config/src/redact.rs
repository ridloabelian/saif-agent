use regex::Regex;
use std::sync::OnceLock;

fn bearer_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(authorization\s*:\s*bearer\s+|\bbearer\s+)([A-Za-z0-9._~+/=-]{12,})")
            .unwrap_or_else(|_| unreachable!())
    })
}

fn connstr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b((?:postgres|postgresql|mysql|redis|rediss|mongodb|mongodb\+srv|amqp|amqps)://[^/\s:@]*:)([^@\s/]+)(@)",
        )
        .unwrap_or_else(|_| unreachable!())
    })
}

fn api_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:sk-ant-[A-Za-z0-9_-]{10,}|sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{20,}|\b[MN][A-Za-z0-9_-]{23,}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27,}\b|\b\d{8,12}:AA[A-Za-z0-9_-]{30,}\b)",
        )
        .unwrap_or_else(|_| unreachable!())
    })
}

/// Structured secret redaction matching Hermes `agent/redact_*.py` boundaries:
/// masks Bearer headers, database connection string credentials, and API/bot token shapes.
pub fn redact_sensitive_text(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let step1 = bearer_re().replace_all(input, "${1}[REDACTED_SECRET]");
    let step2 = connstr_re().replace_all(&step1, "${1}[REDACTED_DB_SECRET]${3}");
    api_key_re()
        .replace_all(&step2, "[REDACTED_API_KEY]")
        .into_owned()
}
