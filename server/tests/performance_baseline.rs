use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::{tempdir, TempDir};

use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    CandidateSource, CoverageLevel, DiscoveryBasis,
};
use tessera_lib::domain::query::SearchRequest;
use tessera_lib::index::SourceRegistry;

const FIXTURE_ROOT: &str = "../tests/fixtures/benchmarks/codex-anonymized-v1";
const BENCHMARK_MANIFEST: &str = "../tests/benchmarks/memory-index.json";
const QUERY_RUNS: usize = 25;
const COLD_SCAN_TRIALS: usize = 5;
const RSS_PROBE_MARKER: &str = "TESSERA_PERFORMANCE_RSS_BYTES=";

#[derive(Debug, PartialEq, Eq)]
enum GateDecision {
    Open,
    Enforced,
}

#[derive(Debug)]
struct Measurements {
    cold_scan_ms: u64,
    query_p50_ms: u64,
    query_p95_ms: u64,
    rss_bytes: u64,
    index_size_bytes: u64,
}

#[test]
fn phase_zero_baseline_gate_measures_and_enforces_the_approved_fixture() {
    validate_fixture(Path::new(FIXTURE_ROOT)).expect("benchmark fixture must remain sanitised");
    let manifest = load_json(BENCHMARK_MANIFEST);
    assert_manifest_fixture(&manifest);
    assert_eq!(
        gate_decision(&manifest).expect("complete manifest policy"),
        GateDecision::Enforced
    );
    let measurements = measure_fixture();

    eprintln!(
        "performance_baseline cold_scan_ms={} query_p50_ms={} query_p95_ms={} rss_bytes={} index_size_bytes={}",
        measurements.cold_scan_ms,
        measurements.query_p50_ms,
        measurements.query_p95_ms,
        measurements.rss_bytes,
        measurements.index_size_bytes,
    );

    assert_positive_measurements(&measurements);
    assert_at_most(
        "cold_scan",
        measurements.cold_scan_ms,
        number(&manifest, &["metrics", "cold_scan", "threshold"]),
    );
    assert_at_most(
        "query_p50",
        measurements.query_p50_ms,
        number(&manifest, &["metrics", "query", "threshold_p50"]),
    );
    assert_at_most(
        "query_p95",
        measurements.query_p95_ms,
        number(&manifest, &["metrics", "query", "threshold_p95"]),
    );
    assert_at_most(
        "memory_rss",
        measurements.rss_bytes,
        number(&manifest, &["metrics", "memory", "threshold"]),
    );
    assert_at_most(
        "index_size",
        measurements.index_size_bytes,
        number(&manifest, &["metrics", "index_size", "threshold"]),
    );
}

#[test]
fn incomplete_policy_stays_open_and_is_not_mistaken_for_an_enforced_gate() {
    let incomplete = serde_json::json!({
        "metrics": {
            "cold_scan": { "baseline": null, "threshold": null },
            "query": { "baseline_p50": null, "baseline_p95": null, "threshold_p50": null, "threshold_p95": null },
            "memory": { "baseline": null, "threshold": null },
            "index_size": { "baseline": null, "threshold": null }
        },
        "gate": { "enforce": false }
    });

    assert_eq!(gate_decision(&incomplete).unwrap(), GateDecision::Open);
}

#[test]
fn enforced_policy_rejects_positive_non_double_thresholds() {
    let mut manifest = load_json(BENCHMARK_MANIFEST);
    manifest["metrics"]["memory"]["threshold"] = Value::from(3_u64);

    assert!(gate_decision(&manifest).is_err());
}

#[test]
fn duration_millis_ceil_rounds_fractional_values_up() {
    assert_eq!(duration_millis_ceil(Duration::ZERO), 0);
    assert_eq!(duration_millis_ceil(Duration::from_nanos(1)), 1);
    assert_eq!(duration_millis_ceil(Duration::from_millis(1)), 1);
    assert_eq!(duration_millis_ceil(Duration::from_nanos(1_000_001)), 2);
    assert_eq!(duration_millis_ceil(Duration::from_micros(2_001)), 3);
}

