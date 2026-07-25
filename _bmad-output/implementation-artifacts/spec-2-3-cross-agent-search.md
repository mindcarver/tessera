---
title: 'Story 2.3: Cross-provider keyword search & provenance comparison'
type: 'feature'
created: '2026-07-25'
status: 'done'
baseline_revision: '15e2f20'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-2-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-2-2-claude-parse-index.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** After 2.2, a search already fans out across Codex + Claude Code and returns a `provider` tag and full provenance per result — but the results are ordered by `record_id` (not relevance), there is no way to see "this source was unavailable while others answered" (the only unavailability signal is an all-or-nothing `empty_state` that fires when the whole index is empty), and the UI renders a flat list with no provider grouping or comparison affordance.

**Approach:** (1) Add a relevance sort key to the search query (title-match precedence, then recency, then coverage — a SQL `ORDER BY`, no new schema); (2) add a per-source status sidecar to `SearchPage` so each confirmed source is reported `available`/`degraded`/`unavailable` for that query (a down source is flagged while its already-indexed records still return and other sources answer normally — the FR-14 prototype); (3) in the UI, group/badge results by provider and surface a partial-unavailability banner. No external models.

## Boundaries & Constraints

**Always:**
- A keyword search defaults to ALL confirmed sources with an active generation (already the case); results are ordered by a defined relevance key: **title-match first, then most-recently-observed, then `coverage_level='full'`, then `record_id` as a stable tiebreak** — expressed as a SQL `ORDER BY`, computed from columns already selected.
- Each result card shows `provider` + full provenance (already on the wire; the UI must render provider distinctly so Codex vs Claude Code cards are comparable at a glance).
- `SearchPage` carries a **source-status sidecar**: for every confirmed source, `{ source_id, provider, native_project, status }` where `status ∈ {available, degraded, unavailable}`, derived from `health_state` + whether an active generation exists (+ `latest_run` state). A source that is `degraded`/`error` or has no active generation is flagged, but its existing active records (if any) are **not suppressed** — the flag is informational.
- A single unavailable source never fails the whole query: its flag appears in the sidecar while other sources' results return normally (FR-14 prototype).
- Queries stay local (no external model / remote search — NFR-2); literal-substring matching and 2–3 character CJK recall are preserved (no tokenizer change).

**Block If:** The "ranked by relevance" AC is read as requiring FTS5/`bm25` true relevance. This spec deliberately uses a SQL heuristic sort (no schema change) and scopes FTS5 to a later search-quality story — FTS5 is a migration that risks the pinned CJK-recall contract. If a true `bm25` ranking is required to satisfy the AC, stop and re-plan rather than introducing a virtual table under this story.

**Never:**
- Introduce an FTS5 virtual table / `bm25` / a tokenizer change under this story (schema-migration-grade; CJK-recall risk; separate story).
- Suppress a down source's already-indexed records from results (the sidecar flags; it does not hide).
- Call any external model or remote search service.
- Change the global `current_index_revision` cursor model (per-source revision splitting is out of scope; any rescan still invalidates cursors as today).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Both providers indexed, shared term | Codex + Claude sources confirmed+indexed, query matches both | Both providers' records return; each card tagged with its `provider`; ordered by relevance key | No error |
| Relevance ordering | title-match record vs body-only record vs older record | title-match sorts first; among same match-tier, more recent first; stable `record_id` tiebreak | No error |
| One source unavailable | source A healthy+indexed, source B `error`/no active gen, query matches A | A's records return; sidecar marks A `available`, B `unavailable`; B does not break the query | No error |
| Degraded source keeps records | source `degraded` with prior active records | Its records still return; sidecar marks it `degraded` (not hidden) | No error |
| All sources unavailable | no confirmed source has an active generation, ≥1 latest run Failed | zero results + `empty_state = source_unavailable` (existing path preserved) | No error |
| Nothing indexed | confirmed sources, none scanned | zero results + `empty_state = source_not_indexed` | No error |
| Genuine no-match | indexed, query matches nothing | zero results + `empty_state = no_match` | No error |
| CJK recall preserved | 2–3 character CJK query | still matches (literal-substring path unchanged) | No error |
| Stale cursor | a rescan activated a new generation mid-pagination | 409 `cursor_stale` (existing behavior) | Structured error |

