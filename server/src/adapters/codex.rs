//! `adapters::codex` — Codex Provider adapter.
//!
//! Story 1.2 ships the **discovery slice** of the Codex adapter: it resolves
//! the local Codex Agent Memory root and reports whether that root appears to
//! exist. It does NOT read memory content (NFR-5), does NOT enumerate files
//! (Story 1.5), and does NOT canonicalize the root or allocate `source_id`
//! (Story 1.3). The Codex data boundary is enforced by the Supported Artifact
//! Matrix (AD-11): transcripts, session JSONL and human-authored rule files
//! are out of scope here — at discovery time we only check the *memories
//! directory* exists, and even then we do not look at what is inside.
//!
//! ## `CODEX_HOME` priority
//!
//! Per spec I/O matrix:
//! - If `CODEX_HOME` is set AND non-empty (after trimming whitespace), probe
//!   `$CODEX_HOME/memories` only. Do NOT fall back to `~/.codex/memories` — an
//!   explicit `CODEX_HOME` means the user has relocated their Codex home and
//!   silently double-reporting the default location would violate capability
//!   honesty (AD-3) and produce a misleading candidate list. An explicit but
//!   invalid `CODEX_HOME` (relative, or pointing at a missing/non-dir path)
//!   yields no candidate rather than a silent fallback.
//! - Otherwise probe `$HOME/.codex/memories`.
//! - Empty / whitespace-only values are treated as "unset" (Design Notes).
//!
//! ## Root validity
//!
//! A candidate root must be an **absolute**, **existing directory** with a
//! **UTF-8** path:
//! - Absolute: a relative `CODEX_HOME`/`HOME` would resolve against the
//!   process CWD and produce a candidate whose root drifts between launches.
//!   `ponytail:` roots are absolute by nature; reject relative rather than
//!   emit CWD-dependent candidates.
//! - Directory (`is_dir`, not `exists`): a regular file named `memories` is
//!   not a usable root, and a broken symlink correctly reports false. A
//!   symlink-to-dir is followed and accepted.
//! - UTF-8: a non-UTF-8 root would stringify to `U+FFFD` replacement chars —
//!   a display path that does not exist on disk and cannot be confirmed in
//!   1.3. Drop rather than emit garbage.
//!
//! ## Testability
//!
//! Path resolution is factored into [`resolve_codex_memories_root`] as a pure
//! function of `(codex_home, home)` — no env reads. Discovery is factored into
//! [`CodexAdapter::discover_with_env`], which takes the same injected values
//! and runs the full path (resolver → directory check → candidate
//! construction). Both are deliberate: under `cargo test`'s parallel executor
//! `std::env::set_var` races other tests and is `unsafe` on edition 2024, so
//! tests inject tempdir paths directly and exercise the adapter's own code
//! (not a test-local mirror of it). `discover()` is three steps of glue: read
//! env → `discover_with_env`.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::domain::ports::provider_adapter::{
    ArtifactDiagnostic, ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis,
    EnumerateError, FileUnit, ProviderAdapter, ProviderMemoryType, SupportedArtifact,
};

/// Codex Provider adapter (Story 1.2 discovery slice; Story 1.4 adds the
/// enumeration slice).
///
/// Unit struct — both slices are stateless. The adapter reads provider files
/// but never writes them (NFR-1 zero-write); enumeration reads metadata only,
/// never body content (NFR-5).
#[derive(Debug, Clone, Copy, Default)]
pub struct CodexAdapter;

const PROVIDER_ID: &str = "codex";

/// The three known first-level filenames in the Supported Artifact Matrix
/// (AD-11). Only these exact names at the root's first level are indexed;
/// everything else at the first level is skipped.
const KNOWN_ROOT_FILES: &[&str] = &["MEMORY.md", "memory_summary.md", "raw_memories.md"];

/// The one directory whose direct `*.md` children are indexed (AD-11). Only
/// direct children (one level, not recursive) are considered.
const ROLLOUT_SUMMARIES_DIR: &str = "rollout_summaries";

impl ProviderAdapter for CodexAdapter {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn coverage_level(&self) -> CoverageLevel {
        // Codex's memories root is a local directory tree that the adapter
        // will be able to enumerate in full once `enumerate` lands in Story
        // 1.5. Declaring `Full` here is honest capability disclosure (AD-3 /
        // AD-18): it describes the *provider surface*, not the slice that
        // ships in this Story. The actual enumeration implementation belongs
        // to 1.5; declaring Full now does not bypass that — the trait only
        // grows the discovery slice in 1.2.
        CoverageLevel::Full
    }