#[test]
fn fixture_verifier_rejects_each_sensitive_identifier_class() {
    for sample in [
        "path /Users/example/private",
        "credential token = demo-value",
        "credential secret : demo-value",
        "bearer demo-credential-value",
        "link https://example.invalid/private",
        "email user@example.invalid",
        "uuid 123e4567-e89b-12d3-a456-426614174000",
        "hash 0123456789abcdef0123456789abcdef",
        "prefix ghp_012345678901234567890123456789012345",
        "prefix sk-012345678901234567890123",
        "access AKIA0123456789ABCDEF",
    ] {
        assert!(
            contains_sensitive_identifier(sample),
            "sample must be rejected"
        );
    }
}

#[test]
fn fixture_verifier_rejects_residual_identifier_in_manifest_text() {
    let temp = tempdir().expect("temporary fixture");
    write_minimal_fixture(
        temp.path(),
        Some(("unexpected_note", "bearer test-credential-value")),
    );

    assert!(validate_fixture(temp.path()).is_err());
}

#[test]
fn fixture_verifier_rejects_unexpected_and_changed_content_artifacts() {
    let temp = tempdir().expect("temporary fixture");
    write_minimal_fixture(temp.path(), None);
    fs::write(temp.path().join("unexpected.txt"), "unexpected")
        .expect("write unexpected fixture file");
    assert!(validate_fixture(temp.path()).is_err());

    fs::remove_file(temp.path().join("unexpected.txt")).expect("remove temporary unexpected file");
    fs::write(
        temp.path().join("memory_summary.md"),
        "changed fixture content",
    )
    .expect("change temporary fixture content");
    assert!(validate_fixture(temp.path()).is_err());
}

#[cfg(unix)]
#[test]
fn fixture_verifier_rejects_symlink_artifacts() {
    let temp = tempdir().expect("temporary fixture");
    write_minimal_fixture(temp.path(), None);
    std::os::unix::fs::symlink(
        temp.path().join("memory_summary.md"),
        temp.path().join("linked.md"),
    )
    .expect("create temporary symlink");

    assert!(validate_fixture(temp.path()).is_err());
}

#[test]
#[ignore = "invoked by the performance gate in a dedicated Tessera child process"]
fn rss_probe_protocol() {
    let (_app_data, state) = fresh_scanned_state();
    let conn = state.conn.lock().expect("index connection lock");
    let registry = SourceRegistry::new(&conn);
    assert_declared_queries_resolve(&registry, &conn, Path::new(FIXTURE_ROOT));
    let rss_bytes = current_process_rss_bytes().expect("read isolated Tessera process RSS");
    println!("{RSS_PROBE_MARKER}{rss_bytes}");
}

fn measure_fixture() -> Measurements {
    let cold_scan_ms = percentile(
        &(0..COLD_SCAN_TRIALS)
            .map(|_| measure_cold_scan_trial())
            .collect::<Vec<_>>(),
        50,
    );
    let (_app_data, state) = fresh_scanned_state();
    let conn = state.conn.lock().expect("index connection lock");
    let registry = SourceRegistry::new(&conn);
    assert_declared_queries_resolve(&registry, &conn, Path::new(FIXTURE_ROOT));

    let mut query_samples =
        Vec::with_capacity(QUERY_RUNS * declared_queries(Path::new(FIXTURE_ROOT)).len());
    for _ in 0..QUERY_RUNS {
        for query in declared_queries(Path::new(FIXTURE_ROOT)) {
            let started = Instant::now();
            let results = application::search(
                &registry,
                &conn,
                SearchRequest::new(query.to_string(), None, None).expect("valid benchmark query"),
            )
            .expect("query benchmark fixture");
            assert!(
                !results.results().is_empty(),
                "declared query identifier must resolve"
            );
            query_samples.push(duration_millis_ceil(started.elapsed()));
        }
    }
    query_samples.sort_unstable();

    let index_size_bytes = fs::metadata(&state.db_path)
        .expect("Derived Index database metadata")
        .len();

    Measurements {
        cold_scan_ms,
        query_p50_ms: percentile(&query_samples, 50),
        query_p95_ms: percentile(&query_samples, 95),
        rss_bytes: isolated_process_rss_bytes(),
        index_size_bytes,
    }
}

