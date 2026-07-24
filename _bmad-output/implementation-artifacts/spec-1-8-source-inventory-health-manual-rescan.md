---
title: 'Story 1.8: Source Inventory, Health, and Manual Rescan'
type: 'feature'
created: '2026-07-24'
status: 'done'
baseline_revision: '5dc868d'
final_revision: '15e572338889a4609b734e2d4c078617141e49e7'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-1-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-1-7-open-original-location.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-1-6-keyword-search-provenance.md'
warnings: []
---

<intent-contract>

## Intent

**Problem:** Confirmed Codex Sources can be scanned, but the browser cannot assess the scope, freshness, health, or last safe failure of each source. The synchronous scan endpoint also has no observable, cancellable rescan lifecycle.

**Approach:** Add a server-derived Inventory view for each registered Source, then expose a per-confirmed-Source rescan job with versioned, ordered SSE progress and cancellation. Keep the Rust core as the sole owner of source, index, and host filesystem authority.

## Boundaries & Constraints

**Always:** Inventory facts come from Source Registry and scan/index state, never browser guesses. Display a complete record count only for `full` coverage; all other coverage levels must state their limitation. Preserve the confirmed relationship when health changes. The browser submits only `source_id`; no paths, SQL, generation identifiers, raw OS errors, or content cross the wire. Every SSE event carries `api_version` and a monotonically increasing sequence for its rescan. Cancellation must prevent a cancelled or superseded scan owner from activating a generation, while previous successful search/open results remain available.

**Block If:** Correct cancellation cannot be made race-safe with the persistent scan fencing/CAS protocol, or the loopback HTTP implementation cannot keep the SSE stream and scan work from holding the same database mutex for the whole job.

**Never:** Do not rescan all Sources from one UI action, add automatic watching/retry scheduling, change search ranking or canonical parsing rules, downgrade a failed Source by deleting it, expose raw error details, or represent non-full coverage as a complete synchronized inventory.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|---------------------------|----------------|
| Inventory read | Confirmed Source with an active generation | Versioned Inventory item shows provider, root, native project, coverage, health, last successful scan, complete record count, and safe latest error | No error expected |
| Limited coverage | `search_only`, `existence_only`, or `unsupported` Source | Inventory shows coverage limitation; complete count is absent, never `0` as a substitute | No error expected |
| Rescan success | Browser starts a rescan for one confirmed Source | Ordered versioned SSE lifecycle ends in success; Inventory becomes healthy and reports the new active count | No error expected |
| Rescan cancellation or failure | Cancel request, invalid root, denied read, unsupported format, or scan error | Stream terminates safely; no partial generation activates; Inventory classifies health and exposes only a safe reason | Versioned safe error/event; prior active generation remains searchable |
| Invalid rescan request | Unknown or disabled/rejected `source_id` | No job starts and no filesystem access occurs | Safe `source_not_found` or not-confirmed error |

</intent-contract>

## Code Map

