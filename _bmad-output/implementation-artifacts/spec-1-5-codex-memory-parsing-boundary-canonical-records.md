---
title: 'Story 1.5: Codex Memory Parsing, Boundaries, and Canonical Records'
type: 'feature'
created: '2026-07-23'
status: 'done'
baseline_revision: '19b63da'
final_revision: '6b0ed3c69af8c858cc3ac7f46478fb79e6f8d7d7'
review_loop_iteration: 5
followup_review_recommended: false
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-1-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-1-4-scan-pipeline.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md'
warnings: [oversized]
---

<intent-contract>

## Intent

**Problem:** Story 1.4 indexes every supported Codex Markdown file as one opaque file-level row. It cannot distinguish generated memory sections, preserve their provenance, or diagnose files outside the supported artifact matrix.

**Approach:** Extend the existing Codex scan path to parse only allowlisted Markdown into section-level canonical records, persist their identity and provenance in the existing staging-generation model, and retain an explicit diagnostic for unsupported in-root artifacts.

## Boundaries & Constraints

**Always:** Keep the Rust core as the only file and SQLite boundary; preserve Story 1.4's manifest validation, fencing/CAS, generation isolation, source revalidation, and zero-write behavior. Allow only `MEMORY.md`, `memory_summary.md`, `raw_memories.md`, and direct `rollout_summaries/*.md` files inside a confirmed Codex root. A record identity must remain stable across body edits: hash `source_id + provider + stable semantic locator + unit_kind`; a display line range must not participate in that identity. Preserve the Source's `native_project` as-is, including `None` for unmapped Codex memory roots. Treat all Markdown as untrusted data: do not render it, log body text, or add a network/API surface.

**Block If:** The accepted planning contracts require an incompatible definition of a canonical record identity, or the existing schema cannot preserve both prior active generations and the required canonical provenance through an additive migration.

**Never:** Do not index JSONL, sessions, state-database conversation data, `CLAUDE.md`, `AGENTS.md`, rules, arbitrary files, or paths outside the confirmed root. Do not infer a Native Project from the path or body. Do not add search, UI result rendering, FTS, external dependencies, writeback, or a second scan/write path.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Section parse | Allowlisted Markdown with leading prose, nested/repeated headings, CRLF/bare-CR lines, Setext headings, and fenced code | Deterministic non-overlapping records: a preamble for leading prose, then heading-path plus structural duplicate ordinal records; each heading owns content only until the next heading of any level; title/body, a percent-encoded semantic file-URI locator, and independent `file://…#Lx-Ly` display locator | No error expected |
| File fallback | Allowlisted Markdown with no usable headings | One file-level record with the file URI identity locator and complete file body | No error expected |
| Artifact boundary | JSONL/session/rule file or unknown in-root file (including non-UTF-8 names) beside allowlisted Markdown | Excluded/unknown lexical entries create reversible safe persisted `unsupported_artifact` diagnostics; no canonical row is staged | No source content is persisted as a record |
| Malformed source | An allowlisted file is unreadable or cannot be canonicalized | The run fails with a safe source-scoped parse error before commit; prior active records remain and no diagnostic-only generation activates | No source content is persisted in errors or diagnostics |
| Reconciliation | An already active source is rescanned after section text or line positions change | New canonical generation becomes active only after existing manifest/CAS checks; unchanged semantic records retain IDs while their content hash, source revision, or display range may change | A failed scan retains the prior active generation |
| Path safety | Symlinked or retargeted file escapes the confirmed root or resolves to a root-internal path outside its allowlist role | No record or diagnostic from the target is staged | Existing scan failure/dirty handling applies |
| Upgrade | A populated v3 index has an active file-level generation | Migration keeps the Source Registry but atomically invalidates obsolete derived scan state before blank canonical-provenance fields can be served | Next scan rebuilds the derived index |

</intent-contract>

## Code Map

