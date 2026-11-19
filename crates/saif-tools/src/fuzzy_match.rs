use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub type Span = (usize, usize);

pub const IDENTICAL_STRINGS_ERROR: &str =
    "No edit was applied because old_string and new_string are identical. Provide the existing text to replace in old_string and the changed replacement text in new_string.";

#[derive(Debug, Clone)]
pub struct FuzzyReplaceResult {
    pub content: String,
    pub match_count: usize,
    pub strategy: Option<String>,
    pub error: Option<String>,
}

const UNICODE_MAP: &[(char, &str)] = &[
    ('\u{201c}', "\""),
    ('\u{201d}', "\""),
    ('\u{2018}', "'"),
    ('\u{2019}', "'"),
    ('\u{2014}', "--"),
    ('\u{2013}', "-"),
    ('\u{2026}', "..."),
    ('\u{00a0}', " "),
    ('\u{2212}', "-"),
    ('\u{2000}', " "),
    ('\u{2001}', " "),
    ('\u{2002}', " "),
    ('\u{2003}', " "),
    ('\u{2004}', " "),
    ('\u{2005}', " "),
    ('\u{2006}', " "),
    ('\u{2007}', " "),
    ('\u{2008}', " "),
    ('\u{2009}', " "),
    ('\u{200a}', " "),
    ('\u{202f}', " "),
    ('\u{205f}', " "),
    ('\u{3000}', " "),
];

fn unicode_repl(ch: char) -> Option<&'static str> {
    UNICODE_MAP
        .iter()
        .find_map(|(k, v)| (*k == ch).then_some(*v))
}

fn unicode_normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if let Some(repl) = unicode_repl(ch) {
            out.push_str(repl);
        } else {
            out.push(ch);
        }
    }
    out
}

fn line_spans_for_window(
    content_lines: &[&str],
    start_line: usize,
    n: usize,
    total_len: usize,
) -> Span {
    let start_byte: usize = content_lines[..start_line]
        .iter()
        .map(|l| l.len() + 1)
        .sum();
    let end_raw: usize = content_lines[..start_line + n]
        .iter()
        .map(|l| l.len() + 1)
        .sum();
    let end_byte = end_raw.saturating_sub(1).min(total_len);
    (start_byte, end_byte)
}

fn match_transformed_lines(
    content: &str,
    pattern: &str,
    transform: fn(&[&str]) -> Vec<String>,
) -> Vec<Span> {
    let c_lines: Vec<&str> = content.split('\n').collect();
    let p_lines: Vec<&str> = pattern.split('\n').collect();
    let p_norm = transform(&p_lines);
    let n = p_norm.len();
    if n == 0 || n > c_lines.len() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for i in 0..=c_lines.len() - n {
        if transform(&c_lines[i..i + n]) == p_norm {
            spans.push(line_spans_for_window(&c_lines, i, n, content.len()));
        }
    }
    spans
}

fn strategy_exact(content: &str, pattern: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = 0;
    while let Some(pos) = content[start..].find(pattern) {
        let abs = start + pos;
        let end = abs + pattern.len();
        spans.push((abs, end));
        start = end;
    }
    spans
}

fn strategy_line_trimmed(content: &str, pattern: &str) -> Vec<Span> {
    match_transformed_lines(content, pattern, |ls| {
        ls.iter().map(|l| l.trim().to_string()).collect()
    })
}

fn ws_collapse_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[ \t]+").unwrap_or_else(|_| unreachable!()))
}