- `server/src/domain/source.rs` and `server/src/index/source_registry.rs` -- extend persisted health states and controlled updates without changing lifecycle identity.
- `server/src/domain/scan.rs`, `server/src/index/scan_store.rs`, and `server/src/application/scan.rs` -- define Inventory DTOs, safe health classification, scan-job progress/cancellation, and fenced active-generation behavior.
- `server/src/http/mod.rs` and `server/src/http/server.rs` -- expose the versioned Inventory, rescan start/cancel, and SSE surfaces while retaining loopback and error-envelope guarantees.
- `src/api/sources.ts`, `src/api/scan.ts`, and `src/api/errors.ts` -- validate Inventory and scan/SSE wire payloads and preserve safe error messages.
- `src/features/sources/Sources.tsx` -- render the Source Inventory facts and accessible per-Source Rescan/Cancel progress controls.
- `server/tests/inventory.rs`, `server/tests/scan_pipeline.rs`, `server/tests/http_api.rs`, and `tests/ui/accessibility.spec.ts` -- cover aggregation, health/cancel fencing, HTTP/SSE safety, and keyboard-visible UI behavior.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/source.rs` and `server/src/index/source_registry.rs` -- add `unknown`, `healthy`, `degraded`, and `error` persistence/validation plus an explicit health-update path; reject unknown stored values rather than silently treating corruption as `unknown`.
- `server/src/domain/scan.rs`, `server/src/index/scan_store.rs`, and `server/src/application/scan.rs` -- add a Source Inventory read model that separately obtains the last successful completion, active-generation count, and most recent safe failure; map path, permission, and unsupported-format conditions to `degraded`, scan failures to `error`, and successful scans to `healthy`. Introduce cancellable per-Source job ownership/progress that remains compatible with persistent fencing and CAS activation.
- `server/src/http/mod.rs` and `server/src/http/server.rs` -- add a versioned Inventory endpoint plus one-source rescan start, ordered SSE progress, and cancel endpoints. Validate `source_id`, retain Host/Origin protections, and redact source text, raw filesystem paths, credentials, and OS error strings.
- `src/api/sources.ts`, `src/api/scan.ts`, and `src/api/errors.ts` -- add strict runtime guards and clients for the new Inventory, progress, cancel, and safe-error contracts; reject malformed or out-of-order events.
- `src/features/sources/Sources.tsx` -- replace the generic scan presentation with an Inventory card showing native project, coverage, health, last successful scan, count eligibility/value, and safe latest error; provide keyboard-reachable Rescan and Cancel controls with ordered `aria-live` progress.
- `server/tests/inventory.rs`, `server/tests/scan_pipeline.rs`, `server/tests/http_api.rs`, and `tests/ui/accessibility.spec.ts` -- prove complete-vs-limited counts, health reason classification without leaked details, one-source isolation, cancellation/cas non-activation, versioned ordered SSE, malformed/unauthorized rescan rejection, and keyboard operation/feedback.

**Acceptance Criteria:**
- Given a confirmed Codex Source, when Carver views Inventory, then its provider, root, native project, coverage, health, last successful scan, count eligibility/value, and safe latest error are rendered from server facts; a health change does not remove its confirmation.
- Given a Source without full coverage, when Inventory renders it, then the UI does not label any count as a complete synchronization and states the declared coverage limitation.
- Given Carver activates Rescan on a confirmed Source, when the job runs, then only that `source_id` is scanned, the browser receives versioned strictly increasing SSE progress, and keyboard cancellation is available.
- Given a rescan is cancelled or fails, when its owner attempts to finish, then its staged data cannot become active, existing successful search/open results remain available, and Inventory shows a safe health classification without body, credential, or raw-path leakage.

## Spec Change Log

### 2026-07-24 — Verification configuration correction
- Finding: the planned browser command referenced a non-existent implementation-artifact config file.
- Amendment: use the committed `playwright.story-1-8.config.ts` isolated-port configuration.
- Avoids: reporting an unexecutable verification command as completed.
- KEEP: retain an isolated browser port so unrelated local services are never terminated.

## Review Triage Log

### 2026-07-24 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 9: (high 3, medium 5, low 1)
- defer: 0
- reject: 0
- addressed_findings:
  - `[high]` `[patch]` Reserved a durable rescan run before returning the queued response so immediate cancellation fences pending work.
  - `[high]` `[patch]` Claimed one active job per Source and scoped ordered events to `job_id` to prevent duplicate workers and interleaved progress.
  - `[high]` `[patch]` Preserved the durable cancelled state through scan transitions and prevented cancelled work from activating a generation.
  - `[medium]` `[patch]` Added lifecycle to Inventory and restricted rescan controls to confirmed Sources.
  - `[medium]` `[patch]` Bounded per-job event history and added `after_sequence` progress reads.
  - `[medium]` `[patch]` Kept a committed rescan truthful when a post-commit health update cannot be persisted.
  - `[medium]` `[patch]` Preserved a safe Inventory failure reason for root validation failures without erasing prior success facts.
  - `[medium]` `[patch]` Added HTTP, storage, and browser coverage for ordered versioned events, immediate cancellation, terminal refresh, and retained search/open behavior.
  - `[low]` `[patch]` Corrected the browser verification command to a committed isolated-port configuration.

## Design Notes

- Health is a durable Source fact because search and open already consume `source_registry.health_state`; it cannot be an Inventory-only UI calculation.
- “Latest scan” and “last successful scan” are different facts. The Inventory query must not make a recent failed/cancelled run erase the last successful time or active record count.
- SSE is transport-only observation: scan authorization, state transitions, fencing, cancellation, and the final activation decision stay in the Rust application/index boundary.

## Verification

**Commands:**
- `cargo test --test inventory` -- expected: Inventory aggregation, count eligibility, and safe health classification pass.
- `cargo test --test scan_pipeline` -- expected: cancellation and per-Source fencing preserve prior active generations and source isolation.
- `cargo test --test http_api` -- expected: Inventory, rescan, cancel, and versioned ordered SSE contracts pass without leakage.
- `cargo test` -- expected: all Rust suites pass.
- `npm run build` -- expected: TypeScript guards and Inventory UI build successfully.
- `npx playwright test -c playwright.story-1-8.config.ts tests/ui/accessibility.spec.ts` -- expected: keyboard rescan/cancel and accessible progress/state rendering pass.
- `git diff --check` -- expected: no whitespace errors.

## Auto Run Result

Status: done

Summary:
- Added server-derived Source Inventory facts, durable Source health, and scoped manual rescan jobs with ordered versioned SSE progress and cancellation.
- Preserved prior active generations and user-visible search/open availability through failed or cancelled rescans.

Files changed:
- Server domain, index, application, HTTP, and runtime modules implement health, inventory aggregation, cancellable fenced jobs, and SSE transport.
- Frontend API and Sources UI modules validate the contract and render accessible Inventory, Rescan, Cancel, and safe status feedback.
- Rust HTTP/index/scan tests and Playwright accessibility tests cover Inventory truthfulness, cancellation, scoped progress, and browser controls.
- `playwright.story-1-8.config.ts` supplies a repeatable isolated-port browser test configuration because the default test port is occupied by an unrelated local service.

Review findings:
- Patched: 9 (high 3, medium 5, low 1); deferred: 0; rejected: 0.
- Follow-up review recommendation: true (patched score 16; high-severity patch findings require a fresh post-commit review pass).

Verification performed:
- `cargo test --test inventory` — passed, 7 tests.
- `cargo test --test scan_pipeline` — passed, 37 tests.
- `cargo test --test http_api` — passed, 12 tests.
- `cargo test` — passed, all Rust suites.
- `cargo clippy --all-targets -- -D warnings` — passed.
- `npm run build` — passed.
- `npx playwright test -c playwright.story-1-8.config.ts tests/ui/accessibility.spec.ts` — passed, 2 tests on isolated loopback port 1422.
- `git diff --check` — passed.

Residual risks:
- Rescan job/event state is process-local apart from the durable scan run; an application restart ends in-flight progress observation and requires a fresh manual rescan.