fn measure_cold_scan_trial() -> u64 {
    let app_data = tempdir().expect("temporary app data");
    let state = tessera_lib::boot(app_data.path()).expect("boot isolated Derived Index");
    let conn = state.conn.lock().expect("index connection lock");
    let registry = SourceRegistry::new(&conn);
    let source = confirm_fixture_source(&registry);
    let started = Instant::now();
    application::scan_source(&registry, &conn, &source.source_id).expect("scan benchmark fixture");
    duration_millis_ceil(started.elapsed())
}

fn fresh_scanned_state() -> (TempDir, tessera_lib::IndexState) {
    let app_data = tempdir().expect("temporary app data");
    let state = tessera_lib::boot(app_data.path()).expect("boot isolated Derived Index");
    let conn = state.conn.lock().expect("index connection lock");
    let registry = SourceRegistry::new(&conn);
    let source = confirm_fixture_source(&registry);
    application::scan_source(&registry, &conn, &source.source_id).expect("scan benchmark fixture");
    drop(conn);
    (app_data, state)
}

fn confirm_fixture_source(registry: &SourceRegistry<'_>) -> tessera_lib::domain::source::Source {
    application::confirm_source(
        registry,
        &CandidateSource {
            provider: "codex".to_string(),
            root_path: canonical_fixture_root(),
            basis: DiscoveryBasis::CodexHomeEnv,
            coverage_level: CoverageLevel::Full,
            native_project: None,
        },
    )
    .expect("confirm benchmark fixture")
}

fn assert_positive_measurements(measurements: &Measurements) {
    assert!(
        measurements.cold_scan_ms > 0,
        "cold_scan measurement must be positive"
    );
    assert!(
        measurements.query_p50_ms > 0,
        "query P50 measurement must be positive"
    );
    assert!(
        measurements.query_p95_ms > 0,
        "query P95 measurement must be positive"
    );
    assert!(
        measurements.rss_bytes > 0,
        "RSS measurement must be positive"
    );
    assert!(
        measurements.index_size_bytes > 0,
        "Derived Index size must be positive"
    );
}

fn write_minimal_fixture(root: &Path, extra_manifest_entry: Option<(&str, &str)>) {
    let memory = root.join("memory_summary.md");
    fs::write(&memory, "# Safe fixture\n\nlocal-first benchmark text.\n")
        .expect("write temporary fixture content");
    let digest = sha256_hex(&fs::read(&memory).expect("read temporary fixture content"));
    let mut fixture_manifest = serde_json::json!({
        "fixture_version": "test",
        "sanitisation": ["test-only"],
        "queries": ["local_first"],
        "content_digests": { "memory_summary.md": digest }
    });
    if let Some((key, value)) = extra_manifest_entry {
        fixture_manifest[key] = Value::from(value);
    }
    fs::write(
        root.join("fixture-manifest.json"),
        serde_json::to_vec(&fixture_manifest).expect("serialize temporary fixture manifest"),
    )
    .expect("write temporary fixture manifest");
}