</intent-contract>

## Code Map

- `server/src/index/scan_store.rs` — `search_records`: change `ORDER BY m.record_id` to the relevance key (title-match precedence, `observed_at` recency, `coverage_level`, `record_id` tiebreak); keep the literal-substring `instr` match and CJK recall.
- `server/src/application/query.rs` — compute the per-source status sidecar for every confirmed source (`available`/`degraded`/`unavailable` from `health_state` + active-generation presence + `latest_run`); attach to `SearchPage`.
- `server/src/domain/query.rs` — add `SourceQueryStatus { source_id, provider, native_project, status }` and `SearchPage.sources: Vec<SourceQueryStatus>`; keep `results`/`next_cursor`/`empty_state`.
- `server/src/http/mod.rs` — serialize the new sidecar in the `/api/search` envelope (versioned DTO, snake_case).
- `src/api/search.ts` — mirror `SourceQueryStatus` + `SearchPage.sources`; runtime shape guards.
- `src/features/search/Search.tsx` — group/badge results by provider (visual distinction for comparison); render a partial-unavailability banner from the sidecar; keep the three empty states.
- `server/tests/search.rs`, `server/tests/http_api.rs`, `tests/ui/accessibility.spec.ts` — multi-provider search fixture + relevance-ordering + FR-14 mixed-availability (one down, one up) + sidecar assertions.

## Tasks & Acceptance

**Execution:**
- `server/src/index/scan_store.rs` -- replace the `ORDER BY m.record_id ASC` in `search_records` with the relevance key (title-match via `instr(m.title, ?)` first, then `m.observed_at DESC`, then `coverage_level='full'` DESC, then `m.record_id ASC` tiebreak) -- satisfy "ranked by relevance" without a schema change, preserving literal-substring match + CJK recall.
- `server/src/domain/query.rs` -- add `SourceQueryStatus { source_id, provider, native_project, status }` (`status` enum `Available|Degraded|Unavailable`, snake_case) and `SearchPage.sources: Vec<SourceQueryStatus>` -- the FR-14 per-query sidecar on the wire.
- `server/src/application/query.rs` -- build the sidecar: for each confirmed source derive `status` from `health_state` + active-generation presence + `latest_run.state`; attach to every `SearchPage` (not only the empty case) -- one down source is reported while others answer.
- `server/src/http/mod.rs` -- serialize `SearchPage.sources` in the `/api/search` envelope; keep `empty_state` for the all-empty case.
- `src/api/search.ts` -- mirror `SourceQueryStatus` + `sources` with runtime guards; extend the `SearchPage` type.
- `src/features/search/Search.tsx` -- render a provider badge/heading per result (or group by provider) so Codex vs Claude Code cards are visually comparable; add a partial-unavailability banner when the sidecar has any non-`available` source; keep the three empty-state copies and keyboard reachability.
- `server/tests/search.rs` -- add a multi-provider fixture (Codex + Claude rows sharing a term): assert both `provider` tags return, the relevance order, and the sidecar marks both `available`; add an FR-14 case (one confirmed source Failed/no-active-gen, one healthy) asserting the healthy source's results return and the sidecar flags the failed one.
- `server/tests/http_api.rs` -- assert the `/api/search` envelope carries `sources` with correct statuses for the multi-provider + mixed-availability cases.
- `tests/ui/accessibility.spec.ts` -- mock a multi-provider result list (Codex + Claude) and assert provider badges + a partial-unavailability banner render; keep the existing single-result and empty-state coverage.

