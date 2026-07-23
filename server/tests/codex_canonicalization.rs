//! Story 1.5 contract tests for the dependency-free Codex Markdown parser and
//! artifact boundary. These tests intentionally inspect parsed data directly;
//! scan-pipeline persistence is covered separately by `scan_pipeline`.

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use tessera_lib::adapters::codex::{
    canonicalize_markdown, file_uri, safe_relative_path, CodexAdapter,
    CODEX_MARKDOWN_PARSER_VERSION,
};
use tessera_lib::domain::{EnumerateError, ProviderAdapter, ProviderMemoryType};

#[test]
fn parses_preamble_nested_and_repeated_headings_without_overlap() {
    let units = canonicalize_markdown(
        b"leading prose\r\n \r\n# Alpha\r\nfirst\r\n\r\n## Child\r\nchild\r\n# Alpha\r\nlast\r",
    )
    .expect("valid markdown");

    assert_eq!(units.len(), 4);
    assert_eq!(units[0].unit_kind, "preamble");
    assert_eq!(units[0].body, "leading prose\n \n");
    assert_eq!((units[0].start_line, units[0].end_line), (1, 2));

    assert_eq!(units[1].title, "Alpha");
    assert_eq!(units[1].native_unit_id, "section/h1:5:Alpha:1");
    assert_eq!(units[1].body, "first\n\n");
    assert_eq!((units[1].start_line, units[1].end_line), (3, 5));

    assert_eq!(units[2].native_unit_id, "section/h1:5:Alpha:1/h2:5:Child:1");
    assert_eq!(units[2].body, "child\n");
    assert_eq!(units[3].native_unit_id, "section/h1:5:Alpha:2");
    assert_eq!(units[3].body, "last\n");
}

#[test]
fn repository_fixture_preserves_canonical_heading_boundaries() {
    let units = canonicalize_markdown(include_bytes!(
        "fixtures/providers/codex/canonical-boundaries.md"
    ))
    .expect("fixture parses");

    assert_eq!(units.len(), 3);
    assert_eq!(units[0].title, "Preamble");
    assert_eq!(units[1].native_unit_id, "section/h1:7:Fixture:1");
    assert_eq!(units[1].body, "body\n\n");
    assert_eq!(
        units[2].native_unit_id,
        "section/h1:7:Fixture:1/h2:5:Child:1"
    );
    assert_eq!(units[2].body, "child body\n");
}

#[test]
fn whitespace_only_preamble_and_unicode_atx_variants_are_preserved() {
    let units = canonicalize_markdown(" \t\n# 你好\nbody\n# 好###\nlast\n".as_bytes())
        .expect("Unicode headings must parse without byte-boundary panics");

    assert_eq!(units.len(), 3);
    assert_eq!(units[0].unit_kind, "preamble");
    assert_eq!(units[0].body, " \t\n");
    assert_eq!(units[1].title, "你好");
    assert_eq!(units[2].title, "好###");
}

#[test]
fn setext_fence_and_unicode_grammar_is_bounded_and_char_safe() {
    let units = canonicalize_markdown(
        "Title\n---\n---\n```bad`\n# Visible\n~~~\n# Hidden\n~~~~\n  ### café ###\nbody\n"
            .as_bytes(),
    )
    .expect("unicode title must not panic");

    // The second delimiter is not a title because a delimiter-only line is
    // never a Setext title. The invalid backtick opener is plain text, so
    // `Visible` remains a real heading. The valid tilde fence hides `Hidden`.
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].title, "Title");
    assert_eq!(units[1].title, "Visible");
    assert_eq!(units[2].title, "café");
    assert!(units[0].body.contains("---"));
    assert!(units[1].body.contains("# Hidden"));
}

#[test]
fn indented_tabbed_and_mixed_setext_lookalikes_stay_plain_body() {
    let units =
        canonicalize_markdown(b"    code block\n---\n\ttabbed\n===\nmixed\n-=-\n# Actual\nbody\n")
            .expect("valid markdown");

    assert_eq!(units.len(), 2);
    assert_eq!(units[0].unit_kind, "preamble");
    assert!(units[0].body.contains("    code block\n---"));
    assert!(units[0].body.contains("\ttabbed\n==="));
    assert!(units[0].body.contains("mixed\n-=-"));
    assert_eq!(units[1].title, "Actual");
}

