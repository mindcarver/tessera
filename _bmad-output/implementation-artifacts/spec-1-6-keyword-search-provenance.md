---
title: 'Story 1.6: Keyword Search and Provenance Results'
type: 'feature'
created: '2026-07-24'
status: 'done'
baseline_revision: 'fd9892c'
review_loop_iteration: 1
followup_review_recommended: false
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-1-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-1-5-codex-memory-parsing-boundary-canonical-records.md'
  - '{project-root}/docs/phase-0-verification.md'
warnings: [oversized]
---

<intent-contract>

## Intent

**Problem:** The active, canonical Codex index has no read-side query contract or UI, so Carver cannot find a memory section or verify where a result came from. The existing FTS5 conclusion also makes a bare default-tokenizer implementation untrustworthy for Chinese short queries.

**Approach:** Add a bounded, versioned keyword-search path over only active generations of confirmed Sources, return safe canonical snippets and complete stored provenance, then render it in the loopback UI with truthful empty states and keyboard operation.

## Boundaries & Constraints

**Always:** Rust remains the sole SQLite/filesystem boundary; every request reads only the current `memory_records` belonging to confirmed Sources' active generations and never reads source files. Use parameterized SQLite queries and treat user text as literal terms, not FTS syntax. Preserve Source `native_project`, coverage, and persisted health exactly. Results use stable `record_id ASC` ordering with a bounded cursor containing only the normalized query, current-index revision, and last record id; every continuation recomputes the current eligible scope. If the current-index revision changed, return safe `cursor_stale` and require a fresh search. Each result carries `record_id`, a plain-text excerpt from stored title/body, Provider, `source_id`, native project, semantic and display locators, observed-at as the honest last-observed/update time, coverage level, and stored Source Health. Never log or put the query into an error envelope.

**Block If:** The Chinese fixture measurement required by `docs/phase-0-verification.md` cannot demonstrate non-zero recall for both a two-character and a three-character query while preserving the three empty states, or the selected tokenizer/search form requires an architecture decision or a dependency not already approved. Record the measurements and halt for an AD/stack decision rather than claiming the search is complete.

**Never:** Do not query inactive/staging generations, disabled/rejected Sources, JSONL/transcripts, the filesystem, an external model/service, or an arbitrary path. Do not add writeback, browser filesystem access, raw Markdown/HTML rendering, ranking claims that are not implemented, Source Health mutation, inventory/scan features reserved for Story 1.8, or a second scan/write path.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Match and page | Confirmed Sources with active canonical generations; non-empty literal query | Versioned `SearchPage` with a stable, cursor-resumable page of provenance-complete result cards and no duplicate record across pages | No error expected |
| No match | At least one confirmed Source has an active generation, but no active record matches | `empty_state=no_match`, zero results, no invented availability diagnosis | No error expected |
| Not indexed | Confirmed Source has no active generation and no failed latest run | `empty_state=source_not_indexed`, zero results | No error expected |
| Unavailable | Confirmed Source has no active generation and its latest persisted scan state is failed/retry | `empty_state=source_unavailable`, zero results; stored health remains unchanged | No error expected |
| Invalid request | Blank/whitespace query, invalid cursor, non-positive or over-maximum limit, or malformed wire JSON | Stable safe `bad_request` envelope; no query text reflected | HTTP 400 |
| FTS syntax / markup | Punctuation, quotes, FTS operators, or Markdown/HTML in the query or matched body | Literal parameterized search; excerpt is rendered as React text, never HTML | No error expected |

</intent-contract>

## Code Map