fn validate_fixture(root: &Path) -> Result<(), String> {
    let manifest_path = root.join("fixture-manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|_| "fixture manifest is not readable UTF-8 text".to_string())?;
    let fixture_manifest: Value = serde_json::from_str(&manifest_text)
        .map_err(|_| "fixture manifest is not valid JSON".to_string())?;
    fixture_version(&fixture_manifest)?;
    let queries = fixture_manifest
        .get("queries")
        .and_then(Value::as_array)
        .ok_or_else(|| "fixture manifest has no query identifiers".to_string())?;
    if queries
        .iter()
        .any(|query| query.as_str().is_none_or(str::is_empty))
    {
        return Err("fixture manifest contains an invalid query identifier".to_string());
    }
    let digests = fixture_content_digests(&fixture_manifest)?;
    let expected_files: BTreeSet<String> = std::iter::once("fixture-manifest.json".to_string())
        .chain(digests.keys().cloned())
        .collect();
    let expected_directories = expected_fixture_directories(&digests)?;
    let mut actual_files = BTreeMap::new();
    collect_fixture_files(root, root, &expected_directories, &mut actual_files)?;
    if actual_files.keys().collect::<BTreeSet<_>>()
        != expected_files.iter().collect::<BTreeSet<_>>()
    {
        return Err("fixture has unexpected, missing, or non-regular artifacts".to_string());
    }

    let manifest_for_privacy_scan = redact_approved_digests(&manifest_text, digests.values());
    if contains_sensitive_identifier(&manifest_for_privacy_scan) {
        return Err("fixture manifest contains a residual sensitive identifier".to_string());
    }
    for (relative, expected_digest) in &digests {
        let path = actual_files
            .get(relative)
            .ok_or_else(|| "fixture content file is missing".to_string())?;
        let bytes = fs::read(path).map_err(|_| "fixture content file is unreadable".to_string())?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "fixture content file is not UTF-8 text".to_string())?;
        if contains_sensitive_identifier(text) {
            return Err("fixture content contains a residual sensitive identifier".to_string());
        }
        if sha256_hex(&bytes) != *expected_digest {
            return Err("fixture content digest does not match its pinned identity".to_string());
        }
    }
    Ok(())
}

fn fixture_content_digests(manifest: &Value) -> Result<BTreeMap<String, String>, String> {
    let digests = manifest
        .get("content_digests")
        .and_then(Value::as_object)
        .ok_or_else(|| "fixture manifest has no content digests".to_string())?;
    if digests.is_empty() {
        return Err("fixture manifest has no pinned content files".to_string());
    }
    let mut result = BTreeMap::new();
    for (relative, digest) in digests {
        let path = Path::new(relative);
        if !path.is_relative()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || path.extension().and_then(|extension| extension.to_str()) != Some("md")
        {
            return Err("fixture manifest has an invalid content path".to_string());
        }
        let digest = digest
            .as_str()
            .filter(|value| {
                value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
            })
            .ok_or_else(|| "fixture manifest has an invalid content digest".to_string())?;
        result.insert(relative.clone(), digest.to_ascii_lowercase());
    }
    Ok(result)
}

fn fixture_version(manifest: &Value) -> Result<&str, String> {
    manifest
        .get("fixture_version")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or_else(|| "fixture manifest has no version".to_string())
}

fn fixture_identity_digest(manifest: &Value) -> Result<String, String> {
    let version = fixture_version(manifest)?;
    let queries = manifest
        .get("queries")
        .and_then(Value::as_array)
        .ok_or_else(|| "fixture manifest has no query identifiers".to_string())?;
    let digests = fixture_content_digests(manifest)?;
    let mut identity = format!("fixture_version={version}\n");
    for query in queries {
        let query = query
            .as_str()
            .filter(|query| !query.is_empty())
            .ok_or_else(|| "fixture manifest contains an invalid query identifier".to_string())?;
        identity.push_str("query=");
        identity.push_str(query);
        identity.push('\n');
    }
    for (path, digest) in digests {
        identity.push_str("file=");
        identity.push_str(&path);
        identity.push('\n');
        identity.push_str("digest=");
        identity.push_str(&digest);
        identity.push('\n');
    }
    Ok(sha256_hex(identity.as_bytes()))
}

fn expected_fixture_directories(
    digests: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>, String> {
    let mut directories = BTreeSet::new();
    for path in digests.keys().map(Path::new) {
        let mut current = PathBuf::new();
        for component in path.parent().into_iter().flat_map(Path::components) {
            let Component::Normal(part) = component else {
                return Err("fixture manifest has an invalid content path".to_string());
            };
            current.push(part);
            directories.insert(current.to_string_lossy().into_owned());
        }
    }
    Ok(directories)
}

fn collect_fixture_files(
    root: &Path,
    directory: &Path,
    expected_directories: &BTreeSet<String>,
    files: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(directory).map_err(|_| "fixture directory is unreadable".to_string())?
    {
        let entry = entry.map_err(|_| "fixture directory entry is unreadable".to_string())?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "fixture path escapes its root".to_string())?
            .to_string_lossy()
            .into_owned();
        let file_type = entry
            .file_type()
            .map_err(|_| "fixture file type is unreadable".to_string())?;
        if file_type.is_symlink() {
            return Err("fixture symlinks are not allowed".to_string());
        }
        if file_type.is_dir() {
            if !expected_directories.contains(&relative) {
                return Err("fixture has an unexpected directory".to_string());
            }
            collect_fixture_files(root, &path, expected_directories, files)?;
        } else if file_type.is_file() {
            if files.insert(relative, path).is_some() {
                return Err("fixture contains duplicate paths".to_string());
            }
        } else {
            return Err("fixture contains a non-regular artifact".to_string());
        }
    }
    Ok(())
}

