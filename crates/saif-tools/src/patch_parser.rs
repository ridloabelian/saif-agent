use crate::fuzzy_match::{fuzzy_find_and_replace, is_already_applied};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationType {
    Add,
    Update,
    Delete,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HunkLine {
    pub prefix: char,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Hunk {
    pub context_hint: Option<String>,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchOperation {
    pub operation: OperationType,
    pub file_path: String,
    pub new_path: Option<String>,
    pub hunks: Vec<Hunk>,
}

fn begin_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\*\*\*\s*Begin\s+Patch\s*$").unwrap_or_else(|_| unreachable!()))
}

fn end_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\*\*\*\s*End\s+Patch\s*$").unwrap_or_else(|_| unreachable!()))
}

fn update_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\*\*\*\s*Update\s+File:\s*(.+)").unwrap_or_else(|_| unreachable!())
    })
}

fn add_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*\*\*\s*Add\s+File:\s*(.+)").unwrap_or_else(|_| unreachable!()))
}

fn delete_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\*\*\*\s*Delete\s+File:\s*(.+)").unwrap_or_else(|_| unreachable!())
    })
}

fn move_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\*\*\*\s*Move\s+File:\s*(.+?)\s*->\s*(.+)").unwrap_or_else(|_| unreachable!())
    })
}

fn hint_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@@\s*(.+?)\s*@@").unwrap_or_else(|_| unreachable!()))
}

/// Parses a V4A patch (`*** Begin Patch` .. `*** End Patch`), matching `tools/patch_parser.py`.
pub fn parse_v4a_patch(patch_content: &str) -> Result<Vec<PatchOperation>, String> {
    let lines: Vec<&str> = patch_content
        .split('\n')
        .map(|ln| ln.strip_suffix('\r').unwrap_or(ln))
        .collect();

    let mut start_idx: isize = -1;
    let mut end_idx = lines.len();
    for (i, line) in lines.iter().enumerate() {
        if begin_re().is_match(line) {
            start_idx = i as isize;
        } else if end_re().is_match(line) {
            end_idx = i;
            break;
        }
    }

    let mut operations: Vec<PatchOperation> = Vec::new();
    let mut current_op: Option<PatchOperation> = None;
    let mut current_hunk: Option<Hunk> = None;

    let flush_hunk = |op: &mut Option<PatchOperation>, hunk: &mut Option<Hunk>| {
        if let (Some(o), Some(h)) = (op.as_mut(), hunk.take()) {
            if !h.lines.is_empty() {
                o.hunks.push(h);
            }
        }
    };

    let flush_op = |ops: &mut Vec<PatchOperation>,
                    op: &mut Option<PatchOperation>,
                    hunk: &mut Option<Hunk>| {
        flush_hunk(op, hunk);
        if let Some(o) = op.take() {
            ops.push(o);
        }
    };

    let slice_start = (start_idx + 1).max(0) as usize;
    for &line in &lines[slice_start..end_idx] {
        let op_match = if let Some(m) = update_re().captures(line) {
            Some((OperationType::Update, m[1].trim().to_string(), None))
        } else if let Some(m) = add_re().captures(line) {
            Some((OperationType::Add, m[1].trim().to_string(), None))
        } else if let Some(m) = delete_re().captures(line) {
            Some((OperationType::Delete, m[1].trim().to_string(), None))
        } else {
            move_re().captures(line).map(|m| {
                (
                    OperationType::Move,
                    m[1].trim().to_string(),
                    Some(m[2].trim().to_string()),
                )
            })
        };

        if let Some((kind, file_path, new_path)) = op_match {
            flush_op(&mut operations, &mut current_op, &mut current_hunk);
            let op = PatchOperation {
                operation: kind,
                file_path,
                new_path,
                hunks: Vec::new(),
            };
            if matches!(kind, OperationType::Delete | OperationType::Move) {
                operations.push(op);
                current_op = None;
                current_hunk = None;
            } else {
                current_hunk = if kind == OperationType::Add {
                    Some(Hunk::default())
                } else {
                    None
                };
                current_op = Some(op);
            }
        } else if line.starts_with("@@") {
            if current_op.is_some() {
                flush_hunk(&mut current_op, &mut current_hunk);
                let hint = hint_re().captures(line).map(|c| c[1].to_string());
                current_hunk = Some(Hunk {
                    context_hint: hint,
                    lines: Vec::new(),
                });
            }
        } else if current_op.is_some() && !line.is_empty() {
            let h = current_hunk.get_or_insert_with(Hunk::default);
            let first = line.chars().next().unwrap_or(' ');
            if matches!(first, '+' | '-' | ' ') {
                h.lines.push(HunkLine {
                    prefix: first,
                    content: line[1..].to_string(),
                });
            } else if first != '\\' {
                h.lines.push(HunkLine {
                    prefix: ' ',
                    content: line.to_string(),
                });
            }
        }
    }
    flush_op(&mut operations, &mut current_op, &mut current_hunk);

    let mut errors = Vec::new();
    for op in &operations {
        if op.file_path.is_empty() {
            errors.push("Operation with empty file path".to_string());
        }
        if op.operation == OperationType::Update && op.hunks.is_empty() {
            errors.push(format!("UPDATE '{}': no hunks found", op.file_path));
        }
        if op.operation == OperationType::Move && op.new_path.as_deref().unwrap_or("").is_empty() {
            errors.push(format!("MOVE '{}': missing destination path", op.file_path));
        }
    }

    if errors.is_empty() {
        Ok(operations)
    } else {
        Err(format!("Parse error: {}", errors.join("; ")))
    }
}

