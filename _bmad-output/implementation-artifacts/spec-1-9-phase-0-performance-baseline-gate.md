---
title: 'Story 1.9: Phase 0 Performance Baseline Gate'
type: 'feature'
created: '2026-07-24'
status: 'done'
baseline_revision: '4c29d89254446d458f4efbbafa7f7d4825259700'
final_revision: 'a0d8e37f5bba1c5bf89b9dd490e765a86db287fc'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-1-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-1-8-source-inventory-health-manual-rescan.md'
warnings: []
---

<intent-contract>

## Intent

**Problem:** The repository has an explicitly open Phase 0 benchmark manifest, so later changes have no trustworthy regression gate for cold scan, query latency, RSS, or Derived Index size.

**Approach:** Measure the four metrics on one fixed, Carver-owned anonymized Codex fixture, record the measured baselines and owner-approved thresholds in the root benchmark manifest, and enforce that manifest on the same fixture in the default test path.

## Boundaries & Constraints

**Always:** Preserve the original source files as read-only inputs. Use one fixed, versioned, anonymized fixture and a declared query set for every baseline and regression run. Record actual measurements with their unit and collection method. Keep the root `tests/benchmarks/memory-index.json` as the single authority; a limited search-recall experiment must not be presented as the four-metric gate.

**Block If:** The selected local inputs cannot be sanitised without retaining an identifier, secret, raw home path, or non-public URL; the four measurements cannot be collected reproducibly on the committed fixture; or a required metric does not have a positive observed value. In those cases, halt rather than weakening the privacy boundary or inventing a baseline.

**Never:** Do not substitute the generic E2E fixture for Carver's real anonymized sample, invent baseline values or thresholds, enable an unenforceable gate, upload fixture content, log memory/query text, or claim an FTS5 index-size measurement before FTS5 is the actual query index.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|---------------------------|----------------|
| Baseline establishment | Approved fixed anonymized fixture and threshold policy | Root manifest records cold scan, query P50/P95, process RSS, and actual Derived Index size with a reproducible fixture identifier | No error expected |
| Regression gate | Same fixture with complete numeric baseline/threshold fields | Test reports measured values and fails when a metric exceeds its threshold | Clear metric-specific failure without fixture content |
| Fixture unavailable | No approved real anonymized fixture | Manifest remains open with null metrics and enforcement disabled | Block story; do not write values |
| Incomplete manifest | Any required metric or threshold is null/invalid | Gate refuses to claim enforcement | Clear configuration failure |

</intent-contract>

## Code Map

- `tests/benchmarks/memory-index.json` -- existing authoritative placeholder for the four required metrics and gate policy.
- `server/tests/benchmarks/memory-index.json` and `server/tests/search.rs` -- existing limited recall/latency experiment that must be retired or made non-authoritative to avoid manifest drift.
- `server/src/application/scan.rs` and `server/src/application/query.rs` -- production cold-scan and query seams to measure without transport noise.
- `server/src/lib.rs` -- boot/database-path seam for isolated Derived Index creation and size measurement.
- `server/tests/performance_baseline.rs` -- planned benchmark harness and gate coverage.
- `tests/fixtures/benchmarks/codex-anonymized-v1/` -- planned fixed fixture location, pending owner-supplied anonymized data.

## Tasks & Acceptance