fn strategy_whitespace_normalized(content: &str, pattern: &str) -> Vec<Span> {
    let collapse_with_map = |text: &str| -> (String, Vec<usize>) {
        let mut norm = String::new();
        let mut norm_to_orig_char_start = Vec::new();
        let mut chars = text.char_indices().peekable();
        while let Some((idx, ch)) = chars.next() {
            if ch == ' ' || ch == '\t' {
                norm.push(' ');
                norm_to_orig_char_start.push(idx);
                while let Some(&(_, next_ch)) = chars.peek() {
                    if next_ch == ' ' || next_ch == '\t' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            } else {
                let mut buf = [0u8; 4];
                let enc = ch.encode_utf8(&mut buf);
                for _ in 0..enc.len() {
                    norm_to_orig_char_start.push(idx);
                }
                norm.push(ch);
            }
        }
        norm_to_orig_char_start.push(text.len());
        (norm, norm_to_orig_char_start)
    };

    let norm_pat = ws_collapse_re().replace_all(pattern, " ");
    let (norm_content, map) = collapse_with_map(content);
    let norm_matches = strategy_exact(&norm_content, &norm_pat);
    norm_matches
        .into_iter()
        .filter_map(|(ns, ne)| {
            let os = *map.get(ns)?;
            let oe = *map.get(ne)?;
            Some((os, oe))
        })
        .collect()
}

fn strategy_indentation_flexible(content: &str, pattern: &str) -> Vec<Span> {
    match_transformed_lines(content, pattern, |ls| {
        ls.iter().map(|l| l.trim_start().to_string()).collect()
    })
}

fn strategy_escape_normalized(content: &str, pattern: &str) -> Vec<Span> {
    let unescaped = pattern
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\r", "\r");
    if unescaped == pattern {
        return Vec::new();
    }
    strategy_exact(content, &unescaped)
}

fn strategy_trimmed_boundary(content: &str, pattern: &str) -> Vec<Span> {
    match_transformed_lines(content, pattern, |ls| {
        let mut out: Vec<String> = ls.iter().map(|l| (*l).to_string()).collect();
        if let Some(first) = out.first_mut() {
            *first = first.trim().to_string();
        }
        if out.len() > 1 {
            if let Some(last) = out.last_mut() {
                *last = last.trim().to_string();
            }
        }
        out
    })
}

fn strategy_unicode_normalized(content: &str, pattern: &str) -> Vec<Span> {
    let norm_p = unicode_normalize(pattern);
    let norm_c = unicode_normalize(content);
    if norm_c == content && norm_p == pattern {
        return Vec::new();
    }
    let mut norm_matches = strategy_exact(&norm_c, &norm_p);
    if norm_matches.is_empty() {
        norm_matches = strategy_line_trimmed(&norm_c, &norm_p);
    }
    if norm_matches.is_empty() {
        return Vec::new();
    }

    // Build byte mapping orig_byte -> norm_byte
    let mut orig_bytes = Vec::new();
    let mut norm_to_orig: HashMap<usize, usize> = HashMap::new();
    let mut norm_byte = 0usize;
    for (orig_byte, ch) in content.char_indices() {
        norm_to_orig.entry(norm_byte).or_insert(orig_byte);
        orig_bytes.push((orig_byte, norm_byte));
        if let Some(repl) = unicode_repl(ch) {
            norm_byte += repl.len();
        } else {
            norm_byte += ch.len_utf8();
        }
    }
    norm_to_orig.entry(norm_byte).or_insert(content.len());
    orig_bytes.push((content.len(), norm_byte));

    norm_matches
        .into_iter()
        .filter_map(|(ns, ne)| {
            let os = *norm_to_orig.get(&ns)?;
            let oe = orig_bytes
                .iter()
                .find(|(_, nb)| *nb >= ne)
                .map(|(ob, _)| *ob)
                .unwrap_or(content.len());
            Some((os, oe))
        })
        .collect()
}

fn line_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let lcs = lcs_len(&ac, &bc);
    (2.0 * lcs as f64) / ((ac.len() + bc.len()) as f64)
}

