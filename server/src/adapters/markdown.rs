//! `adapters::markdown` — the shared, provider-agnostic Markdown canonicalizer.
//!
//! Extracted from `adapters::codex` in Story 2.2 so the Claude Code adapter
//! can reuse *exactly the same* heading/section parser Codex uses, with only
//! the persisted `parser_version` tag differing. `canonicalize_markdown` has
//! no Codex semantics: it handles fences, ATX/setext headings, preamble, and
//! nested-section ids over any CommonMark-ish Markdown. One parser, two (or
//! more) version tags — a separate tag per provider lets a future
//! provider-specific grammar bump trigger a reparse without touching any other
//! provider's record identity.
//!
//! Re-exported by `adapters::codex` so existing call sites
//! (`crate::adapters::codex::canonicalize_markdown`, etc.) keep working
//! unchanged. New providers should import from this module directly.
//!
//! ## Behavior contract (unchanged by the extraction)
//!
//! The parser is intentionally narrow, deterministic, and dependency-free;
//! unsupported/malformed UTF-8 is a typed parse failure. Body content is
//! never rendered or logged — the function returns structured units the scan
//! pipeline persists.

use std::collections::HashMap;
use std::path::Path;

/// Canonical, source-relative Markdown unit. Locators are built by the scan
/// service because only it owns Source identity and persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalMarkdownUnit {
    pub unit_kind: String,
    pub native_unit_id: String,
    pub title: String,
    pub body: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkdownParseError;

/// Percent-encode an observed lexical path without attempting to turn it into
/// utf-8. This representation is reversible and safe for SQLite diagnostics.
pub fn safe_relative_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        percent_encode(relative.as_os_str().as_bytes(), true)
    }
    #[cfg(not(unix))]
    {
        percent_encode(relative.to_string_lossy().as_bytes(), true)
    }
}

/// Build a canonical, percent-encoded file URI. Paths and fragments are
/// encoded independently; a line display range never participates in record
/// identity.
pub fn file_uri(path: &Path) -> Result<String, MarkdownParseError> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let bytes = path.to_str().ok_or(MarkdownParseError)?.as_bytes();
    if !path.is_absolute() {
        return Err(MarkdownParseError);
    }
    Ok(format!("file://{}", percent_encode(bytes, true)))
}

pub fn percent_encode_fragment(value: &str) -> String {
    percent_encode(value.as_bytes(), false)
}

fn percent_encode(bytes: &[u8], preserve_slash: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len());
    for &byte in bytes {
        let safe = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if safe || (preserve_slash && byte == b'/') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    output
}

/// Canonicalize one allowlisted Markdown file without rendering or logging
/// body content. The grammar is intentionally narrow, deterministic, and
/// dependency-free; unsupported/malformed UTF-8 is a typed parse failure.
pub fn canonicalize_markdown(
    bytes: &[u8],
) -> Result<Vec<CanonicalMarkdownUnit>, MarkdownParseError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MarkdownParseError)?;
    let normalized = normalize_line_endings(text);
    let terminal_newline = normalized.ends_with('\n');
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    if normalized.ends_with('\n') {
        lines.pop();
    }
    if lines.is_empty() && !normalized.is_empty() {
        lines.push("");
    }
    let in_fence = fence_lines(&lines);
    let headings = parse_headings(&lines, &in_fence);
    if headings.is_empty() {
        return Ok(vec![CanonicalMarkdownUnit {
            unit_kind: "file".to_string(),
            native_unit_id: "file".to_string(),
            title: "File".to_string(),
            body: normalized.clone(),
            start_line: 1,
            end_line: lines.len().max(1),
        }]);
    }

    let mut records = Vec::new();
    let first = headings[0].start;
    if first > 0 {
        records.push(CanonicalMarkdownUnit {
            unit_kind: "preamble".to_string(),
            native_unit_id: "preamble".to_string(),
            title: "Preamble".to_string(),
            body: join_range(&lines, 0, first, terminal_newline),
            start_line: 1,
            end_line: first,
        });
    }

    let mut sibling_counts: HashMap<String, usize> = HashMap::new();
    let mut ancestors: Vec<HeadingFrame> = Vec::new();
    for (index, heading) in headings.iter().enumerate() {
        while ancestors
            .last()
            .is_some_and(|frame| frame.level >= heading.level)
        {
            ancestors.pop();
        }
        let parent_key = ancestors
            .iter()
            .map(|frame| frame.segment.as_str())
            .collect::<Vec<_>>()
            .join("/");
        let duplicate_key = format!(
            "{}|{}|{}:{}",
            parent_key,
            heading.level,
            heading.title.len(),
            heading.title
        );
        let ordinal = sibling_counts.entry(duplicate_key).or_insert(0);
        *ordinal += 1;
        let segment = format!(
            "h{}:{}:{}:{}",
            heading.level,
            heading.title.len(),
            heading.title,
            ordinal
        );
        let mut unit_id = String::from("section");
        for frame in &ancestors {
            unit_id.push('/');
            unit_id.push_str(&frame.segment);
        }
        unit_id.push('/');
        unit_id.push_str(&segment);
        let end = headings
            .get(index + 1)
            .map_or(lines.len(), |next| next.start);
        records.push(CanonicalMarkdownUnit {
            unit_kind: "section".to_string(),
            native_unit_id: unit_id,
            title: heading.title.clone(),
            body: join_range(&lines, heading.content_start, end, terminal_newline),
            start_line: heading.start + 1,
            end_line: end.max(heading.start + 1),
        });
        ancestors.push(HeadingFrame {
            level: heading.level,
            segment,
        });
    }
    Ok(records)
}