- `server/src/domain/ports/query_store.rs` -- replace the placeholder with the bounded search read-port and DTO vocabulary.
- `server/src/domain/query.rs` and `server/src/application/query.rs` -- query request/page/empty-state domain types and confirmed-active-source orchestration.
- `server/src/index/scan_store.rs` -- add the concrete, active-generation SQLite query implementation without a second derived projection.
- `server/src/http/mod.rs` and `server/src/http/server.rs` -- expose the versioned `GET /api/search` contract, strict query parsing, and safe status mapping.
- `server/tests/search.rs`, `server/tests/http_api.rs`, and `tests/ui/accessibility.spec.ts` -- prove current-active scoping, cursor/query binding and stale-revision invalidation, literal query safety, provenance, empty states, real-fixture short-CJK recall, populated-wire serialization, and browser behavior.
- `src/api/search.ts`, `src/api/errors.ts`, `src/features/search/Search.tsx`, and `src/App.tsx` -- validate the search wire shape, render the form/results/empty states, and expose errors and provenance accessibly.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/ports/query_store.rs`, `server/src/domain/query.rs`, and `server/src/application/query.rs` -- define validated, non-public-field `SearchRequest`/`SearchPage`/`SearchResult`/`SearchEmptyState` and orchestrate confirmed Sources plus active-generation and latest-scan facts. Reject blank input, invalid cursors, `limit` outside 1–100, and query strings longer than 1024 UTF-8 bytes before storage.
- `server/src/index/scan_store.rs` -- search only committed active generations with parameterized literal substring matching (the deliberate two-/three-character CJK strategy), never create an unused FTS copy, preserve all provenance fields, and remove superseded generations when a new generation is activated. Return an opaque, versioned cursor that binds the normalized query, current-index revision, and last sort key; continuation pages recompute the current confirmed/active scope and reject a changed revision with `cursor_stale`.
- `server/src/http/mod.rs` and `server/src/http/server.rs` -- map read-side outcomes to a versioned API response and parse only `q`, `cursor`, and `limit` on `GET /api/search`; map invalid input to 400, `cursor_stale` to a safe conflict response, and unexpected storage errors to the existing safe envelope.
- `server/tests/search.rs` and `server/tests/http_api.rs` -- cover every matrix row, including active-index revision change between pages, cross-query cursor reuse, confirmed-only scope, provenance fidelity, literal operator injection, no query/body/path leakage, and a populated HTTP page plus continuation whose serialized fields satisfy the TS client contract. Run the real local Codex fixture via a committed, data-free benchmark harness: choose CJK two-/three-character queries from the indexed corpus without printing source text, assert non-zero recall, record aggregate recall/empty-result/latency measurements in `server/tests/benchmarks/memory-index.json`, and leave all Story 1.9 thresholds null.
- `src/api/search.ts`, `src/api/errors.ts`, `src/features/search/Search.tsx`, `src/App.tsx`, and `tests/ui/accessibility.spec.ts` -- add strict runtime validation and a semantic Search region with labelled input, submit button, live result/empty-state announcement, keyboard-visible result cards, text-only excerpts, complete provenance, pagination control, and user-safe failure rendering. The UI must reset accumulated results/cursor on any query edit and discard stale async completions; a stale cursor keeps existing results visible and tells the user to run the search again. Browser coverage must exercise keyboard submit, result/provenance rendering, empty-state copy, Load more, and API-contract error rendering.

**Acceptance Criteria:**
- Given active indexed records in one or more confirmed Codex Sources, when Carver submits a keyword, then `/api/search` returns a versioned, stable cursor page whose every card contains the stored excerpt and all required provenance, and the UI renders it without executing source content.
- Given a page boundary, when Carver follows its cursor without an index revision change, then the next page is deterministic, contains no prior record, and remains bound to the original normalized query. Given a newly activated scan generation, the continuation returns safe `cursor_stale` and the UI tells Carver to run the search again; a cursor from any other query is rejected.
- Given no matching active record, no active index, or a failed/retry latest scan without an active index, when Carver searches, then the UI displays respectively no-match, not-indexed, or unavailable without changing or inventing Source Health.
- Given the local real Codex fixture's generated two-character and three-character queries, when the selected search strategy is exercised, then each has non-zero recall; aggregate recall, empty-result rate, and latency are recorded without source text or a fabricated threshold; and FTS syntax-like user input remains literal and parameterized.
- Given a malformed request, a contract-shape drift, or a storage failure, when the API/UI handles it, then it emits the existing safe structured error behavior without query terms, record body, credentials, or source paths in error text.

## Design Notes

- `observed_at` is a scan observation timestamp, not a guessed source-file modification time. It is the only stored update-time signal available to this Story and must be labelled accordingly in the UI.
- Empty state derives only from persisted registry/active-generation/latest-run facts. It is separate from `health_state`, which remains the Source-owned field and is evolved in Story 1.8.
- Literal SQLite substring matching is the selected short-CJK strategy. It is parameterized and read-only; performance thresholds remain Story 1.9 work, so this Story records measurements but does not invent a pass/fail latency threshold.
- This is a local, single-user loopback tool. A cursor is pagination state, not an authorization credential: it never contains a source or generation selector, and every request derives eligibility from the current registry and active-generation marker.

## Spec Change Log

### 2026-07-24 — review repair loop 1

Independent review found that the initial execution plan did not make cursor scope stable across active-generation changes, did not bound request construction end to end, asked for an FTS projection with no reader, and treated synthetic CJK data as if it were the required real-fixture evidence. The plan now requires validated opaque snapshot cursors, bounded input, literal CJK substring search without an unused projection, a data-free real-fixture measurement harness with null performance thresholds, populated HTTP coverage, and browser-visible interaction coverage. This avoids skipped/mixed pages, loopback allocation abuse, a divergent unused index, fabricated benchmark claims, and a green backend with an unusable UI. **KEEP:** retain the loopback-only versioned API, confirmed/active-generation boundary, stored provenance, literal query safety, truthful empty states, and text-only rendering.

### 2026-07-24 — local cursor simplification

The product decision is explicit: Tessera is a local, single-user loopback reader. **CHANGE:** replace retained historical-generation snapshots and client-selected source/generation state with a cursor containing only normalized query, current-index revision, and last record id. The server recomputes the confirmed/current-active SQL scope for every page and returns `cursor_stale` after an index revision change. Superseded generation records are deleted on activation. **KEEP:** confirmed/current-active read boundary, query binding, deterministic ordering, safe literal matching, and provenance. This removes unnecessary retention and makes a modified cursor incapable of selecting an inactive or disabled source.

## Review Triage Log

### 2026-07-24 — Review pass
- intent_gap: 0
- bad_spec: 6: (high 3, medium 3, low 0)
- patch: 0
- defer: 0
- reject: 0
- addressed_findings:
  - `[high]` `[bad_spec]` define an opaque cursor that binds query and the full active-generation snapshot, preventing generation mixing, cross-query reuse, and stale UI pagination.
  - `[high]` `[bad_spec]` replace synthetic CJK evidence and fabricated latency threshold with a real local-fixture aggregate measurement gate and null thresholds.
  - `[high]` `[bad_spec]` require outer-surface browser coverage for submit, result/provenance, empty/error, and pagination behavior.
  - `[medium]` `[bad_spec]` make input validation non-bypassable and impose a 1024-byte request bound before decoding/storage.
  - `[medium]` `[bad_spec]` add a populated search HTTP wire contract and continuation coverage rather than testing only an empty response.
  - `[medium]` `[bad_spec]` remove the unconsumed FTS projection from the implementation plan; literal substring search is the selected CJK strategy.

### 2026-07-24 — Review pass
- intent_gap: 1: (high 1, medium 0, low 0)
- bad_spec: 0
- patch: 0
- defer: 0
- reject: 0
- addressed_findings:
  - none

## Auto Run Result

Status: resumed

The earlier cursor trust-model gap is resolved by the explicit local-only cursor decision above. Historical snapshot retention and cursor-selected source/generation state are removed; the continuation scope is always derived server-side from current confirmed active Sources. Remaining implementation work includes stale-revision handling, regression coverage, and truthful benchmark evidence.

Status: complete

Implemented the local cursor model and independently verified the completed Story. Current confirmed/active scope is recomputed per page, index changes return `cursor_stale`, superseded generation records are removed, and the UI preserves visible results while asking for a fresh search. Regression coverage includes disabled/rejected-source exclusion, oversized encoded query rejection before decoding, stale-cursor UI behavior, and all three UI empty states.

## Verification

**Commands:**
- `cargo test --test search` -- expected: read-side query, FTS, provenance, pagination, empty-state, and short-CJK fixture contracts pass.
- `cargo test --test http_api` -- expected: search route, validation, and safe wire envelope coverage pass.
- `cargo test` -- expected: complete Rust suite passes without scan/regression failures.
- `cargo clippy --all-targets -- -D warnings` -- expected: no warnings.
- `npm run build` -- expected: typed UI search contract and production bundle build.
- `git diff --check` -- expected: no whitespace errors.