- `server/src/domain/ports/provider_adapter.rs` -- Codex artifact-enumeration port and `FileUnit` boundary to extend with canonicalization output and diagnostics.
- `server/src/adapters/codex.rs` -- allowlist, realpath containment, safe artifact diagnostics, and Codex-specific Markdown parser/canonicalizer.
- `server/src/application/scan.rs` -- current sole scan write path; replace file-level staging only, preserving manifest, failure, and CAS behavior.
- `server/src/domain/scan.rs` -- stable record-ID primitive and scan-domain types.
- `server/src/index/migrations.rs` -- additive migration after v3 for canonical provenance and diagnostics, with a safe derived-index upgrade reset.
- `server/src/index/scan_store.rs` -- staged-record persistence and diagnostic writes, still using plain inserts into a generation.
- `server/tests/scan_pipeline.rs` -- existing generation, recovery, zero-mutation, and containment regression coverage.
- `server/tests/codex_canonicalization.rs` -- new parser/fixture contract tests.
- `server/tests/fixtures/providers/codex/` -- anonymous Markdown and excluded-artifact fixtures.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/ports/provider_adapter.rs` and `server/src/adapters/codex.rs` -- model allowlisted-artifact enumeration plus diagnostics and implement a deterministic, dependency-free Markdown canonicalizer. Normalize CRLF/bare-CR line endings; support ATX and Setext headings; recognize fences only at valid indentation and valid closers; preserve every non-empty pre-heading source range, including whitespace-only lines, as a preamble; and end every heading record at the next heading of any level. All UTF-8 heading parsing must be char-boundary safe and return a typed parse error rather than panic. A Setext underline consumes its immediately preceding outside-fence text line so it cannot become another heading; a would-be Setext title that itself is a valid Setext underline, is inside a consumed fence, or is inside an already consumed heading span is plain body text; the underline consists only of one repeated `=` or `-` marker. ATX, fence, and Setext markers accept only zero to three leading ASCII spaces; a leading tab or four-or-more spaces is plain body text. Treat a backtick opener whose info string contains a backtick as plain body text; a fence closer must use the same marker, be at least its opener width, and have no trailing non-whitespace. Preserve the complete normalized body range, including trailing blank lines before the next heading. Derive structural heading paths and sibling duplicate ordinals with collision-safe keys; fall back to one file unit only when no usable heading exists. Canonicalize paths first for containment, then require the resolved root-relative target to satisfy the same allowlist role; produce diagnostics from the observed lexical entry. A recognized allowlist name whose target cannot be resolved, inspected, or read is a terminal typed enumeration failure; only a proven root escape or resolved-role mismatch is silently excluded. Build percent-encoded file URIs and reversible safe diagnostic paths without dependencies; classify all four allowed artifact forms; create non-UTF-8 fixture entries using the Rust standard library and require their diagnostic on platforms whose filesystem accepts such names, while preserving a native-byte encoder unit test on macOS where APFS rejects them; malformed allowlisted files are terminal safe scan failures, not active diagnostics.
- `server/src/domain/scan.rs` and `server/src/application/scan.rs` -- build each section's stable semantic locator from the canonical file URI and unit identity, retain a separate display locator/range, set record content hash from record content and source revision from whole source-file bytes, and stage coverage level plus observed-at time with every record through the existing scan pipeline. Capture every allowlisted file's byte digest after the initial read and, immediately before every final re-read, prove the resolved target still matches the root-contained snapshot; re-check containment after the read and compare the digest. Any mismatch is `DirtyAfterValidation`, even when path, size, and mtime are unchanged. Final validation must compare complete `ArtifactEnumeration` data: supported units, roles/metadata, and sorted safe diagnostics. An empty supported-file enumeration must preserve an existing active generation even if diagnostics exist. Rename any remaining file-level-only count documentation to canonical-record terminology.
- `server/src/index/migrations.rs` and `server/src/index/scan_store.rs` -- add an additive v4 migration and extend staged persistence for title, body, native project, provider-memory type, coverage level, observed-at time, source revision, and display location; add a source-scoped diagnostic projection for `unsupported_artifact`. A populated v3 active generation lacks required canonical fields, so migration must atomically clear derived records, scan runs, and active-generation markers while preserving Source Registry rows. Preserve composite `(record_id, generation)` isolation and generation-scoped diagnostic cleanup.
- `server/tests/fixtures/providers/codex/` and `server/tests/codex_canonicalization.rs` -- add fixture contracts covering all supported artifact types, leading prose including whitespace-only preamble ranges, nested/repeated headings with non-overlap, adjacent Setext underlines, delimiter-only/mixed/indented Setext-looking lines, invalid backtick openers, trailing blank body lines, CRLF and bare-CR normalization, Unicode ATX titles with and without legal closing hashes, ATX/Setext/line endings/fence boundaries, fallback, encoded locators, stable identity versus display lines, parser version, actual required non-UTF-8 filesystem-entry diagnostics made through Rust stdlib on filesystems that support them plus native-byte encoder evidence on APFS, lexical diagnostic provenance, malformed source behavior, and capability honesty. Assert every persisted supported file is assigned its exact memory type. Assert malformed allowlisted input creates a failed `scan_runs` record with `error_code='parse_failed'`, remains non-active, and maps through the existing scan HTTP route to the safe source-scoped `scan_failed` envelope without source bytes or paths. Reconciliation must show the edited section's hash changes, an unchanged sibling's hash remains, and both source revisions change.
- `server/tests/scan_pipeline.rs` -- prove canonical title/body and all four memory types persist through the sole scan path; assert exact persisted preamble/repeated-heading/fallback `record_id`, `native_unit_id`, semantic `native_locator`, and separate line-based `display_locator`; verify an edited section preserves its ID while changing its content hash and whole-file source revision, while an unchanged sibling keeps its content hash; prove a same-size, restored-mtime source-byte change detected before CAS fails dirty and leaves the prior generation active; prove a target retargeted during final digest verification is not read outside the confirmed root; prove diagnostic-only enumeration preserves active records/generation; prove diagnostic drift between first and final enumeration fails dirty and preserves the prior active generation; mapped and unmapped native-project/coverage/observed-at propagation; v3 upgrade reset; diagnostic cleanup across successful, failed, and stale generations, including persisted diagnostic kind, lexical observed path, committed generation, no matching canonical record, and recovery removal of stale diagnostic rows; no malformed-source scan replaces an active generation; source files remain byte-for-byte unchanged; boot recovery preserves the prior active generation; and root-containment plus resolved allowlist-role checks reject escapes and aliases.

**Acceptance Criteria:**
- Given a confirmed Codex source containing each supported Markdown artifact, when it is scanned, then every canonical record persists its unit kind, heading-path/duplicate-ordinal or file fallback identity, title/body, provider memory type, unchanged native-project mapping, coverage level, observed-at time, source revision, parser version, stable semantic locator, and separate display line range.
- Given an existing v3 active file-level index, when v4 migrates it, then obsolete derived scan state is not served with blank canonical provenance and the confirmed Source Registry remains available for a clean rebuild.
- Given a record's body or surrounding line positions change while its semantic section remains, when the source is rescanned successfully, then its ID remains stable and only revision/content/display provenance changes; failed or dirty scans leave the previous active generation visible.
- Given a candidate file is a transcript/rollout JSONL, session, state conversation, `CLAUDE.md`, `AGENTS.md`, rules file, unknown in-root artifact, or a root-escaping symlink, when canonicalization runs, then it never becomes an indexed canonical record; unknown in-root artifacts have an `unsupported_artifact` diagnostic without body persistence.
- Given the Codex parser fixtures and scan regressions, when the focused tests, complete Rust suite, and lint run, then fixture contract, zero-source-mutation, parser-version, reconcile-recovery, and capability-honesty coverage all pass without regressing Story 1.4.

## Design Notes

- The record ID formula remains the existing four-part primitive. The semantic locator must therefore distinguish sibling sections; it uses a canonical file URI plus a deterministic heading-path/ordinal fragment. `display_locator` is a separate file URI with `#Lstart-Lend`, so edits that only move lines never change identity.
- `source_revision` is the hash of the complete source file used for the scan; `content_hash` is the record's canonical title/body content hash. They answer different questions and must stay separate.
- `codex-markdown/v1` is the parser version for this contract. A fixture change that changes output requires an intentional parser-version decision, never silent output drift.
- File URI paths and fragments are percent-encoded independently. Heading-frame identity uses length-prefixed structural segments and each ancestor's ordinal, never delimiter-joined display text.
- Markdown ownership is intentionally flat: preamble owns leading prose; every heading record owns text after its heading until the next heading at any level. Nested headings therefore never duplicate a child body in the parent record.