/// Apply parsed V4A operations atomically (validating all hunks in an overlay before disk writes).
pub fn apply_v4a_operations(
    ops: &[PatchOperation],
    base_dir: &Path,
) -> Result<Vec<String>, String> {
    let mut overlay: HashMap<String, String> = HashMap::new();
    let mut deleted: HashSet<String> = HashSet::new();
    let mut summary = Vec::new();

    for op in ops {
        let rel = &op.file_path;
        let full = base_dir.join(rel);
        match op.operation {
            OperationType::Update => {
                let mut content = if let Some(cached) = overlay.get(rel) {
                    cached.clone()
                } else {
                    fs::read_to_string(&full)
                        .map_err(|e| format!("{rel}: failed reading file ({e})"))?
                };
                for (idx, hunk) in op.hunks.iter().enumerate() {
                    let search: Vec<&str> = hunk
                        .lines
                        .iter()
                        .filter(|l| l.prefix != '+')
                        .map(|l| l.content.as_str())
                        .collect();
                    let replace: Vec<&str> = hunk
                        .lines
                        .iter()
                        .filter(|l| l.prefix != '-')
                        .map(|l| l.content.as_str())
                        .collect();
                    if search == replace {
                        continue;
                    }
                    let old_s = search.join("\n");
                    let new_s = replace.join("\n");
                    let res = fuzzy_find_and_replace(&content, &old_s, &new_s, false);
                    if res.match_count > 0 {
                        content = res.content;
                    } else if !is_already_applied(&content, &old_s, &new_s) {
                        return Err(format!(
                            "{rel}: hunk {} failed — {}",
                            idx + 1,
                            res.error.unwrap_or_else(|| "not found".to_string())
                        ));
                    }
                }
                overlay.insert(rel.clone(), content);
                summary.push(format!("Updated {rel}"));
            }
            OperationType::Add => {
                if full.exists() || overlay.contains_key(rel) {
                    return Err(format!("{rel}: Add target already exists"));
                }
                let body: Vec<&str> = op
                    .hunks
                    .iter()
                    .flat_map(|h| h.lines.iter())
                    .filter(|l| l.prefix == '+')
                    .map(|l| l.content.as_str())
                    .collect();
                overlay.insert(rel.clone(), body.join("\n"));
                deleted.remove(rel);
                summary.push(format!("Added {rel}"));
            }
            OperationType::Delete => {
                if !full.exists() && !overlay.contains_key(rel) {
                    return Err(format!("{rel}: file not found for deletion"));
                }
                overlay.remove(rel);
                deleted.insert(rel.clone());
                summary.push(format!("Deleted {rel}"));
            }
            OperationType::Move => {
                let dst = op
                    .new_path
                    .as_ref()
                    .ok_or_else(|| format!("{rel}: missing destination path"))?;
                let dst_path = base_dir.join(dst);
                if dst_path.exists() || overlay.contains_key(dst) {
                    return Err(format!("{dst}: move destination already exists"));
                }
                let content = if let Some(cached) = overlay.get(rel) {
                    cached.clone()
                } else {
                    fs::read_to_string(&full)
                        .map_err(|e| format!("{rel}: source not found ({e})"))?
                };
                overlay.insert(dst.clone(), content);
                overlay.remove(rel);
                deleted.insert(rel.clone());
                summary.push(format!("Moved {rel} -> {dst}"));
            }
        }
    }

    for del in &deleted {
        let p = base_dir.join(del);
        if p.exists() {
            fs::remove_file(&p).map_err(|e| format!("{del}: delete failed ({e})"))?;
        }
    }
    for (rel, body) in &overlay {
        let p = base_dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{rel}: mkdir failed ({e})"))?;
        }
        fs::write(&p, body).map_err(|e| format!("{rel}: write failed ({e})"))?;
    }
    Ok(summary)
}
