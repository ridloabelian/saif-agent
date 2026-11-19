use crate::fuzzy_match::fuzzy_find_and_replace;
use crate::patch_parser::{apply_v4a_operations, parse_v4a_patch};
use crate::registry::{ToolContext, ToolError};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn resolve_path(raw: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// Guard writes to protected agent instruction / secret files (`tools/file_tools_write_guards.py`).
fn check_protected_write(target: &Path, ctx: &ToolContext) -> Result<(), ToolError> {
    let home = &ctx.profile.home_dir;
    let protected = [
        home.join(".env"),
        home.join("config.yaml"),
        home.join("SOUL.md"),
    ];
    let norm_target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    for p in protected {
        let norm_p = p.canonicalize().unwrap_or(p);
        if norm_target == norm_p && ctx.is_delegated_child {
            return Err(ToolError::Execution(format!(
                "Write guard blocked delegated child modification of protected instruction/secret file: {}",
                target.display()
            )));
        }
    }
    Ok(())
}

pub fn handle_read_file(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'path'".to_string()))?;
    let path = resolve_path(path_str, &ctx.cwd);
    let content = fs::read_to_string(&path)
        .map_err(|e| ToolError::Execution(format!("cannot read '{}': {e}", path.display())))?;

    let lines: Vec<&str> = content.lines().collect();
    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v.saturating_sub(1) as usize)
        .unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(lines.len());

    let end = (offset + limit).min(lines.len());
    let slice = if offset < lines.len() {
        lines[offset..end]
            .iter()
            .enumerate()
            .map(|(i, ln)| format!("{:>4}| {ln}", offset + i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    Ok(json!({
        "success": true,
        "path": path.display().to_string(),
        "total_lines": lines.len(),
        "content": slice
    })
    .to_string())
}

pub fn handle_write_file(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'path'".to_string()))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'content'".to_string()))?;
    let path = resolve_path(path_str, &ctx.cwd);
    check_protected_write(&path, ctx)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ToolError::Execution(format!("mkdir failed: {e}")))?;
    }
    fs::write(&path, content).map_err(|e| ToolError::Execution(format!("write failed: {e}")))?;

    Ok(json!({
        "success": true,
        "path": path.display().to_string(),
        "bytes_written": content.len()
    })
    .to_string())
}

pub fn handle_patch(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("replace");
    if mode == "patch" || args.get("patch").and_then(|v| v.as_str()).is_some() {
        let patch_text = args
            .get("patch")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'patch'".to_string()))?;
        let ops = parse_v4a_patch(patch_text).map_err(ToolError::Execution)?;
        for op in &ops {
            check_protected_write(&resolve_path(&op.file_path, &ctx.cwd), ctx)?;
        }
        let applied = apply_v4a_operations(&ops, &ctx.cwd).map_err(ToolError::Execution)?;
        return Ok(json!({
            "success": true,
            "mode": "patch",
            "applied": applied
        })
        .to_string());
    }

    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'path'".to_string()))?;
    let old_string = args
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'old_string'".to_string()))?;
    let new_string = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'new_string'".to_string()))?;
    let replace_all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = resolve_path(path_str, &ctx.cwd);
    check_protected_write(&path, ctx)?;
    let content = fs::read_to_string(&path)
        .map_err(|e| ToolError::Execution(format!("cannot read '{}': {e}", path.display())))?;

    let res = fuzzy_find_and_replace(&content, old_string, new_string, replace_all);
    if let Some(err) = res.error {
        return Ok(json!({
            "success": false,
            "error": err
        })
        .to_string());
    }
    fs::write(&path, &res.content)
        .map_err(|e| ToolError::Execution(format!("cannot write '{}': {e}", path.display())))?;

    Ok(json!({
        "success": true,
        "path": path.display().to_string(),
        "matches": res.match_count,
        "strategy": res.strategy
    })
    .to_string())
}

pub fn handle_search_files(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'pattern'".to_string()))?;
    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("content");
    let root = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| resolve_path(s, &ctx.cwd))
        .unwrap_or_else(|| ctx.cwd.clone());
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;

    let re = Regex::new(pattern)
        .map_err(|e| ToolError::InvalidArgs(format!("invalid regex '{pattern}': {e}")))?;

    let mut matches = Vec::new();
    for entry in WalkBuilder::new(&root).hidden(false).build().flatten() {
        if matches.len() >= limit {
            break;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        if target == "files" {
            if re.is_match(&rel) {
                matches.push(json!({ "path": rel }));
            }
        } else if let Ok(text) = fs::read_to_string(path) {
            for (idx, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    matches.push(json!({
                        "path": rel,
                        "line": idx + 1,
                        "text": line.chars().take(240).collect::<String>()
                    }));
                    if matches.len() >= limit {
                        break;
                    }
                }
            }
        }
    }

    Ok(json!({
        "success": true,
        "target": target,
        "count": matches.len(),
        "matches": matches
    })
    .to_string())
}