**Acceptance Criteria:**
- Given Codex and Claude Code are both confirmed and indexed with a shared keyword, when Carver searches, then results from both providers return, each card is labeled with its `provider` and full provenance, and the two providers' memories are visually comparable.
- Given multiple matching results, when rendered, then they are ordered by the relevance key (title-match before body-only; more recent before older); a body-only result never outranks a title-match result for the same query.
- Given one confirmed source is unavailable (`error` or no active generation) while another is healthy and matches, when Carver searches, then the healthy source's results return normally and the unavailable source is flagged in the response sidecar — the query does not fail.
- Given all confirmed sources are unavailable / nothing is indexed / a genuine no-match, when Carver searches, then the correct `empty_state` (`source_unavailable` / `source_not_indexed` / `no_match`) is shown.
- Given a 2–3 character CJK query against indexed content, when searched, then it still matches (literal-substring recall preserved; no tokenizer introduced).
- Given any query, then no external model or remote search service is invoked (local only).
- Given the search UI, then provider badges/grouping and the partial-unavailability banner are keyboard-reachable and the accessibility contract still holds.

## Spec Change Log

## Review Triage Log

### 2026-07-25 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 7: (high 0, medium 4, low 3)
- defer: 2: (low 2)
- reject: 0
- addressed_findings:
  - `[medium]` `[patch]` `derive_status`: `Error` + an active generation now classifies `Degraded` (records still answer via the active-generation JOIN), not `Unavailable`; resolves the UI "absent from results" contradiction; docstring + unit test updated.
  - `[medium]` `[patch]` Sidecar is best-effort — a per-source status-lookup error no longer fails the whole search (FR-14 preserved); falls back to `Unavailable` for that one source; covered by a corrupt-`scan_runs` test.
  - `[medium]` `[patch]` `SearchPage.sources` is optional in the TS runtime guard (default `[]`) for forward-compat; `api_version` stays `"1"` (additive field, no bump).
  - `[medium]` `[patch]` Codex badge UI test now asserts via `[data-provider="codex"]` (was matching the excerpt text via case-insensitive `getByText`, so a badge regression wasn't caught).
  - `[low]` `[patch]` Partial-unavailability banner renders only for genuine partial unavailability (results non-empty + ≥1 non-available source); suppressed in empty-state; hoisted so it no longer flickers with pagination.
  - `[low]` `[patch]` A stale v1 cursor returns `CursorStale` (409, graceful UI restart) instead of `BadRequest`; covered by a test.
  - `[low]` `[patch]` v1-cursor rejection + sidecar best-effort now covered by tests.
- deferred (see `_bmad-output/implementation-artifacts/deferred-work.md`): `instr(title)` is computed ~5×/row across the ORDER BY + cursor predicate — a CTE/subquery factorization is a perf micro-opt, not correctness (the perf gate is already deferred); the "no external model" AC (NFR-2) holds by inspection (no network call sites on the search path) — a regression fence is discretionary.

## Design Notes

- **Relevance is a heuristic, not FTS5.** True `bm25` ranking needs an FTS5 virtual table (a migration) and a tokenizer that preserves the pinned 2–3 char CJK recall (`search.rs` contract). That is schema-migration grade and belongs to a dedicated search-quality story. 2.3's relevance key is a pure-SQL `ORDER BY`: title-match precedence (`instr(m.title, ?) > 0`), then recency (`observed_at`), then coverage, then `record_id`. It is a defensible, deterministic "relevance" without a schema change; the spec's Block If escalates if true `bm25` is required.
- **FR-14 prototype = an informational sidecar, not record suppression.** A down source's already-indexed records stay queryable (the `Degraded` doc at `source.rs` says "existing active records remain available"); the sidecar flags `unavailable`/`degraded` so the UI can say "source X was unreachable at last scan." A source with no active generation (never scanned / failed) contributes no records and is flagged `unavailable` — the query still returns other sources' results. This avoids per-source fan-out and preserves the single-SQL query + global-revision cursor model.
- **Sidecar shape is search-local.** `SearchPage.sources` is a small per-query status list (not a reuse of `/api/sources/inventory`) so a search response is self-describing about which sources participated and their availability — the UI does not need to cross-reference inventory to render the banner.
- **UI comparison is a layout affordance.** The data layer needs no change for comparison — `provider` is already per-card. 2.3 adds a provider badge/heading (or grouping) plus the partial-unavailability banner; no split-view or complex compare component (YAGNI).
- **Continuity from 2.2 (KEEP).** Cross-provider query scope, per-result `provider` tag, full provenance, the three `empty_state` values, and the global `current_index_revision` cursor model all survive unchanged — 2.3 only adds the relevance key, the sidecar, and the UI affordances.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests pass plus new multi-provider + relevance + FR-14 sidecar tests.
- `cargo test --manifest-path server/Cargo.toml search` -- expected: multi-provider search + relevance ordering + mixed-availability sidecar green.
- `npm run build` -- expected: TS compiles with the extended `SearchPage`/`SourceQueryStatus` types.
- `npx playwright test tests/ui/accessibility.spec.ts` -- expected: multi-provider badges + partial-unavailability banner render; a11y contract holds.
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- expected: clean.

**Manual checks:**
- With one Codex + one Claude source confirmed and indexed, search a shared keyword in the app; confirm both providers' cards appear with provider badges, ordered with title-matches first, and that forcing one source unhealthy (e.g. a failed rescan) surfaces the partial-unavailability banner while the other source's results still return.

## Auto Run Result

Status: done
Follow-up review recommended: true (pass patches: medium 4, low 3 → score 15 ≥ 5).

**Summary:** Cross-provider search now ranks by a relevance key, surfaces per-source availability, and compares providers in the UI. The search SQL orders by title-match precedence → recency → coverage → `record_id` (a SQL `ORDER BY`, no FTS5/schema change; literal-substring CJK recall preserved). `SearchPage` carries a per-source status sidecar (`available`/`degraded`/`unavailable`) computed every query — a down source is flagged while its already-indexed records still return and other sources answer (FR-14 prototype; the sidecar is best-effort and never fails the query). The cursor moved v1→v2 for multi-key sort stability (a stale v1 cursor returns `cursor_stale`, graceful UI restart). The UI adds provider badges + a partial-unavailability banner (rendered only for genuine partial unavailability).

**Files changed:** `server/src/index/scan_store.rs`, `server/src/application/query.rs`, `server/src/domain/{query.rs,ports/query_store.rs}`, `src/api/search.ts`, `src/features/search/Search.tsx`, `server/tests/{search,http_api}.rs`, `tests/ui/accessibility.spec.ts`.

**Review findings:** patches applied 7 (medium 4, low 3); deferred 2 (`instr(title)` CTE perf micro-opt; NFR-2 regression fence); rejected 0.

**Verification:** `cargo test` (skip flaky perf gate) → 257 passed, 0 failed; isolated perf gate → 8 passed (no Codex regression); `cargo clippy --all-targets -D warnings` → clean; `npm run build` → clean; `npx playwright test tests/ui/accessibility.spec.ts` → 4 passed (incl. multi-provider badges + partial-unavailability banner).

**Residual risks:** (1) Relevance is a heuristic (title/recency/coverage), not bm25/FTS5 — scoped out via Block If (schema-migration grade; CJK-recall risk); a future search-quality story can swap in FTS5 without changing this spec's contract. (2) Comparison is badge-level (flat list + badges), not structural grouping/split-view (YAGNI). (3) The perf gate remains parallel-flaky (pre-existing, deferred). (4) `instr(title)` is computed ~5×/row across the ORDER BY + cursor predicate — a CTE factorization is deferred (perf micro-opt).