## Spec Change Log

### 2026-07-23 — review repair loop 1

Review found that the first specification did not define safe v3 upgrade behavior, complete Markdown section boundaries, URI normalization, malformed-artifact diagnostics, or the corresponding regression coverage. The execution and test requirements now require an atomic derived-index reset on v3 upgrade; lossless, non-overlapping Markdown canonicalization; encoded locators; reversible diagnostics; and focused upgrade/provenance/cleanup coverage. This avoids serving blank canonical fields, silently omitting memory content, unstable identities, malformed paths, or diagnostics from failed generations. **KEEP:** retain the dependency-free parser, the existing allowlist/root-containment boundary, the sole scan write path, composite generation isolation, and all passing Story 1.4 regression guarantees.

### 2026-07-23 — review repair loop 2

The second review found a direct conflict between non-overlapping records and same-or-shallower heading boundaries, plus unspecified symlink allowlist revalidation, malformed-source commit behavior, complete provenance, and formatting coverage. The contract now gives each heading content only until the next heading of any level; verifies both lexical and resolved artifact roles; fails malformed allowlisted sources before commit; persists coverage and observation provenance; and requires end-to-end persistence, alias, and formatter checks. This avoids duplicate search bodies, rules/transcripts entering through in-root aliases, malformed scans deleting active records, incomplete provenance, and a failing formatting gate. **KEEP:** retain v3 derived-state invalidation, deterministic URI encoding, collision-safe IDs, generation-scoped diagnostics, and all new tests that prove upgrade and source-revision safety.