fn lcs_len(a: &[char], b: &[char]) -> usize {
    if a.len() > 2048 || b.len() > 2048 {
        let set_b: HashSet<char> = b.iter().copied().collect();
        return a.iter().filter(|c| set_b.contains(c)).count();
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut curr = vec![0usize; b.len() + 1];
    for &ca in a {
        for (j, &cb) in b.iter().enumerate() {
            curr[j + 1] = if ca == cb {
                prev[j] + 1
            } else {
                prev[j + 1].max(curr[j])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    prev[b.len()]
}

fn strategy_block_anchor(content: &str, pattern: &str) -> Vec<Span> {
    let norm_pat = unicode_normalize(pattern);
    let p_lines: Vec<&str> = norm_pat.split('\n').collect();
    if p_lines.len() < 2 {
        return Vec::new();
    }
    let first_p = p_lines[0].trim();
    let last_p = p_lines[p_lines.len() - 1].trim();
    let n = p_lines.len();
    let norm_content = unicode_normalize(content);
    let nc_lines: Vec<&str> = norm_content.split('\n').collect();
    if n > nc_lines.len() {
        return Vec::new();
    }
    let candidates: Vec<usize> = (0..=nc_lines.len() - n)
        .filter(|&i| nc_lines[i].trim() == first_p && nc_lines[i + n - 1].trim() == last_p)
        .collect();
    let threshold = if candidates.len() == 1 { 0.50 } else { 0.70 };
    let p_mid = p_lines[1..n - 1].join("\n");
    let orig_lines: Vec<&str> = content.split('\n').collect();
    let mut spans = Vec::new();
    for i in candidates {
        if n <= 2 || line_similarity(&nc_lines[i + 1..i + n - 1].join("\n"), &p_mid) >= threshold {
            spans.push(line_spans_for_window(&orig_lines, i, n, content.len()));
        }
    }
    spans
}

fn strategy_context_aware(content: &str, pattern: &str) -> Vec<Span> {
    let p_lines: Vec<&str> = pattern.split('\n').collect();
    let c_lines: Vec<&str> = content.split('\n').collect();
    let n = p_lines.len();
    if n == 0 || n > c_lines.len() {
        return Vec::new();
    }
    let first_p = p_lines[0].trim();
    let last_p = p_lines[n - 1].trim();
    let mut spans = Vec::new();
    for i in 0..=c_lines.len() - n {
        let block = &c_lines[i..i + n];
        if line_similarity(first_p, block[0].trim()) < 0.80
            || line_similarity(last_p, block[n - 1].trim()) < 0.80
        {
            continue;
        }
        if p_lines
            .iter()
            .zip(block.iter())
            .all(|(p, c)| p.trim().is_empty() || line_similarity(p.trim(), c.trim()) >= 0.80)
        {
            spans.push(line_spans_for_window(&c_lines, i, n, content.len()));
        }
    }
    spans
}

fn leading_ws(line: &str) -> &str {
    let trimmed = line.trim_start_matches([' ', '\t']);
    &line[..line.len() - trimmed.len()]
}

fn reindent_replacement(file_region: &str, old_string: &str, new_string: &str) -> String {
    let Some(old_first) = old_string.split('\n').find(|l| !l.trim().is_empty()) else {
        return new_string.to_string();
    };
    let Some(file_first) = file_region.split('\n').find(|l| !l.trim().is_empty()) else {
        return new_string.to_string();
    };
    let old_indent = leading_ws(old_first);
    let file_indent = leading_ws(file_first);
    if old_indent == file_indent {
        return new_string.to_string();
    }
    new_string
        .split('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else if leading_ws(line).starts_with(old_indent) {
                format!("{file_indent}{}", &line[old_indent.len()..])
            } else {
                format!("{file_indent}{}", line.trim_start_matches([' ', '\t']))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn longest_common_block(
    a: &[char],
    a_lo: usize,
    a_hi: usize,
    b: &[char],
    b_lo: usize,
    b_hi: usize,
) -> Option<(usize, usize, usize)> {
    let a_len = a_hi.saturating_sub(a_lo);
    let b_len = b_hi.saturating_sub(b_lo);
    if a_len == 0 || b_len == 0 {
        return None;
    }
    let mut prev = vec![0usize; b_len + 1];
    let mut curr = vec![0usize; b_len + 1];
    let mut best_len = 0usize;
    let mut best_a = a_lo;
    let mut best_b = b_lo;
    for i in 0..a_len {
        for j in 0..b_len {
            if a[a_lo + i] == b[b_lo + j] {
                let len = prev[j] + 1;
                curr[j + 1] = len;
                if len > best_len {
                    best_len = len;
                    best_a = a_lo + i + 1 - len;
                    best_b = b_lo + j + 1 - len;
                }
            } else {
                curr[j + 1] = 0;
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    if best_len > 0 {
        Some((best_a, best_b, best_len))
    } else {
        None
    }
}

fn collect_matching_blocks(
    a: &[char],
    a_lo: usize,
    a_hi: usize,
    b: &[char],
    b_lo: usize,
    b_hi: usize,
    out: &mut Vec<(usize, usize, usize)>,
) {
    if let Some((ai, bi, size)) = longest_common_block(a, a_lo, a_hi, b, b_lo, b_hi) {
        collect_matching_blocks(a, a_lo, ai, b, b_lo, bi, out);
        out.push((ai, bi, size));
        collect_matching_blocks(a, ai + size, a_hi, b, bi + size, b_hi, out);
    }
}

/// Preserves file typography (smart quotes, em-dashes) outside changed regions
/// during `unicode_normalized` replacements (`_preserve_unicode_in_replacement`).
fn preserve_unicode_in_replacement(
    file_region: &str,
    old_string: &str,
    new_string: &str,
) -> String {
    let norm_old = unicode_normalize(old_string);
    if norm_old != unicode_normalize(file_region) {
        return new_string.to_string();
    }

    let mut orig_spans_per_norm_char: Vec<(usize, usize)> = Vec::new();
    for (orig_byte, ch) in file_region.char_indices() {
        let orig_end = orig_byte + ch.len_utf8();
        if let Some(repl) = unicode_repl(ch) {
            for _ in repl.chars() {
                orig_spans_per_norm_char.push((orig_byte, orig_end));
            }
        } else {
            orig_spans_per_norm_char.push((orig_byte, orig_end));
        }
    }

    let norm_chars: Vec<char> = norm_old.chars().collect();
    let new_chars: Vec<char> = new_string.chars().collect();
    if norm_chars.len() != orig_spans_per_norm_char.len() {
        return new_string.to_string();
    }

    let min_len = norm_chars.len().min(new_chars.len());
    let mut prefix = 0usize;
    while prefix < min_len && norm_chars[prefix] == new_chars[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < min_len - prefix
        && norm_chars[norm_chars.len() - 1 - suffix] == new_chars[new_chars.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mid_a_hi = norm_chars.len() - suffix;
    let mid_b_hi = new_chars.len() - suffix;
    if (mid_a_hi - prefix).saturating_mul(mid_b_hi - prefix) > 4_000_000 {
        return new_string.to_string();
    }

    let mut blocks: Vec<(usize, usize, usize)> = Vec::new();
    if prefix > 0 {
        blocks.push((0, 0, prefix));
    }
    collect_matching_blocks(
        &norm_chars,
        prefix,
        mid_a_hi,
        &new_chars,
        prefix,
        mid_b_hi,
        &mut blocks,
    );
    if suffix > 0 {
        blocks.push((mid_a_hi, mid_b_hi, suffix));
    }
    blocks.push((norm_chars.len(), new_chars.len(), 0));

    let mut result = String::with_capacity(new_string.len().max(file_region.len()));
    let mut prev_b = 0usize;
    for (ai, bi, size) in blocks {
        if bi > prev_b {
            result.extend(&new_chars[prev_b..bi]);
        }
        if size > 0 {
            let orig_s = orig_spans_per_norm_char[ai].0;
            let orig_e = orig_spans_per_norm_char[ai + size - 1].1;
            if orig_s <= orig_e && orig_e <= file_region.len() {
                result.push_str(&file_region[orig_s..orig_e]);
            }
        }
        prev_b = bi + size;
    }
    result
}

pub fn is_already_applied(content: &str, old_string: &str, new_string: &str) -> bool {
    if new_string.trim().len() < 8 || !content.contains(new_string) {
        return false;
    }
    old_string == new_string || !content.contains(old_string)
}

/// 9-strategy progressive fuzzy replacement (`tools/fuzzy_match.py` parity).
pub fn fuzzy_find_and_replace(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> FuzzyReplaceResult {
    if old_string.is_empty() {
        return FuzzyReplaceResult {
            content: content.to_string(),
            match_count: 0,
            strategy: None,
            error: Some("old_string is empty — nothing to match.".to_string()),
        };
    }
    if old_string.trim().is_empty() {
        return FuzzyReplaceResult {
            content: content.to_string(),
            match_count: 0,
            strategy: None,
            error: Some(
                "old_string is only whitespace — provide non-blank text to match.".to_string(),
            ),
        };
    }
    if old_string == new_string {
        return FuzzyReplaceResult {
            content: content.to_string(),
            match_count: 0,
            strategy: None,
            error: Some(IDENTICAL_STRINGS_ERROR.to_string()),
        };
    }

    type StrategyFn = fn(&str, &str) -> Vec<Span>;
    let strategies: &[(&str, StrategyFn, bool)] = &[
        ("exact", strategy_exact, false),
        ("line_trimmed", strategy_line_trimmed, false),
        (
            "whitespace_normalized",
            strategy_whitespace_normalized,
            false,
        ),
        ("indentation_flexible", strategy_indentation_flexible, false),
        ("escape_normalized", strategy_escape_normalized, false),
        ("trimmed_boundary", strategy_trimmed_boundary, false),
        ("unicode_normalized", strategy_unicode_normalized, false),
        ("block_anchor", strategy_block_anchor, true),
        ("context_aware", strategy_context_aware, true),
    ];

    for &(name, func, is_similarity) in strategies {
        let matches = func(content, old_string);
        if matches.is_empty() {
            continue;
        }
        if matches.len() > 1 && !replace_all {
            return FuzzyReplaceResult {
                content: content.to_string(),
                match_count: 0,
                strategy: None,
                error: Some(format!(
                    "Found {} matches for old_string. Provide more context to make it unique, or use replace_all=true.",
                    matches.len()
                )),
            };
        }
        if replace_all && matches.len() > 1 && is_similarity {
            return FuzzyReplaceResult {
                content: content.to_string(),
                match_count: 0,
                strategy: None,
                error: Some(format!(
                    "Found {} approximate matches via '{name}'; replace_all requires exact/normalizing match.",
                    matches.len()
                )),
            };
        }

        let mut sorted = matches;
        sorted.sort_by_key(|a| std::cmp::Reverse(a.0));
        let count = sorted.len();
        let mut updated = content.to_string();
        for (start, end) in sorted {
            let adjusted = if name == "exact" {
                new_string.to_string()
            } else {
                let region = &content[start..end];
                let candidate = if name == "unicode_normalized" {
                    preserve_unicode_in_replacement(region, old_string, new_string)
                } else {
                    new_string.to_string()
                };
                reindent_replacement(region, old_string, &candidate)
            };
            updated.replace_range(start..end, &adjusted);
        }

        return FuzzyReplaceResult {
            content: updated,
            match_count: count,
            strategy: Some(name.to_string()),
            error: None,
        };
    }

    FuzzyReplaceResult {
        content: content.to_string(),
        match_count: 0,
        strategy: None,
        error: Some("Could not find a match for old_string in the file".to_string()),
    }
}