**Execution:**
- `tests/fixtures/benchmarks/codex-anonymized-v1/` -- create a fixed repository-safe fixture from the selected real local Codex rollout summaries (`2026-07-19T11-20-08-Lpkm-tessera_agent_memory_explorer_architecture_spec.md`, `2026-07-23T13-05-45-l515-agentguide_91ai_gap_analysis_and_main_push.md`, and `2026-07-08T05-21-29-DRms-91ai_discoverability_metadata_and_commit.md`), replacing paths, user names, URLs, email addresses, UUIDs, hashes, credential-shaped values, and other identifiers before any content is written. Add a fixture manifest that records only the sanitisation rules and non-sensitive query identifiers.
- `server/tests/performance_baseline.rs` -- measure cold scan, repeated query P50/P95, current-process RSS, and main Derived Index SQLite size through production scan/query paths; compare against the authoritative manifest in the default Rust suite.
- `tests/benchmarks/memory-index.json` -- write observed values and a documented policy of `2 ×` each measured baseline (rounded upward in the metric's unit) as the first regression threshold; enable the gate only after all four measurements are non-null and positive.
- `server/tests/benchmarks/memory-index.json` and `server/tests/search.rs` -- remove duplicate authority or clearly retain the recall experiment as a non-gating fixture test that reads no competing performance threshold.

**Acceptance Criteria:**
- Given the selected local Codex inputs, when the fixture is prepared, then it contains only sanitised text and a verifier rejects residual paths, credentials, URLs, emails, UUIDs, and hash-like identifiers before it is committed.
- Given the fixed anonymized Codex fixture, when the Phase 0 harness runs, then it writes/verifies measured cold scan, query P50/P95, RSS, and actual current-index size in the root manifest without exposing source text.
- Given the complete root manifest and the same fixture, when a later change exceeds a configured threshold, then the default quality gate fails with the affected metric and measured value.
- Given no approved fixture or incomplete metric policy, when the harness is evaluated, then the gate remains explicitly open and no baseline/threshold is fabricated.

## Design Notes

- The existing `tests/fixtures/e2e-codex-home` content explicitly identifies itself as an E2E search fixture; it is useful for mechanics but is not evidence of an anonymized real Carver sample.
- Current production search uses the existing Derived Index implementation; index-size reporting must state that fact rather than claim an FTS5-backed measurement not present in the query path.

## Spec Change Log

### 2026-07-24 — Resumed baseline policy
- Finding: Carver explicitly authorized resolving the missing-fixture block, but the original plan did not define a reproducible tolerance policy or fixture preparation boundary.
- Amendment: lock a repository-safe real anonymized fixture, five fresh-database cold-scan trials with a median statistic, an isolated RSS probe, and an exact `2 ×` threshold policy.
- Avoids: synthetic data, privacy leakage, non-reproducible samples, and arbitrary gate thresholds.
- KEEP: source files remain read-only; no raw fixture text, paths, URLs, credentials, or query text is logged.

## Review Triage Log

### 2026-07-24 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 7: (high 5, medium 2, low 0)
- defer: 0
- reject: 2: (low 2)
- addressed_findings:
  - `[high]` `[patch]` Allowed and privacy-scanned every expected fixture artifact, rejecting symlinks and unexpected files.
  - `[high]` `[patch]` Added whitespace-tolerant and credential-prefix detection with corpus-independent verifier regressions.
  - `[high]` `[patch]` Pinned allowed fixture content and digest identity before measurement.
  - `[medium]` `[patch]` Measured five fresh-database cold scans and used the documented median statistic.
  - `[medium]` `[patch]` Corrected fractional-millisecond rounding so measured values cannot be understated.
  - `[high]` `[patch]` Moved RSS collection to an isolated Tessera child-process probe.
  - `[high]` `[patch]` Enforced exact overflow-safe `2 ×` thresholds and P50/P95 invariants before enabling the gate.

## Verification

**Commands:**
- `cargo test --test performance_baseline` -- expected: all four measured metrics and gate decisions are exercised against the approved fixture.
- `cargo test` -- expected: the default Rust suite includes the completed threshold gate.
- `cargo clippy --all-targets -- -D warnings` -- expected: benchmark harness and dependencies are lint-clean.
- `jq empty tests/benchmarks/memory-index.json tests/fixtures/benchmarks/codex-anonymized-v1/fixture-manifest.json` -- expected: authoritative manifests are valid JSON.
- `git diff --check` -- expected: no whitespace errors.

## Resume Authorization

On 2026-07-24 Carver explicitly instructed the workflow to resolve the block and continue. The selected real local input corpus and the data-derived `2 ×` tolerance policy above are authorised only for this repository-safe, sanitised Phase 0 fixture.

## Auto Run Result

Status: blocked
Blocking condition: missing approved anonymized Carver Codex fixture and Phase 0 threshold policy.

Evidence:
- `tests/benchmarks/memory-index.json` remains at version `0` with all four baselines/thresholds `null` and `gate.enforce: false`.
- The only committed E2E fixture identifies itself as a generic search fixture, and no benchmark fixture/query set establishes it as Carver-owned real anonymized data.
- The separate `server/tests/benchmarks/memory-index.json` measures only recall and single-query latency, not the Story's four required baseline metrics.

No production or test implementation was started, and no thresholds were fabricated.

### Resumed Run — 2026-07-24

Status: done

Summary:
- Created and pinned a repository-safe anonymized fixture from the authorized local Codex corpus.
- Added a default Rust gate for cold-scan median, query P50/P95, isolated-process RSS, and current Derived Index size.
- Replaced the open placeholder policy with observed baselines and exact `2 ×` thresholds; the gate is now enforced.

Review findings:
- Patched: 7 (high 5, medium 2, low 0); deferred: 0; rejected: 2 (low).
- Follow-up review recommendation: true (high-severity privacy and gate-integrity patches).

Verification performed:
- `cargo test --test performance_baseline -- --nocapture` — passed, 8 tests; 1 probe-only test ignored by direct runner and invoked by the gate child process.
- `cargo test` — passed, including the default performance gate.
- `cargo clippy --all-targets -- -D warnings` — passed.
- `jq empty tests/benchmarks/memory-index.json tests/fixtures/benchmarks/codex-anonymized-v1/fixture-manifest.json` — passed.
- `git diff --check` — passed.

Residual risks:
- The isolated RSS probe currently depends on POSIX `ps` output, which is verified on this macOS runner; a future non-POSIX target needs an equivalent probe.
- A 1 ms query baseline yields a 2 ms threshold under the approved policy, so a heavily loaded CI host can fail the gate truthfully.