### 2026-07-23 — review repair loop 3

The third review found incomplete supported Markdown grammar and acceptance evidence: Setext/title consumption could overlap or panic, malformed backtick fences could suppress real headings, range reconstruction could lose trailing blank content, and the end-to-end assertions did not prove identity, changed-content, diagnostic, diagnostic-only-generation, or required non-UTF-8 behavior. The execution and test requirements now define fence and Setext consumption precisely, preserve complete normalized ranges, and require the missing SQLite-boundary and filesystem assertions. This avoids malformed confirmed memory sources disrupting scans, canonical content silently losing whitespace, malformed markdown changing record granularity, and regressions passing without proof at the persisted consumer surface. **KEEP:** retain the dependency-free parser, flat next-heading ownership, source/file identity separation, all containment and allowlist checks, the sole scan path, v3 reset semantics, generation-scoped diagnostics, and all prior source-integrity/recovery coverage.

### 2026-07-23 — review repair loop 4

The fourth review found that the grammar and scan validation still admitted silent data loss: known allowlist targets could disappear on inspection failure, Setext indentation and whitespace-only preamble ownership were unspecified, and final validation trusted mutable size/mtime metadata instead of the source bytes used to stage records. It also found missing persisted parse-failure and existing HTTP-envelope evidence. The requirements now make all inspected allowlist failures terminal, precisely bound Markdown indentation/preamble behavior, revalidate per-file byte digests before CAS, and require the missing status and transport tests. This avoids a successful scan silently omitting corrupted supported memories, stale records becoming active after metadata-preserving edits, indented code becoming identities, and safe failure behavior lacking consumer-surface proof. **KEEP:** retain typed safe errors, the existing HTTP route rather than any new endpoint, all prior parser grammar, byte-free diagnostics, sole scan path, CAS/generation isolation, v3 reset, and the focused persistence/recovery tests.

