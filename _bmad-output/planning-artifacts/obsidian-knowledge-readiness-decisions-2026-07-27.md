---
title: "Obsidian Knowledge Preimplementation Decisions"
status: final
date: 2026-07-27
measurement_scope: "Stat-only metadata; no note bodies read"
binds:
  - FR-21
  - FR-24
  - FR-25
  - NFR-3
  - NFR-11
  - NFR-14
---

# Obsidian Knowledge Preimplementation Decisions

## 1. Purpose

This artifact closes the preimplementation decisions identified by the
2026-07-27 Implementation Readiness assessment:

1. exact maximum Markdown note size;
2. aggregate-only performance evidence schema and privacy boundary;
3. real Obsidian visible-open evidence contract;
4. failed-measurement remediation behavior.

It does not authorize feature implementation.

## 2. Stat-only real-Vault measurement

Measurement date: 2026-07-27.

Method:

- read the local Obsidian registry to identify registered roots;
- inspect filesystem metadata only;
- include regular `.md` files under allowed non-hidden paths;
- exclude all dot-paths, `.obsidian/**`, `.git/**`, trash directories,
  symlinks, and non-Markdown artifacts;
- do not open or read any note body;
- do not persist or report Vault paths or filenames.

Current aggregate snapshot:

| Metric | Value |
| --- | ---: |
| Registered Vaults | 6 |
| Existing Vault roots | 6 |
| Supported Markdown files | 1,796 |
| Total Markdown bytes | 18,336,419 |
| Minimum file bytes | 0 |
| P50 file bytes | 3,334 |
| P95 file bytes | 35,720 |
| P99 file bytes | 48,190 |
| Maximum file bytes | 86,142 |
| Files over 1 MiB | 0 |
| Observed symlinks | 0 |

The earlier proposal recorded 1,813 Markdown files and 18,581,683 bytes using a
broader snapshot. The current decision uses the final Phase C.0 inclusion
policy and the filesystem state observed on 2026-07-27. Neither snapshot reads
note bodies.

## 3. Maximum note-size decision

**Decision**

```text
max_note_bytes = 1,048,576
```

This is exactly 1 MiB.

Rationale:

- `1,048,576 / 86,142 = 12.1726`, so the bound is 12.1726 times the current
  maximum observed note size;
- `1,048,576 / 48,190 = 21.7592`, so the bound is 21.7592 times the current
  P99;
- it admits every currently supported note;
- it gives substantial growth headroom while bounding per-note read,
  decode, parse, hash, staging, and error-handling allocations;
- it is simple to test and represent consistently across Rust, SQLite, API,
  fixtures, and diagnostics.

Enforcement:

- check file metadata before allocating or reading a body;
- reject a file whose observed size is greater than `max_note_bytes`;
- if the file grows beyond the bound during read/validation, reject the
  generation as drifting;
- return a safe `knowledge_note_too_large` diagnostic with Source and phase,
  but no note body or unredacted path;
- do not publish a partial record;
- retain the previous successful generation;
- changing this bound requires a new measured decision artifact and parser/
  policy revision; it is not a hidden runtime override.

## 4. Knowledge performance evidence contract

Canonical aggregate artifact:

`tests/benchmarks/knowledge-index.json`

Required top-level fields:

```json
{
  "schema_version": "knowledge-index-benchmark/v1",
  "fixture_revision": "opaque-non-path-id",
  "measured_at": "RFC3339 timestamp",
  "vault_count": 0,
  "note_count": 0,
  "total_bytes": 0,
  "cold_scan_ms": 0,
  "noop_reconcile_ms": 0,
  "single_note_freshness_ms": 0,
  "query_p50_ms": 0,
  "query_p95_ms": 0,
  "rss_peak_bytes": 0,
  "index_bytes": 0,
  "file_descriptors_peak": 0,
  "threads_peak": 0,
  "thresholds": {},
  "decision": "pass | remediation_required"
}
```

The committed artifact must not contain:

- note bodies or snippets;
- search text;
- filenames or Vault-relative paths;
- full or redacted Vault root paths;
- native Vault IDs or names;
- registry payloads;
- credentials;
- per-note measurements that could identify a private note.

Threshold process:

1. Story 6.11 measures the current implementation with this schema.
2. The measurement review records explicit thresholds and decision rationale.
3. If literal search, reconcile cadence, or another metric requires
   remediation, create a separately named backlog Story.
4. Story 6.11 does not implement FTS or another optimization.
5. Story 6.12 cannot begin until required remediation and remeasurement pass.

## 5. Real Obsidian visible-open evidence contract

Canonical manual evidence artifact:

`_bmad-output/test-artifacts/obsidian-open-e2e.md`

Required fields for each case:

- case ID using an opaque label;
- precondition class, such as same-name Vault or Unicode path;
- expected Vault selection outcome without revealing its real name;
- expected note selection outcome without revealing its filename;
- automated URI-construction result;
- OS dispatch result;
- human-visible result: correct Vault and correct note;
- `.obsidian` event observation and whether Tessera scheduled reconcile;
- pass/fail;
- operator and timestamp.

Redaction rules:

- no note body, title, snippet, filename, full path, native Vault ID, registry
  payload, screenshot containing private content, or raw URI;
- screenshots are optional and must use controlled fixtures or fully redacted
  visible content;
- OS dispatch success alone cannot mark the case passed;
- human-visible confirmation is required for the predetermined controlled note.

## 6. Final-gate rule

Story 6.12 is verification-only:

- it may pass;
- it may fail and name the failed contract;
- it may require a new remediation Story;
- it may not introduce schema changes, FTS, a new tokenizer, a new picker
  implementation, or any other hidden product/technical scope.

This rule prevents acceptance work from expanding unpredictably after
implementation has supposedly completed.