    fn discover(&self) -> Vec<CandidateSource> {
        // Three-step glue (Design Notes): read env → injected-env discover.
        // All input validation (trim, absolute) lives in the resolver; all FS
        // checks + candidate building live in `candidate_if_existing_dir`.
        // `discover_with_env` is the testable seam tests drive directly.
        let codex_home = env::var("CODEX_HOME").ok();
        let home = env::var("HOME").ok();
        self.discover_with_env(codex_home.as_deref(), home.as_deref())
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        Ok(self
            .enumerate_artifacts(root)?
            .supported
            .into_iter()
            .map(|artifact| artifact.file)
            .collect())
    }

    fn enumerate_artifacts(&self, root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
        enumerate_codex_artifacts(root)
    }
}

impl CodexAdapter {
    /// Discovery driven by injected env values rather than `std::env`, so the
    /// full discover path (resolver → directory check → candidate construction)
    /// is exercisable under `cargo test` parallelism without env mutation.
    /// [`ProviderAdapter::discover`] reads `CODEX_HOME`/`HOME` and delegates
    /// here.
    pub fn discover_with_env(
        &self,
        codex_home: Option<&str>,
        home: Option<&str>,
    ) -> Vec<CandidateSource> {
        match resolve_codex_memories_root(codex_home, home) {
            Some((basis, root)) => self.candidate_if_existing_dir(basis, &root),
            None => Vec::new(),
        }
    }

    /// Build the single candidate for a resolved root iff the root is an
    /// existing UTF-8 directory. A regular file, a missing path, a broken
    /// symlink, or a non-UTF-8 root yields no candidate. NFR-5: this is an
    /// existence/type check only — directory contents are never read.
    fn candidate_if_existing_dir(
        &self,
        basis: DiscoveryBasis,
        root: &Path,
    ) -> Vec<CandidateSource> {
        if !root.is_dir() {
            return Vec::new();
        }
        // Non-UTF-8 root: `to_string_lossy` would emit U+FFFD, producing a
        // display path that does not exist on disk and cannot be confirmed
        // (1.3). Drop instead of emitting garbage.
        let Some(path_str) = root.to_str() else {
            return Vec::new();
        };
        vec![CandidateSource {
            provider: PROVIDER_ID.to_string(),
            root_path: path_str.to_string(),
            basis,
            coverage_level: self.coverage_level(),
            // Codex memories are a global store with no discoverable
            // per-project split (Design Notes). Story 1.5 may revisit if the
            // parsed content lets us infer a project boundary; until then,
            // honestly report "unknown".
            native_project: None,
        }]
    }
}

/// Pure path resolver for the Codex memories root.
///
/// Extracted from [`CodexAdapter::discover`] so tests can exercise the
/// `CODEX_HOME`-priority rule without touching the process environment
/// (env-mutation races under `cargo test` parallelism; edition 2024 marks
/// `set_var` unsafe). The function does no FS I/O — `discover_with_env` ->
/// `candidate_if_existing_dir` applies the directory check to the returned
/// path.
///
/// Validation (applied here, the single source of truth):
/// - Empty / whitespace-only values are treated as unset.
/// - Values must be absolute; a relative root would resolve against the
///   process CWD and drift between launches, so it is rejected (returns
///   `None`, never a fallback for an explicit-but-invalid `CODEX_HOME`).
///
/// Returns `None` when neither a usable `codex_home` nor a usable `home` is
/// supplied, or when a supplied value fails validation.
pub fn resolve_codex_memories_root(
    codex_home: Option<&str>,
    home: Option<&str>,
) -> Option<(DiscoveryBasis, PathBuf)> {
    // Priority 1: explicit CODEX_HOME (non-empty after trim, absolute). No
    // fallback to ~/.codex — an explicit override is final, even when invalid.
    if let Some(ch) = codex_home.filter(|s| !s.trim().is_empty()) {
        let p = Path::new(ch);
        if p.is_absolute() {
            return Some((DiscoveryBasis::CodexHomeEnv, p.join("memories")));
        }
        return None;
    }
    // Priority 2: default home (non-empty after trim, absolute).
    let home = home.filter(|s| !s.trim().is_empty())?;
    let p = Path::new(home);
    if !p.is_absolute() {
        return None;
    }
    Some((
        DiscoveryBasis::DefaultHome,
        p.join(".codex").join("memories"),
    ))
}

/// Parser-version contract. Output changes require a deliberate version
/// decision rather than silently changing record identities or bodies.
pub const CODEX_MARKDOWN_PARSER_VERSION: &str = "codex-markdown/v1";

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