#[test]
fn fallback_line_endings_and_locators_are_deterministic() {
    let units =
        canonicalize_markdown(b"no heading\rwith bare return\r\n").expect("fallback parses");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].unit_kind, "file");
    assert_eq!(units[0].body, "no heading\nwith bare return\n");
    assert_eq!(CODEX_MARKDOWN_PARSER_VERSION, "codex-markdown/v1");

    let uri = file_uri(Path::new("/tmp/a space/#.md")).expect("absolute URI");
    assert_eq!(uri, "file:///tmp/a%20space/%23.md");
}

#[test]
fn enumeration_classifies_all_supported_types_and_persists_safe_unknowns() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("MEMORY.md"), "memory\n").expect("memory");
    fs::write(root.join("memory_summary.md"), "summary\n").expect("summary");
    fs::write(root.join("raw_memories.md"), "raw\n").expect("raw");
    fs::create_dir(root.join("rollout_summaries")).expect("rollout dir");
    fs::write(root.join("rollout_summaries").join("one.md"), "rollout\n").expect("rollout");
    fs::write(root.join("AGENTS.md"), "rules\n").expect("unknown");

    let observation = CodexAdapter
        .enumerate_artifacts(root)
        .expect("enumerate artifacts");
    let types: Vec<ProviderMemoryType> = observation
        .supported
        .iter()
        .map(|artifact| artifact.memory_type)
        .collect();
    assert_eq!(
        types,
        vec![
            ProviderMemoryType::Memory,
            ProviderMemoryType::MemorySummary,
            ProviderMemoryType::RawMemories,
            ProviderMemoryType::RolloutSummary,
        ]
    );
    assert_eq!(observation.diagnostics.len(), 1);
    assert_eq!(observation.diagnostics[0].kind, "unsupported_artifact");
    assert_eq!(observation.diagnostics[0].observed_path, "AGENTS.md");
}

#[test]
fn recognized_allowlist_directories_fail_enumeration() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    fs::create_dir(root.join("MEMORY.md")).expect("known root name as directory");
    assert!(matches!(
        CodexAdapter.enumerate_artifacts(root),
        Err(EnumerateError::AllowlistedArtifactUnresolvable)
    ));

    fs::remove_dir(root.join("MEMORY.md")).expect("remove root directory");
    fs::create_dir(root.join("rollout_summaries")).expect("rollout dir");
    fs::create_dir(root.join("rollout_summaries").join("run.md"))
        .expect("rollout markdown name as directory");
    assert!(matches!(
        CodexAdapter.enumerate_artifacts(root),
        Err(EnumerateError::AllowlistedArtifactUnresolvable)
    ));
}

#[cfg(unix)]
#[test]
fn resolved_role_mismatch_is_excluded_without_reclassification() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("raw_memories.md"), "raw\n").expect("raw memory");
    std::os::unix::fs::symlink(root.join("raw_memories.md"), root.join("MEMORY.md"))
        .expect("mismatched alias");

    let observation = CodexAdapter
        .enumerate_artifacts(root)
        .expect("enumeration succeeds");
    assert_eq!(observation.supported.len(), 1);
    assert_eq!(
        observation.supported[0].file.relative_path,
        "raw_memories.md"
    );
    assert_eq!(
        observation.supported[0].memory_type,
        ProviderMemoryType::RawMemories
    );
}

#[cfg(unix)]
#[test]
fn native_byte_unknown_entry_is_diagnosed_when_filesystem_accepts_it() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("MEMORY.md"), "memory\n").expect("memory");
    let non_utf8 = root.join(OsString::from_vec(b"bad\xff-name".to_vec()));
    if fs::write(&non_utf8, "unknown\n").is_err() {
        // APFS rejects this name. The native-byte encoder contract below is
        // the platform-specific evidence for that case.
        return;
    }

    let observation = CodexAdapter
        .enumerate_artifacts(root)
        .expect("enumerate native-byte entry");
    assert_eq!(observation.supported.len(), 1);
    assert_eq!(observation.diagnostics.len(), 1);
    assert_eq!(observation.diagnostics[0].kind, "unsupported_artifact");
    assert_eq!(observation.diagnostics[0].observed_path, "bad%FF-name");
}

#[cfg(unix)]
#[test]
fn native_byte_diagnostic_encoder_is_reversible_when_filesystem_rejects_a_name() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = Path::new("/tmp");
    let non_utf8_name = OsString::from_vec(b"bad\xff-name".to_vec());
    let non_utf8 = Path::new(&non_utf8_name);
    assert_eq!(safe_relative_path(root, non_utf8), "bad%FF-name");
}