### 2026-07-23 — review repair loop 5

The fifth review found remaining untrusted-input and final-observation gaps: byte-indexed Unicode title handling could panic; Setext grammar admitted invalid marker lines; final digest reads could occur after a target escaped root; and diagnostics were neither revalidated nor fully covered through reconciliation and recovery. The contract now requires char-safe parsing, exact Setext grammar, containment before and after final reads, complete artifact-enumeration equality, and focused parser/scan tests for those cases. This avoids a valid UTF-8 title terminating the local server, outside-root content being read during a scan, and stale diagnostic state reaching the active projection. **KEEP:** retain all prior typed failure, valid-fence, root-role, byte-digest, HTTP safety, provenance, migration, and zero-write behavior; retain the APFS-specific evidence note rather than claiming unsupported invalid filename creation succeeded locally.

## Review Triage Log

### 2026-07-23 — Review pass
- intent_gap: 0
- bad_spec: 5: (high 2, medium 3, low 0)
- patch: 0
- defer: 0
- reject: 1: (high 0, medium 0, low 1)
- addressed_findings:
  - `[high]` `[bad_spec]` v3 active records could expose blank canonical provenance after an additive-only migration; require an atomic derived-index reset while retaining the Source Registry.
  - `[high]` `[bad_spec]` section boundaries and leading content were undefined; require lossless preamble plus non-overlapping structural heading records.
  - `[medium]` `[bad_spec]` Markdown/fence/line-ending and structural-ordinal semantics were underspecified; define the supported deterministic grammar and collision-safe identity.
  - `[medium]` `[bad_spec]` normalized file URIs and malformed/non-UTF-8 artifact diagnostics were underspecified; require encoded locators and reversible safe diagnostics.
  - `[medium]` `[bad_spec]` the test plan omitted upgrade, source-revision, native-project, memory-type, and diagnostic-lifecycle regressions; require explicit coverage.

### 2026-07-23 — Review pass
- intent_gap: 0
- bad_spec: 5: (high 2, medium 3, low 0)
- patch: 0
- defer: 0
- reject: 3: (high 0, medium 0, low 3)
- addressed_findings:
  - `[high]` `[bad_spec]` non-overlapping records conflicted with a same-or-shallower parent boundary; define next-heading-of-any-level ownership.
  - `[high]` `[bad_spec]` allowlist validation did not explicitly apply to resolved in-root symlink targets; require both lexical observation and resolved-role validation.
  - `[medium]` `[bad_spec]` malformed allowlisted artifacts had no explicit fail-before-commit rule; preserve the active generation and surface only safe failure data.
  - `[medium]` `[bad_spec]` record-level coverage and observed-at provenance required by the architecture were omitted; persist and test both fields.
  - `[medium]` `[bad_spec]` end-to-end persistence, actual non-UTF-8 enumeration, alias, and format-gate coverage were omitted; require focused tests and `cargo fmt --check`.