/// Enumerate allowlisted Codex artifacts and lexical unknowns. A known
/// artifact that cannot be resolved or inspected is terminal; only a proven
/// root escape or resolved-role mismatch is silently excluded.
fn enumerate_codex_artifacts(root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
    let canonical_root =
        std::fs::canonicalize(root).map_err(|_| EnumerateError::RootUnresolvable)?;
    let entries = std::fs::read_dir(&canonical_root).map_err(|_| EnumerateError::Unreadable)?;
    let mut supported = Vec::new();
    let mut diagnostics = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|_| EnumerateError::Unreadable)?;
        let lexical_path = entry.path();
        let name = entry.file_name();
        let name_utf8 = name.to_str();
        match name_utf8 {
            Some(name) if KNOWN_ROOT_FILES.contains(&name) => {
                let expected = root_memory_type(name).expect("known root artifact");
                if let Some(artifact) =
                    resolve_supported_artifact(&canonical_root, &lexical_path, expected, false)?
                {
                    supported.push(artifact);
                }
            }
            Some(ROLLOUT_SUMMARIES_DIR) => {
                let real_dir = match std::fs::canonicalize(&lexical_path) {
                    Ok(path) => path,
                    Err(_) => return Err(EnumerateError::AllowlistedArtifactUnresolvable),
                };
                if !real_dir.starts_with(&canonical_root)
                    || real_dir.strip_prefix(&canonical_root).ok()
                        != Some(Path::new(ROLLOUT_SUMMARIES_DIR))
                    || !std::fs::metadata(&real_dir)
                        .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
                        .is_dir()
                {
                    continue;
                }
                let rollout_entries = std::fs::read_dir(&real_dir)
                    .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
                for rollout_entry in rollout_entries {
                    let rollout_entry = rollout_entry
                        .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
                    let rollout_path = rollout_entry.path();
                    let observed = safe_relative_path(&canonical_root, &rollout_path);
                    let is_markdown = rollout_entry
                        .file_name()
                        .to_str()
                        .is_some_and(|entry_name| entry_name.ends_with(".md"));
                    if !is_markdown {
                        diagnostics.push(unsupported_diagnostic(observed));
                        continue;
                    }
                    if let Some(artifact) = resolve_supported_artifact(
                        &canonical_root,
                        &rollout_path,
                        ProviderMemoryType::RolloutSummary,
                        true,
                    )? {
                        supported.push(artifact);
                    }
                }
            }
            _ => diagnostics.push(unsupported_diagnostic(safe_relative_path(
                &canonical_root,
                &lexical_path,
            ))),
        }
    }

    supported.sort_by(|a, b| a.file.relative_path.cmp(&b.file.relative_path));
    supported.dedup_by(|a, b| a.file.relative_path == b.file.relative_path);
    diagnostics.sort();
    diagnostics.dedup();
    Ok(ArtifactEnumeration {
        supported,
        diagnostics,
    })
}

// Retained as a narrow Story 1.4 compatibility seam for the adapter's
// existing unit tests. Production scanning uses `enumerate_codex_artifacts`
// so it can validate diagnostics and exact artifact roles as one boundary.
#[cfg(test)]
fn enumerate_codex_file_units(root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
    Ok(enumerate_codex_artifacts(root)?
        .supported
        .into_iter()
        .map(|artifact| artifact.file)
        .collect())
}

fn root_memory_type(name: &str) -> Option<ProviderMemoryType> {
    match name {
        "MEMORY.md" => Some(ProviderMemoryType::Memory),
        "memory_summary.md" => Some(ProviderMemoryType::MemorySummary),
        "raw_memories.md" => Some(ProviderMemoryType::RawMemories),
        _ => None,
    }
}

fn resolve_supported_artifact(
    canonical_root: &Path,
    lexical_path: &Path,
    expected_type: ProviderMemoryType,
    is_rollout_child: bool,
) -> Result<Option<SupportedArtifact>, EnumerateError> {
    let real = match std::fs::canonicalize(lexical_path) {
        Ok(path) => path,
        Err(_) => return Err(EnumerateError::AllowlistedArtifactUnresolvable),
    };
    let Ok(relative) = real.strip_prefix(canonical_root) else {
        return Ok(None);
    };
    let role_matches = if is_rollout_child {
        relative.parent() == Some(Path::new(ROLLOUT_SUMMARIES_DIR))
            && relative.extension().and_then(|value| value.to_str()) == Some("md")
    } else {
        relative
            .to_str()
            .and_then(root_memory_type)
            .is_some_and(|actual| actual == expected_type)
    };
    if !role_matches {
        return Ok(None);
    }
    let metadata =
        std::fs::metadata(&real).map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
    if !metadata.is_file() {
        return Err(EnumerateError::AllowlistedArtifactUnresolvable);
    }
    let relative_path = relative
        .to_str()
        .ok_or(EnumerateError::AllowlistedArtifactUnresolvable)?
        .to_string();
    let mtime = metadata
        .modified()
        .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
        .as_nanos() as i64;
    Ok(Some(SupportedArtifact {
        file: FileUnit {
            relative_path,
            absolute_path: real,
            size: metadata.len(),
            mtime,
        },
        memory_type: expected_type,
    }))
}