fn normalize_line_endings(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            normalized.push('\n');
        } else {
            normalized.push(ch);
        }
    }
    normalized
}

#[derive(Debug, Clone)]
struct ParsedHeading {
    start: usize,
    content_start: usize,
    level: usize,
    title: String,
}

#[derive(Debug, Clone)]
struct HeadingFrame {
    level: usize,
    segment: String,
}

fn parse_headings(lines: &[&str], in_fence: &[bool]) -> Vec<ParsedHeading> {
    let mut headings = Vec::new();
    let mut consumed = vec![false; lines.len()];
    let mut index = 0;
    while index < lines.len() {
        if in_fence[index] || consumed[index] {
            index += 1;
            continue;
        }
        if let Some((level, title)) = parse_atx(lines[index]) {
            headings.push(ParsedHeading {
                start: index,
                content_start: index + 1,
                level,
                title,
            });
            index += 1;
            continue;
        }
        let setext_level = if index + 1 < lines.len()
            && !in_fence[index + 1]
            && !consumed[index + 1]
            && valid_setext_title(lines[index])
        {
            parse_setext_underline(lines[index + 1])
        } else {
            None
        };
        if let Some(level) = setext_level {
            headings.push(ParsedHeading {
                start: index,
                content_start: index + 2,
                level,
                title: trim_ascii(lines[index]).to_string(),
            });
            consumed[index + 1] = true;
            index += 2;
            continue;
        }
        index += 1;
    }
    headings
}

fn fence_lines(lines: &[&str]) -> Vec<bool> {
    let mut result = vec![false; lines.len()];
    let mut open: Option<(u8, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some((marker, width)) = open {
            result[index] = true;
            if is_fence_closer(line, marker, width) {
                open = None;
            }
            continue;
        }
        if let Some((marker, width)) = parse_fence_opener(line) {
            result[index] = true;
            open = Some((marker, width));
        }
    }
    result
}

fn parse_fence_opener(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let offset = ascii_indent(bytes)?;
    let marker = *bytes.get(offset)?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let width = bytes[offset..]
        .iter()
        .take_while(|&&value| value == marker)
        .count();
    if width < 3 {
        return None;
    }
    let info = &line[offset + width..];
    if marker == b'`' && info.contains('`') {
        return None;
    }
    Some((marker, width))
}

fn is_fence_closer(line: &str, marker: u8, width: usize) -> bool {
    let bytes = line.as_bytes();
    let Some(offset) = ascii_indent(bytes) else {
        return false;
    };
    if bytes.get(offset) != Some(&marker) {
        return false;
    }
    let actual = bytes[offset..]
        .iter()
        .take_while(|&&value| value == marker)
        .count();
    actual >= width
        && bytes[offset + actual..]
            .iter()
            .all(|value| matches!(value, b' ' | b'\t'))
}

fn parse_atx(line: &str) -> Option<(usize, String)> {
    let bytes = line.as_bytes();
    let offset = ascii_indent(bytes)?;
    let level = bytes[offset..]
        .iter()
        .take_while(|&&value| value == b'#')
        .count();
    if !(1..=6).contains(&level) || !matches!(bytes.get(offset + level), None | Some(b' ' | b'\t'))
    {
        return None;
    }
    let mut title = trim_ascii(&line[offset + level..]);
    let without_spaces = trim_ascii_end(title);
    let hashes = without_spaces
        .as_bytes()
        .iter()
        .rev()
        .take_while(|&&value| value == b'#')
        .count();
    if hashes > 0 {
        let before = &without_spaces[..without_spaces.len() - hashes];
        if before
            .as_bytes()
            .last()
            .is_some_and(|value| matches!(value, b' ' | b'\t'))
        {
            title = trim_ascii_end(before);
        }
    }
    Some((level, title.to_string()))
}

fn parse_setext_underline(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let offset = ascii_indent(bytes)?;
    let marker = *bytes.get(offset)?;
    if marker != b'=' && marker != b'-' {
        return None;
    }
    let width = bytes[offset..]
        .iter()
        .take_while(|&&value| value == marker)
        .count();
    if width == 0
        || !bytes[offset + width..]
            .iter()
            .all(|value| matches!(value, b' ' | b'\t'))
    {
        return None;
    }
    Some(if marker == b'=' { 1 } else { 2 })
}

fn valid_setext_title(line: &str) -> bool {
    ascii_indent(line.as_bytes()).is_some()
        && !trim_ascii(line).is_empty()
        && parse_setext_underline(line).is_none()
        && parse_atx(line).is_none()
}

fn ascii_indent(bytes: &[u8]) -> Option<usize> {
    let spaces = bytes.iter().take_while(|&&byte| byte == b' ').count();
    if spaces <= 3 && bytes.get(spaces) != Some(&b'\t') {
        Some(spaces)
    } else {
        None
    }
}

fn trim_ascii(value: &str) -> &str {
    value.trim_matches(|ch| matches!(ch, ' ' | '\t'))
}

fn trim_ascii_end(value: &str) -> &str {
    value.trim_end_matches([' ', '\t'])
}

fn join_range(lines: &[&str], start: usize, end: usize, terminal_newline: bool) -> String {
    let start = start.min(lines.len());
    let end = end.min(lines.len());
    if start >= end {
        return String::new();
    }
    let mut body = lines[start..end].join("\n");
    if end < lines.len() || terminal_newline {
        body.push('\n');
    }
    body
}