fn redact_approved_digests<'a>(text: &str, digests: impl Iterator<Item = &'a String>) -> String {
    digests.fold(text.to_string(), |redacted, digest| {
        redacted.replace(digest, "approved-fixture-digest")
    })
}

fn contains_sensitive_identifier(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("/users/")
        || lower.contains("/home/")
        || lower.contains("~/")
        || lower.contains("~\\")
        || lower.contains("https://")
        || lower.contains("http://")
        || contains_credential_assignment(&lower)
        || contains_bearer_credential(text)
        || text.split_whitespace().any(looks_like_absolute_path)
        || text.split_whitespace().any(looks_like_email)
        || text.split_whitespace().any(looks_like_uuid)
        || text.split_whitespace().any(looks_like_hash)
        || text.split_whitespace().any(looks_like_credential_prefix)
}

fn contains_credential_assignment(lower: &str) -> bool {
    [
        "token",
        "secret",
        "password",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
    ]
    .iter()
    .any(|key| {
        lower.match_indices(key).any(|(offset, _)| {
            let key_is_bounded = offset == 0
                || !matches!(
                    lower.as_bytes()[offset - 1],
                    b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'
                );
            let suffix = lower[offset + key.len()..].trim_start();
            let value = suffix
                .strip_prefix('=')
                .or_else(|| suffix.strip_prefix(':'))
                .map(str::trim_start)
                .unwrap_or_default();
            key_is_bounded
                && value
                    .chars()
                    .next()
                    .is_some_and(|character| !matches!(character, '"' | '\'' | ',' | ';'))
        })
    })
}

fn contains_bearer_credential(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("bearer").any(|(offset, _)| {
        let before_is_boundary =
            offset == 0 || !lower.as_bytes()[offset - 1].is_ascii_alphanumeric();
        let value = lower[offset + "bearer".len()..]
            .trim_start_matches(|character: char| !character.is_ascii_alphanumeric());
        let value = value
            .split(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-' && character != '_'
            })
            .next()
            .unwrap_or_default();
        before_is_boundary
            && value.len() >= 12
            && value
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
    })
}

fn looks_like_absolute_path(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        matches!(
            character,
            '`' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | '.'
        )
    });
    token.starts_with('/')
        || token.starts_with("\\\\")
        || (token.len() >= 3
            && token.as_bytes()[0].is_ascii_alphabetic()
            && token.as_bytes()[1] == b':'
            && matches!(token.as_bytes()[2], b'/' | b'\\'))
}

fn looks_like_email(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric()
            && character != '@'
            && character != '.'
            && character != '_'
            && character != '-'
    });
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
}

fn looks_like_uuid(token: &str) -> bool {
    let token =
        token.trim_matches(|character: char| !character.is_ascii_hexdigit() && character != '-');
    let groups: Vec<_> = token.split('-').collect();
    [8, 4, 4, 4, 12] == groups.iter().map(|group| group.len()).collect::<Vec<_>>()[..]
        && groups
            .iter()
            .all(|group| group.chars().all(|character| character.is_ascii_hexdigit()))
}

fn looks_like_hash(token: &str) -> bool {
    let token = token.trim_matches(|character: char| !character.is_ascii_hexdigit());
    matches!(token.len(), 32 | 40 | 64)
        && token.chars().all(|character| character.is_ascii_hexdigit())
}