fn unsupported_diagnostic(observed_path: String) -> ArtifactDiagnostic {
    ArtifactDiagnostic {
        kind: "unsupported_artifact",
        observed_path,
    }
}

/// Percent-encode an observed lexical path without attempting to turn it into
/// UTF-8. This representation is reversible and safe for SQLite diagnostics.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure resolver: CODEX_HOME non-empty wins, returns memories under it.
    /// No fallback to ~/.codex (AD-3 honesty — explicit override is final).
    #[test]
    fn resolver_prefers_codex_home_when_set() {
        let (basis, root) = resolve_codex_memories_root(Some("/x"), Some("/home/u"))
            .expect("codex_home set → Some");
        assert_eq!(basis, DiscoveryBasis::CodexHomeEnv);
        assert_eq!(root, PathBuf::from("/x/memories"));
    }

    /// Empty CODEX_HOME is treated as unset → falls back to default home.
    #[test]
    fn resolver_treats_empty_codex_home_as_unset() {
        let (basis, root) =
            resolve_codex_memories_root(Some(""), Some("/home/u")).expect("home set → Some");
        assert_eq!(basis, DiscoveryBasis::DefaultHome);
        assert_eq!(root, PathBuf::from("/home/u/.codex/memories"));
    }

    /// Whitespace-only CODEX_HOME is treated as unset (same as empty).
    #[test]
    fn resolver_treats_whitespace_codex_home_as_unset() {
        let (basis, _) = resolve_codex_memories_root(Some("   "), Some("/home/u"))
            .expect("whitespace codex_home → fallback to home");
        assert_eq!(basis, DiscoveryBasis::DefaultHome);
    }

    /// Relative CODEX_HOME is rejected (would resolve against CWD); it does
    /// NOT fall back to ~/.codex — an explicit override is final.
    #[test]
    fn resolver_rejects_relative_codex_home_without_fallback() {
        // Even when a valid default home is supplied, an explicit (relative)
        // CODEX_HOME yields None rather than silently using the default.
        assert!(resolve_codex_memories_root(Some("relative/path"), Some("/home/u")).is_none());
    }

    /// Relative HOME is rejected (would resolve against CWD).
    #[test]
    fn resolver_rejects_relative_home() {
        assert!(resolve_codex_memories_root(None, Some("./home")).is_none());
    }

    /// No CODEX_HOME but HOME set → default home basis.
    #[test]
    fn resolver_falls_back_to_default_home() {
        let (basis, root) =
            resolve_codex_memories_root(None, Some("/home/u")).expect("home set → Some");
        assert_eq!(basis, DiscoveryBasis::DefaultHome);
        assert_eq!(root, PathBuf::from("/home/u/.codex/memories"));
    }

    /// Neither CODEX_HOME nor HOME → None. No candidate, no error.
    #[test]
    fn resolver_returns_none_when_neither_env_set() {
        assert!(resolve_codex_memories_root(None, None).is_none());
        // Empty/whitespace strings count as unset too.
        assert!(resolve_codex_memories_root(Some(""), Some("")).is_none());
        assert!(resolve_codex_memories_root(Some("  "), None).is_none());
        // CODEX_HOME set but HOME empty/whitespace: CODEX_HOME path is used.
        let (basis, root) = resolve_codex_memories_root(Some("/x"), Some(""))
            .expect("codex_home set with empty home still resolves");
        assert_eq!(basis, DiscoveryBasis::CodexHomeEnv);
        assert_eq!(root, PathBuf::from("/x/memories"));
    }

    /// Adapter declares Codex capability honestly (AD-3).
    #[test]
    fn adapter_declares_codex_id_and_full_coverage() {
        let adapter = CodexAdapter;
        assert_eq!(adapter.provider_id(), "codex");
        assert_eq!(adapter.coverage_level(), CoverageLevel::Full);
    }

    // --- Story 1.4: enumerate_file_units (AD-11 boundary) ------------------

    /// Enumeration indexes only in-matrix files: the three known root files +
    /// `rollout_summaries/*.md` direct children. Excluded files (sessions,
    /// JSONL, CLAUDE.md, unknown names) are skipped (AD-11).
    #[test]
    fn enumerate_indexes_only_supported_artifact_matrix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(root.join("MEMORY.md"), "mem").expect("w");
        std::fs::write(root.join("memory_summary.md"), "sum").expect("w");
        std::fs::write(root.join("raw_memories.md"), "raw").expect("w");
        // Excluded: human instruction file, unknown name, sessions dir + JSONL.
        std::fs::write(root.join("CLAUDE.md"), "rules").expect("w");
        std::fs::write(root.join("unknown.md"), "unknown").expect("w");
        std::fs::create_dir_all(root.join("sessions")).expect("mkdir sessions");
        std::fs::write(root.join("sessions").join("foo.jsonl"), "{}").expect("w");
        // rollout_summaries with an .md and a non-.md child.
        std::fs::create_dir_all(root.join("rollout_summaries")).expect("mkdir rollout");
        std::fs::write(root.join("rollout_summaries").join("2026-07-01.md"), "r").expect("w");
        std::fs::write(root.join("rollout_summaries").join("notes.txt"), "r").expect("w");

        let units = enumerate_codex_file_units(root).expect("enumerate ok");
        let mut rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        rels.sort_unstable();
        assert_eq!(
            rels,
            vec![
                "MEMORY.md",
                "memory_summary.md",
                "raw_memories.md",
                "rollout_summaries/2026-07-01.md",
            ],
            "only in-matrix files are enumerated"
        );
    }

    /// `rollout_summaries/` is NOT recursive: a nested subdirectory's `.md` is
    /// skipped (one-level rule, spec Design Notes "枚举边界").
    #[test]
    fn enumerate_rollout_summaries_is_not_recursive() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let nested = root.join("rollout_summaries").join("nested");
        std::fs::create_dir_all(&nested).expect("mkdir nested");
        std::fs::write(root.join("rollout_summaries").join("top.md"), "r").expect("w");
        std::fs::write(nested.join("deep.md"), "r").expect("w");

        let units = enumerate_codex_file_units(root).expect("enumerate ok");
        let rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        assert_eq!(rels, vec!["rollout_summaries/top.md"], "nested .md skipped");
    }

    /// A symlinked file whose realpath escapes the canonical root is skipped
    /// (AD-4 / spec Block If — symlink escape). The in-root file is kept.
    #[cfg(unix)]
    #[test]
    fn enumerate_skips_symlink_escape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        // Real in-root file.
        std::fs::write(root.join("MEMORY.md"), "mem").expect("w");
        // A file outside the root, symlinked in as memory_summary.md.
        std::fs::write(outside.join("secret.md"), "secret").expect("w");
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("memory_summary.md"))
            .expect("symlink");

        let units = enumerate_codex_file_units(&root).expect("enumerate ok");
        let rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        assert_eq!(rels, vec!["MEMORY.md"], "escaping symlink skipped");
    }

    /// A symlinked `rollout_summaries` directory that resolves outside the
    /// confirmed root is skipped before it is opened; only in-root supported
    /// artifacts remain enumerable.
    #[cfg(unix)]
    #[test]
    fn enumerate_skips_rollout_directory_symlink_escape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        std::fs::write(root.join("MEMORY.md"), "mem").expect("w root");
        std::fs::write(outside.join("secret.md"), "secret").expect("w outside");
        std::os::unix::fs::symlink(&outside, root.join(ROLLOUT_SUMMARIES_DIR))
            .expect("symlink rollout dir");

        let units = enumerate_codex_file_units(&root).expect("enumerate ok");
        let rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        assert_eq!(
            rels,
            vec!["MEMORY.md"],
            "external rollout directory skipped"
        );
    }

    /// Enumeration of an empty directory succeeds with zero units (spec I/O
    /// matrix — empty directory scan is a complete success).
    #[test]
    fn enumerate_empty_directory_succeeds_with_zero_units() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let units = enumerate_codex_file_units(tmp.path()).expect("enumerate ok");
        assert!(units.is_empty());
    }

    /// Enumeration of a non-existent root returns Err (root unresolvable).
    #[test]
    fn enumerate_fails_for_missing_root() {
        let bogus = Path::new("/this/does/not/exist/tessera-1-4-enum");
        assert!(enumerate_codex_file_units(bogus).is_err());
    }
}
