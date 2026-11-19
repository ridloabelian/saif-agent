const PREVIEW_MAX_CHARS: usize = 60;
const SKILL_EXCERPT_JOINT: &str = "\n...\n";

/// Matches Hermes `hermes_state_common._shape_preview`.
pub fn shape_preview(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let single_line = trimmed.replace(['\n', '\r'], " ");
    let base = if let Some((head, _)) = single_line.split_once(SKILL_EXCERPT_JOINT) {
        head
    } else {
        single_line.as_str()
    };
    let char_count = base.chars().count();
    if char_count > PREVIEW_MAX_CHARS {
        let prefix: String = base.chars().take(PREVIEW_MAX_CHARS).collect();
        format!("{prefix}...")
    } else {
        base.to_string()
    }
}