### 2026-07-23 — Review pass
- intent_gap: 0
- bad_spec: 9: (high 1, medium 8, low 0)
- patch: 0
- defer: 0
- reject: 0
- addressed_findings:
  - `[high]` `[bad_spec]` adjacent Setext underline handling could emit overlapping headings and panic; require consumed title/underline spans and a regression proving a safe non-overlapping parse.
  - `[medium]` `[bad_spec]` Setext detection did not exclude fence-consumed candidates and malformed backtick fence openers were undefined; define valid opener/closer behavior and regressions.
  - `[medium]` `[bad_spec]` canonical body fidelity did not explicitly retain trailing blank range content; preserve complete normalized section and preamble ranges.
  - `[medium]` `[bad_spec]` the SQLite test plan did not require exact persisted semantic identity, unit ID, or separate display locator values; add direct projection assertions.
  - `[medium]` `[bad_spec]` reconciliation coverage omitted the changed section's content-hash transition; assert it together with stable identity and source revision.
  - `[medium]` `[bad_spec]` diagnostic persistence coverage omitted its kind, lexical path, generation, and record exclusion; assert all persisted consumer fields.
  - `[medium]` `[bad_spec]` diagnostic-only rescans were not required to prove active-generation preservation; add that case.
  - `[medium]` `[bad_spec]` the non-UTF-8 fixture permitted skipped assertions after an external command failure; require stdlib creation and unconditional coverage.
  - `[medium]` `[bad_spec]` the parser did not state that invalid backtick fence openers are plain text; add exact grammar behavior so headings remain visible.

### 2026-07-23 — Review pass
- intent_gap: 0
- bad_spec: 4: (high 2, medium 2, low 0)
- patch: 2: (high 0, medium 2, low 0)
- defer: 1: (high 0, medium 1, low 0)
- reject: 1: (high 0, medium 0, low 1)
- addressed_findings:
  - `[high]` `[bad_spec]` resolved/metadata failures for observed allowlist names could be reclassified as unsupported or silently dropped; require a terminal safe enumeration failure unless a root escape or resolved-role mismatch is proven.
  - `[high]` `[bad_spec]` final validation did not bind committed canonical records to the exact bytes that were parsed; require a pre-CAS byte digest recheck even when path, size, and mtime agree.
  - `[medium]` `[bad_spec]` Setext marker indentation and whitespace-only preamble ownership were unspecified; bound marker indentation and preserve every non-empty leading range.
  - `[medium]` `[bad_spec]` parser failure status and transport behavior were not explicitly required; require safe persisted and HTTP-envelope assertions.
  - `[medium]` `[patch]` add exact assertion that malformed parsing records failed scan state and `parse_failed` error code.
  - `[medium]` `[patch]` exercise the existing scan HTTP error mapping for malformed source content without adding an endpoint.

### 2026-07-23 — Review pass
- intent_gap: 0
- bad_spec: 5: (high 3, medium 2, low 0)
- patch: 6: (high 0, medium 6, low 0)
- defer: 1: (high 0, medium 1, low 0)
- reject: 1: (high 0, medium 0, low 1)
- addressed_findings:
  - `[high]` `[bad_spec]` UTF-8 ATX title parsing was not constrained to char boundaries; require no panic and Unicode title regressions.
  - `[high]` `[bad_spec]` final digest validation did not state containment before or after the file read; require both checks before comparing bytes.
  - `[high]` `[bad_spec]` diagnostic observations were omitted from final validation; compare the complete artifact enumeration before activation.
  - `[medium]` `[bad_spec]` Setext title and underline grammar did not reject delimiter-only/mixed marker lines; define both conditions.
  - `[medium]` `[bad_spec]` recovery and reconciliation test requirements did not explicitly include stale diagnostics, diagnostic drift, exact type association, or unchanged sibling hashes.
  - `[medium]` `[patch]` add bare-CR normalization coverage.
  - `[medium]` `[patch]` add exact per-artifact provider-memory-type persistence assertions.
  - `[medium]` `[patch]` assert unchanged sibling content hash under a changed source revision.
  - `[medium]` `[patch]` cover diagnostic-only rescan active-generation preservation.
  - `[medium]` `[patch]` cover boot cleanup of stale diagnostic rows.
  - `[medium]` `[patch]` cover diagnostics changing between first and final enumeration.