fn looks_like_credential_prefix(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '_' && character != '-'
    });
    let lower = token.to_ascii_lowercase();
    (lower.starts_with("ghp_") && token.len() >= 20)
        || (lower.starts_with("github_pat_") && token.len() >= 24)
        || (lower.starts_with("sk-") && token.len() >= 16)
        || ((token.starts_with("AKIA") || token.starts_with("ASIA"))
            && token.len() == 20
            && token
                .chars()
                .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit()))
}

fn canonical_fixture_root() -> String {
    fs::canonicalize(FIXTURE_ROOT)
        .expect("canonical benchmark fixture root")
        .to_string_lossy()
        .into_owned()
}

fn declared_queries(root: &Path) -> Vec<&'static str> {
    let manifest = load_json_path(&root.join("fixture-manifest.json"));
    manifest["queries"]
        .as_array()
        .expect("fixture query identifiers")
        .iter()
        .map(|query| match query.as_str() {
            Some("local_first") => "local-first",
            Some("onboarding_template") => "template",
            Some("discoverability_metadata") => "discoverability",
            _ => panic!("unknown fixture query identifier"),
        })
        .collect()
}

fn assert_declared_queries_resolve(
    registry: &SourceRegistry<'_>,
    conn: &rusqlite::Connection,
    root: &Path,
) {
    for query in declared_queries(root) {
        let results = application::search(
            registry,
            conn,
            SearchRequest::new(query.to_string(), None, None).expect("valid fixture query"),
        )
        .expect("query fixture");
        assert!(
            !results.results().is_empty(),
            "declared query identifier must resolve"
        );
    }
}

fn duration_millis_ceil(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos().div_ceil(1_000_000)).expect("duration fits in u64")
}

fn percentile(samples: &[u64], percentile: usize) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() * percentile).div_ceil(100)).saturating_sub(1);
    sorted[index]
}

fn isolated_process_rss_bytes() -> u64 {
    let executable = std::env::current_exe().expect("current test executable");
    let output = Command::new(executable)
        .args(["--ignored", "--exact", "rss_probe_protocol", "--nocapture"])
        .output()
        .expect("start isolated Tessera RSS probe");
    assert!(output.status.success(), "isolated Tessera RSS probe failed");
    let output = std::str::from_utf8(&output.stdout).expect("RSS probe output is UTF-8");
    let values: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix(RSS_PROBE_MARKER))
        .map(|value| value.parse::<u64>().expect("RSS probe value is numeric"))
        .collect();
    assert_eq!(values.len(), 1, "RSS probe must emit exactly one metric");
    values[0]
}