### 2026-07-23 — Final review and repair pass
- intent_gap: 0
- bad_spec: 0
- patch: 12: (high 3, medium 8, low 1)
- defer: 0
- reject: 1: (high 0, medium 0, low 1)
- addressed_findings:
  - `[high]` repaired allowlist directories being silently omitted and bound opened file handles to the current root-contained file before and after reading.
  - `[medium]` completed Setext indentation grammar, exact persisted canonical identity/provenance, native-project/observed-at, diagnostic generation/lifecycle, native-byte, role-mismatch, and malformed-wire safety coverage.
  - `[low]` added the repository-held canonical Markdown parser fixture.
  - `[low]` excluded unrelated BMAD hook/configuration artifacts from this Story's commit.

## Verification

**Commands:**
- `cargo test --test codex_canonicalization` -- expected: parser boundary, identity/provenance, diagnostics, and capability contract tests pass.
- `cargo test --test scan_pipeline` -- expected: canonical staging preserves the 1.4 atomic-generation, recovery, and source-integrity guarantees.
- `cargo test` -- expected: all Rust unit and integration tests pass.
- `cargo clippy --all-targets -- -D warnings` -- expected: no warnings.
- `rustfmt --edition 2021 --config skip_children=true --check src/adapters/codex.rs src/application/scan.rs src/domain/mod.rs src/domain/ports/provider_adapter.rs src/domain/scan.rs src/http/mod.rs src/index/migrations.rs src/index/mod.rs src/index/scan_store.rs tests/codex_canonicalization.rs tests/http_api.rs tests/scan_pipeline.rs tests/source_registry.rs tests/fts5_available.rs` -- expected: Story-touched Rust files already match rustfmt output without reformatting unrelated baseline files.
- `npm run build` -- expected: existing browser client still type-checks and builds without requiring a Story 1.5 UI surface.

## Auto Run Result

Status: blocked

Blocking condition: no subagents — the required adversarial, edge-case, verification-gap, and intent-alignment review agents could not start because the collaboration account reported its usage limit.

Implemented and locally verified the Story 1.5 canonical Markdown parsing, source-boundary diagnostics, v4 provenance migration, and scan-pipeline integration. The following all passed before the review block: `cargo test --test codex_canonicalization` (4 tests), `cargo test --test scan_pipeline` (28 tests), `cargo test` (all suites), `cargo clippy --all-targets -- -D warnings`, and `npm run build`. The I/O matrix has passed coverage in `codex_canonicalization` and `scan_pipeline`, but the required independent review triage and final commit remain incomplete.

Residual artifacts: all implementation, test, fixture, generated Epic-context, and this Story spec changes remain uncommitted in the working tree so the review loop can resume without re-implementation.

### 2026-07-23 — Resumed completion

Status: done

Implementation revision: `6b0ed3c69af8c858cc3ac7f46478fb79e6f8d7d7`

Implemented canonical, allowlisted Codex Markdown records with versioned provenance, safe unsupported-artifact diagnostics, v4 derived-index reset, byte-digest reconciliation, and safe parse-error HTTP mapping. The final review repairs also close the descriptor/path race before body reads, reject uninspectable allowlist directories, and complete the persisted-boundary regression coverage.

Review result: four independent review layers found no intent or specification gap in the final contract; all actionable implementation and verification findings were repaired. Unrelated `.gitignore`, BMAD loop, hook, and local configuration artifacts remain out of this Story commit.

Verification passed: `cargo test` (all Rust unit, integration, and doc tests), `cargo clippy --all-targets -- -D warnings`, the Story-scoped `rustfmt --check`, `git diff --check 19b63da`, and `npm run build`.

Residual platform note: APFS may reject native non-UTF-8 names; the suite attempts the real standard-library filesystem case where supported and retains native-byte encoder coverage for APFS.