fn current_process_rss_bytes() -> Result<u64, String> {
    let process_id = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &process_id])
        .output()
        .map_err(|_| "could not execute ps".to_string())?;
    if !output.status.success() {
        return Err("ps did not report current process RSS".to_string());
    }
    let kibibytes = std::str::from_utf8(&output.stdout)
        .map_err(|_| "ps RSS output was not UTF-8".to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|_| "ps RSS output was not numeric".to_string())?;
    kibibytes
        .checked_mul(1024)
        .ok_or_else(|| "RSS value overflowed bytes".to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_json(path: &str) -> Value {
    load_json_path(Path::new(path))
}

fn load_json_path(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read JSON manifest"))
        .expect("parse JSON manifest")
}

fn assert_manifest_fixture(manifest: &Value) {
    assert_eq!(
        manifest.pointer("/fixture/name").and_then(Value::as_str),
        Some("codex-anonymized-v1"),
        "benchmark manifest must name the fixed approved fixture"
    );
    assert_eq!(
        manifest
            .pointer("/fixture/seed_path")
            .and_then(Value::as_str),
        Some("tests/fixtures/benchmarks/codex-anonymized-v1"),
        "benchmark manifest must retain the fixed fixture location"
    );
    let fixture_manifest = load_json_path(&Path::new(FIXTURE_ROOT).join("fixture-manifest.json"));
    let fixture_version = fixture_version(&fixture_manifest).expect("fixture version");
    assert_eq!(
        manifest.pointer("/fixture/version").and_then(Value::as_str),
        Some(fixture_version),
        "benchmark manifest must pin the fixture version"
    );
    let fixture_identity = fixture_identity_digest(&fixture_manifest).expect("fixture identity");
    assert_eq!(
        manifest
            .pointer("/fixture/identity_digest")
            .and_then(Value::as_str),
        Some(fixture_identity.as_str()),
        "benchmark manifest must pin the fixture identity"
    );
    assert_eq!(
        number(manifest, &["metrics", "cold_scan", "collection", "trials"]) as usize,
        COLD_SCAN_TRIALS,
        "cold scan trial count must be pinned"
    );
    assert_eq!(
        manifest
            .pointer("/metrics/cold_scan/collection/statistic")
            .and_then(Value::as_str),
        Some("median"),
        "cold scan statistic must be pinned"
    );
    assert_eq!(
        manifest
            .pointer("/metrics/memory/collection/process")
            .and_then(Value::as_str),
        Some("dedicated_tessera_probe"),
        "RSS must come from the dedicated probe"
    );
}

fn gate_decision(manifest: &Value) -> Result<GateDecision, String> {
    let complete = [
        ["metrics", "cold_scan", "baseline"],
        ["metrics", "cold_scan", "threshold"],
        ["metrics", "query", "baseline_p50"],
        ["metrics", "query", "baseline_p95"],
        ["metrics", "query", "threshold_p50"],
        ["metrics", "query", "threshold_p95"],
        ["metrics", "memory", "baseline"],
        ["metrics", "memory", "threshold"],
        ["metrics", "index_size", "baseline"],
        ["metrics", "index_size", "threshold"],
    ]
    .into_iter()
    .all(|path| number_optional(manifest, &path).is_some_and(|value| value > 0));
    let enforce = manifest
        .pointer("/gate/enforce")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match (complete, enforce) {
        (true, true) => {
            validate_threshold_policy(manifest)?;
            Ok(GateDecision::Enforced)
        }
        (false, false) => Ok(GateDecision::Open),
        (false, true) => Err("gate is enabled with incomplete metric policy".to_string()),
        (true, false) => Err("complete metric policy is not enforced".to_string()),
    }
}

fn validate_threshold_policy(manifest: &Value) -> Result<(), String> {
    for (baseline, threshold) in [
        (
            ["metrics", "cold_scan", "baseline"],
            ["metrics", "cold_scan", "threshold"],
        ),
        (
            ["metrics", "query", "baseline_p50"],
            ["metrics", "query", "threshold_p50"],
        ),
        (
            ["metrics", "query", "baseline_p95"],
            ["metrics", "query", "threshold_p95"],
        ),
        (
            ["metrics", "memory", "baseline"],
            ["metrics", "memory", "threshold"],
        ),
        (
            ["metrics", "index_size", "baseline"],
            ["metrics", "index_size", "threshold"],
        ),
    ] {
        let baseline = number(manifest, &baseline);
        let expected = baseline
            .checked_mul(2)
            .ok_or_else(|| "baseline threshold overflows 2x policy".to_string())?;
        if number(manifest, &threshold) != expected {
            return Err("threshold does not equal the required 2x baseline".to_string());
        }
    }
    if number(manifest, &["metrics", "query", "baseline_p95"])
        < number(manifest, &["metrics", "query", "baseline_p50"])
        || number(manifest, &["metrics", "query", "threshold_p95"])
            < number(manifest, &["metrics", "query", "threshold_p50"])
    {
        return Err("query P95 must be at least query P50".to_string());
    }
    Ok(())
}

fn number(manifest: &Value, path: &[&str]) -> u64 {
    number_optional(manifest, path).expect("positive numeric manifest field")
}

fn number_optional(manifest: &Value, path: &[&str]) -> Option<u64> {
    let pointer = format!("/{}", path.join("/"));
    manifest.pointer(&pointer)?.as_u64()
}

fn assert_at_most(metric: &str, measured: u64, threshold: u64) {
    assert!(
        measured <= threshold,
        "performance gate failed for {metric}: measured={measured} threshold={threshold}"
    );
}
