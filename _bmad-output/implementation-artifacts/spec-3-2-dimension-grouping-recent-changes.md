---
title: 'Story 3.2: Memory-Type Browse Filter & Recent-Scan-First Ordering'
type: 'feature'
created: '2026-07-25'
status: 'in-review'
review_loop_iteration: 0
followup_review_recommended: true
baseline_revision: '89cf308f0669f9f40405f9ab74299e0aae16809d'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-3-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-3-1-browse-page-entry.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** Within a single source's Browse view, Carver cannot narrow a large list by what kind of memory each entry is, and the "most recent" intent is implicit only — the list happens to be ordered by `observed_at DESC` but nothing communicates that. Story 3.1 shipped the per-source browse surface; this story adds the one dimension that genuinely varies within a source and makes the ordering legible.

**Approach:** Add a single real filter to `BrowseRequest` — `memory_type` — reusing Search 2.4's vocabulary (`PROVIDER_MEMORY_TYPES`) and the same `provider_memory_type = ?` SQL predicate Search already uses. Bump the browse cursor to bind the filter (cursor version `b3` → `b4`), so a memory-type change forces a fresh snapshot (mirrors Search's "resolve filter once on page 1" invariant). Make the existing `observed_at DESC` order visible in the Browse UI as "Recent scan first" — explicitly labeled as scan recency, never as content-change recency. Do not pretend Provider / Native Project / time are filterable: within one source they are constant, so they remain Browse's heading context, not filter controls.

## Boundaries & Constraints

**Always:**
- Browse still reads ONLY through the Query Service `QueryStore` port (`application::query::browse`). No browse-specific direct-SQL bypass.
- `BrowseRequest` gains exactly one optional filter field — `memory_type: Option<ProviderMemoryType>` — validated against the same `PROVIDER_MEMORY_TYPES` vocabulary Search uses (`server/src/domain/query.rs`). An invalid value returns `400 bad_request` (phase `browse`), mirroring Search's validation.
- The browse cursor binds the in-effect `memory_type` (alongside `source`, `revision`, and the existing sort key). A generation change OR a `memory_type` change invalidates any in-flight cursor → `409 cursor_stale` on continuation. This honors Search's "resolve filter once on page 1" invariant (Story 2.4 Spec Change Log) at the browse layer.
- The memory-type filter is applied as `AND m.provider_memory_type = ?` in the existing `browse_records` SQL, mirroring the predicate shape Search uses (`?N = 0 OR m.provider_memory_type = ?M` present-flag form, so a no-filter request runs the same SQL shape).
- Provider / Native Project remain Browse's heading context (shown in the sub-heading from 3.1's `providerLabel` / `nativeProject` props), never rendered as filter controls — within one source they are single-valued.
- "Recent scan first" is a **label**, not a new sort or data path. The ordering is the existing `observed_at DESC → coverage_full → record_id ASC` from 3.1; 3.2 only surfaces it in the UI copy, explicitly as *scan* recency (AD-7: never disguise Derived-Index state as source-data state).
- Browse result rows continue to reuse the `SearchResult` DTO verbatim and the shared `ResultCard` / `EmptyState` / `LoadMore` components. The memory-type filter does not require `provider_memory_type` to cross the wire on each row (it remains a filter input, not a returned attribute, matching Search 2.4).
- A memory-type-filtered browse with zero results returns `BrowseEmptyState::NoIndexableMemory` on page 1 — the same state 3.1 uses for "scanned, zero records" — because at the contract level a filter narrowing to zero is indistinguishable from a source with no records of that type. (The three-state derivation from 3.1 is unchanged: filter presence does not re-derive the source-level states.)
- TS client mirrors the Rust DTO: `browseMemories(sourceId, memoryType?, cursor?, limit?)`. The response envelope shape is unchanged (`BrowsePage` does not echo `memory_type` — it is a request param only); the client validates the `memoryType` **argument** against `PROVIDER_MEMORY_TYPES` before sending and rejects unknown values client-side. `isBrowseEnvelope` (response guard) is unaffected.
- The Browse view's filter control and the recent-first label are keyboard-reachable (AD-21); a memory-type-filter case is added to `tests/ui/accessibility.spec.ts`.

**Block If:**
- None. (The dimension-degeneracy question — "what about Provider/Native Project/time?" — is resolved by the intent: within 3.1's locked single-source scope these are constant, so they are honestly demoted to context, not filters. This is a documented product decision, not an open question.)

**Never:**
- Never widen `BrowseRequest` to cross-source scope. Provider and Native Project filters are out of scope — they belong to the Inventory panorama (Story 2.5) and cross-source Search (Story 2.4), not single-source Browse. Widening would break 3.1's locked `done` contract.
- Never add a time/date filter to Browse. `observed_at` is set once per scan (`scan.rs:330`) and is constant across a source's active generation; a time filter would be degenerate (always "all"). "Recent" is communicated only as a sort-order label.
- Never build a per-record "recent changes" / generational-diff view (added/removed/changed since last scan). That needs prior-generation data `QueryStore` does not expose; it is a separate future concern, not this story. "Recent scan first" is ordering only.
- Never add group-by / nesting / header rendering. Search 2.4 shipped filters, not groups; 3.2 is filter-by to stay consistent with A-23's shared-contract intent.
- Never return `provider_memory_type` as a new field on `SearchResult`. It stays a filter input. (If a future story needs it displayed per row, that is a separate DTO + `api_version` bump.)
- Never change Search's wire contract, DTO, or tests. The shared components and vocabulary are reused; Search is untouched.
- Never collapse or rename 3.1's three `BrowseEmptyState` variants. The filter does not introduce a fourth state.
- Never build the Tessera-Project browse/projection path (reserved for Epic 5).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Happy filtered | `GET /api/browse?source=src_N&memory_type=topic_memory` on a confirmed source with matching records | `BrowsePage`: filtered `results`, `next_cursor` when more remain, `sources` sidecar, no `empty_state` | No error |
| Filter narrows to zero | confirmed source scanned OK, has records but none of the requested `memory_type`, page 1 | `empty_state = no_indexable_memory`; UI shows that distinct message | No error |
| Invalid memory_type | `GET /api/browse?source=src_N&memory_type=bogus` | `400 bad_request` (phase `browse`); no rows returned | Same validation as Search's memory_type |
| Stale cursor (filter change) | client paginates a `memory_type=memory` cursor, then sends it with `memory_type=topic_memory` | `409 cursor_stale` (`CursorStale`, phase `browse`) | Client restarts from fresh snapshot |
| Stale cursor (generation change) | source rescanned mid-pagination, client sends prior `memory_type`-bound cursor | `409 cursor_stale` | Same recovery as 3.1 |
| Cross-type cursor reuse | client sends a `v3.` search cursor (or a `b3.` cursor from pre-3.2) to `/api/browse` | `409 cursor_stale` (browse's own version/decode checks reject it) | Mirrors 3.1's forward-compat path; new `b4.` prefix |
| No filter (unchanged) | `GET /api/browse?source=src_N` (no `memory_type`) | Identical to 3.1 — same results, same order, same empty-state semantics | No error; SQL `present=0` short-circuit keeps shape |
| Continuation | `GET /api/browse?source=src_N&memory_type=memory&cursor=<b4-next>` | Next page, same filter bound in cursor, deterministic order | No error |

</intent-contract>

## Code Map

- `server/src/domain/query.rs` — `BrowseRequest` (add `memory_type: Option<ProviderMemoryType>`); reuse `PROVIDER_MEMORY_TYPES` + `MAX_FILTER_BYTES` already defined here.
- `server/src/domain/ports/query_store.rs` — `BrowseCursorKey` is unaffected (sort key unchanged); the filter is applied in SQL, not the cursor key. The port signature `browse_records(&request, after)` already threads the filter via `request`.
- `server/src/index/scan_store.rs` — `browse_records` SQL: add `AND (?F = 0 OR m.provider_memory_type = ?G)` present-flag predicate (mirror Search's `scan_store.rs:724` shape), bind the filter.
- `server/src/application/query.rs` — `browse()`: validate `memory_type`; `BrowseCursor` gains `memory_type` field, version bump `BROWSE_CURSOR_VERSION` 3 → 4, envelope prefix `b3.` → `b4.`; cursor decode keeps the version-not-checked-in-decode rule (3.1 V3 patch) so an old `b3.` cursor reaches the version check and maps to `CursorStale`.
- `server/src/http/server.rs` — `parse_browse_query`: accept optional `memory_type` (validate against vocabulary, `MAX_FILTER_BYTES`-bounded); reject unknown/duplicate keys (existing strictness). Map invalid → `400 bad_request` (phase `browse`).
- `server/src/http/mod.rs` — `browse()` handler unchanged (envelope wraps `BrowsePage`); `envelope.rs` `cursor_stale("browse")` already parameterized (3.1 F4 patch).
- `server/tests/browse.rs` — extend: filtered happy path, filter-narrows-to-zero → `no_indexable_memory`, invalid `memory_type` → 400, stale cursor on filter change, `b3.` legacy cursor → `CursorStale`, continuation with filter bound.
- `server/tests/http_api.rs` — wire tests: `memory_type` query param, 400 on invalid, sidecar unaffected by filter.
- `src/api/browse.ts` — `BrowsePage` client: `browseMemories(sourceId, memoryType?, cursor?, limit?)`; validates the `memoryType` argument against `PROVIDER_MEMORY_TYPES` before sending (rejects unknown values client-side). The response envelope shape is unchanged — `BrowsePage` does not echo `memory_type` — so `isBrowseEnvelope` (response guard) is unaffected.
- `src/features/browse/Browse.tsx` — add a Memory-type `<select>` (id `browse-filter-type`, options from `PROVIDER_MEMORY_TYPES`, "All" default) inside a `<fieldset aria-label="Browse filters">`; reuses Search 2.4's filter-reset pattern (change → `++request.current`, clear results+cursor, re-fetch page 1); add a `data-testid="browse-effective-order"` readout stating "Recent scan first" (scan recency, explicit).
- `src/components/` — no new atom needed; the filter control mirrors Search's inline `<select>` pattern (Search's filter UI was intentionally kept inline per the 2.4 survey; consistency over extraction).
- `tests/ui/accessibility.spec.ts` — add a Browse memory-type-filter keyboard case: enter from inventory, focus+select the type filter, assert narrowed `listitem` count and the effective-order readout, paginate, observe Provenance.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/query.rs` -- add `memory_type: Option<ProviderMemoryType>` to `BrowseRequest` (+ accessor `memory_type()`); reuse the existing `ProviderMemoryType` parse/validation -- the single real filter dimension, sharing Search's vocabulary.
- `server/src/index/scan_store.rs` -- extend `browse_records` SQL with `AND (?F = 0 OR m.provider_memory_type = ?G)` present-flag predicate and bind `request.memory_type()` -- the only indexed read path, through `QueryStore`, same predicate shape as Search.
- `server/src/application/query.rs` -- bump `BROWSE_CURSOR_VERSION` to 4 and envelope prefix to `b4.`; add `memory_type` to `BrowseCursor`; in `browse()`, validate `memory_type`, bind it into issued cursors, and reject a cursor whose bound `memory_type` differs from the request's as `CursorStale` -- binds the filter into the snapshot, mirroring Search's "resolve once on page 1" invariant.
- `server/src/http/server.rs` -- extend `parse_browse_query` to accept an optional `memory_type` (vocabulary-validated, `MAX_FILTER_BYTES`-bounded, strict on unknown/duplicate keys); invalid → `Err(())` → `400 bad_request` (phase `browse`) -- the wire entry for the filter.
- `server/tests/browse.rs` + `server/tests/http_api.rs` -- add application + wire tests: filtered happy path, filter-narrows-to-zero → `no_indexable_memory`, invalid `memory_type` → 400, stale cursor on filter change and on generation change, legacy `b3.` cursor → `CursorStale`, continuation binds the filter, sidecar unaffected -- verify the contract.
- `src/api/browse.ts` -- extend `browseMemories(sourceId, memoryType?, cursor?, limit?)`; validate the `memoryType` argument against `PROVIDER_MEMORY_TYPES` before sending (reject unknown values client-side). `isBrowseEnvelope` (response guard) is unchanged — `BrowsePage` does not echo `memory_type`. -- TS mirror of the Rust request DTO.
- `src/features/browse/Browse.tsx` -- add a Memory-type `<select>` (`browse-filter-type`, `PROVIDER_MEMORY_TYPES` options, "All" default) inside a `<fieldset aria-label="Browse filters">`; on change, reset and re-fetch page 1 (mirror Search's `++request.current` + clear-results pattern); add a `data-testid="browse-effective-order"` readout labeled "Recent scan first" (scan-recency semantics, AD-7-honest); filter-narrowed-to-zero renders `EmptyState` with the `no_indexable_memory` copy -- the filter surface and the made-legible ordering.
- `tests/ui/accessibility.spec.ts` -- add a Browse memory-type-filter keyboard case (enter from inventory via keyboard, focus+selectOption on `browse-filter-type`, assert narrowed `listitem` count + the effective-order readout, paginate, observe Provenance fields) following the existing Browse a11y pattern -- AD-21 acceptance artifact.

**Acceptance Criteria:**
- Given a confirmed source with records of multiple memory types, when Carver selects a memory type in the Browse filter, then the list narrows to records of that `provider_memory_type` only, the cursor binds the filter, and "Load more" continues within the same filtered snapshot.
- Given a confirmed source scanned successfully but with no records of the selected memory type, when browsing page 1 with that filter, then `empty_state` is `no_indexable_memory` and the UI shows that distinct message (never "no match").
- Given an in-flight browse cursor bound to one `memory_type`, when the client requests the next page with a different `memory_type`, then the API returns `409 cursor_stale` and the UI restarts from the new snapshot.
- Given a `memory_type` not in the shared vocabulary, when browsing is requested, then the API returns `400 bad_request` (phase `browse`) and returns no rows.
- Given any Browse view, when rendered, then a readout communicates "Recent scan first" as scan-recency (never implying content-change tracking), satisfying AD-7's no-disguise rule.
- Given the Browse filter control, when operated by keyboard alone, then Carver can focus it, select a type, and read the narrowed results without a pointer (asserted in `tests/ui/accessibility.spec.ts`).
- Given a request with no `memory_type`, when browsing, then behavior is identical to Story 3.1 (same results, order, empty-state semantics) — the filter is purely additive.

## Spec Change Log

### 2026-07-25 — Review pass 1 (bad_spec amendment, documentation-only)
- **Triggering finding:** Blind-hunter F1 + Edge #6 — the `version != BROWSE_CURSOR_VERSION` branch in `browse()` is unreachable for legacy `b3.` and future `b5.` cursors because the prefix gate (`!raw.starts_with("b4.")`) rejects them first; the Design Note "Cursor version bump `b3.` → `b4.`" and the in-code comments described the version check as the recovery backstop, which is wrong.
- **Amended:** the Design Note below now describes the real recovery mechanism (the prefix gate is the cross-version boundary; the inner `version` field is a same-prefix backstop only). No code behavior changes — the implementation already matched 3.1's V3 spirit (decode does not validate version; prefix gates cross-version reuse; version check is a same-prefix integrity backstop).
- **Known-bad state avoided:** a future maintainer bumping the cursor trusting the old note would expect `browse()`'s version check to handle a legacy prefix and could weaken the prefix gate, or would mis-document the recovery path. The corrected note names the prefix gate as the forward-compat boundary.
- **KEEP:** the prefix-gate-rejects-cross-version → `CursorStale` behavior is correct and must survive; the inner `version` field stays in the cursor struct as a same-prefix backstop (do not delete it).


## Review Triage Log

### 2026-07-25 — Review pass 1
- intent_gap: 0
- bad_spec: 1: (low 1) — Design Note "Cursor version bump" described the version check as the cross-version recovery backstop; the prefix gate is the real boundary. Amended the Design Note (documentation-only; code behavior unchanged).
- patch: 8: (high 0, medium 2, low 6)
- defer: 1
- reject: 4
- addressed_findings:
  - `[low]` `[bad_spec]` Cursor-version Design Note described an unreachable control flow (version check as cross-version backstop). Amended the Design Note to name the prefix gate as the cross-version boundary and the inner `version` field as a same-prefix integrity backstop only. Code unchanged — implementation already matched 3.1's V3 spirit.
  - `[medium]` `[patch]` F3 (blind-hunter): a hand-edited cursor whose `memory_type` fails `parse_str` returns `BadRequest` (400), but a cursor whose `memory_type` parses but mismatches returns `CursorStale` (409). Move the `parse_str` vocabulary check out of `decode_browse_cursor` so every "cursor filter ≠ request" case funnels to `CursorStale` — the 409 recovery (re-run page 1) is the correct UX for both.
  - `[medium]` `[patch]` VG-F1 (verification-gap): the new `decode_browse_cursor` smuggled-`memory_type` rejection has no test; search's identical sibling is tested. Add a browse test mirroring search's `decode_cursor(... Some("bogus_type") ...).is_none()` (assert `application::browse` returns `BadRequest`, or `CursorStale` once F3's reroute lands).
  - `[low]` `[patch]` F1+F2 (blind-hunter): the `version != BROWSE_CURSOR_VERSION` comment and the `browse_rejects_future_version_cursor_as_stale` test describe/claim a forward-compat path the prefix gate makes unreachable for real `b5.` cursors. Fix the comment to name the prefix gate as the forward-compat mechanism; add a `b5.`-prefixed case to the test proving the prefix gate rejects it as `CursorStale`.
  - `[low]` `[patch]` F4 (blind-hunter): `BrowseCursor.memory_type` is the only cursor string field without a length cap in decode. Add `MAX_FILTER_BYTES` bound alongside the vocabulary check, matching the `source`/`native_project` pattern in `decode_cursor`.
  - `[low]` `[patch]` Edge #2: the cursor filter comparison is by raw stored string, not typed enum — correctness depends on an unstated `parse_str`/`as_str` inverse invariant. Normalize on the comparison path (`cursor.memory_type.as_deref().and_then(ProviderMemoryType::parse_str) != request.memory_type()`).
  - `[low]` `[patch]` Edge #5: `browse_memory_type_filter_change_invalidates_cursor_as_stale` claims symmetry but only covers `unfiltered→filtered` and `memory→topic_memory`; the `topic_memory→memory` replay is abandoned (`let _ = page1_topic` silences clippy). Add a second topic record + a `topic_memory`-issued cursor replayed under `memory` asserting `CursorStale`; remove the dead-code bind.
  - `[low]` `[patch]` F6 (blind-hunter): `browseMemories` guard message "Tessera core rejected an unknown memory type." blames the server for a client-side throw. Reword to attribute the rejection to the client before sending.
  - `[low]` `[patch]` F8 (blind-hunter): `BrowseRequest` struct doc overstates that `memory_type` is "validated at the HTTP layer" — the domain constructor accepts any `ProviderMemoryType` without re-validation. Soften the doc to "callers MUST obtain the value via `parse_str`."
- deferred_findings:
  - Edge #1: `memory_records.provider_memory_type` has schema default `''` (v3 migration), a value outside the 5-variant filter vocabulary; filtered browse silently excludes such rows. Pre-existing schema-level hole, not introduced by this story.
- rejected_findings:
  - F9 (blind-hunter): filter-narrows-to-zero reusing `NoIndexableMemory` — the intent-contract explicitly chose this and forbids a fourth state; reopening it contradicts the binding intent.
  - F5 (blind-hunter): Playwright mock cursor `b4.page2` is non-hex — mock is UI-only, cursor is opaque to the client, consistent with sibling browse/search mocks.
  - F7 (blind-hunter): `memoryType` truthiness guard silently drops `""` — `ProviderMemoryType` is a non-empty union; speculative future risk, no current defect.
  - VG-F2 (verification-gap): `browseMemories` client-side throw unverified — no TS unit-test layer exists; server already rejects the value (covered); browse-only defense-in-depth with no sibling precedent.
  - F10 (blind-hunter): restart shows old-filter results briefly — speculative timing window, over-speculative.


## Design Notes

- **Why only `memory_type`, and why the other dimensions are demoted.** Verified in code: within a single confirmed source, `provider` is constant (one source = one provider), `native_project` is `source.native_project.clone()` per record (`scan.rs:371`, NULL for Codex), and `observed_at` is `unix_seconds_now()` set once per scan (`scan.rs:330`) and stamped on every record of that generation. Provider/Native Project/Time are therefore single-valued within the browseable set — a filter control for them would be dishonest (clicking does nothing). `provider_memory_type` is the only column that genuinely varies. This demotion is a documented product decision (confirmed with Carver), not a gap: cross-source Provider/Native-Project grouping already lives at the Inventory panorama (Story 2.5) and cross-source Search (Story 2.4); Browse's unique value is per-source scope, which this story preserves.
- **Why "recent changes" becomes "recent scan first".** A true per-record generational diff (added/removed/changed since last scan) needs prior-generation data `QueryStore` does not expose — a separate future data path, out of scope here. `observed_at DESC` is already 3.1's sort; 3.2 only makes it legible. The label says *scan* recency explicitly so it never implies content-change tracking (AD-7: do not disguise Derived-Index state as source-data state). All records in one active generation share one `observed_at`, so the ordering is stable within a generation and the `coverage_full → record_id` tiebreaks from 3.1 remain meaningful.
- **Why filter-by, not group-by.** Search 2.4 shipped filters; A-23's "shared query contract" is honored at the mechanism level (same cursor/limit/sort/EmptyState shape), and filter-by keeps Browse consistent with Search rather than introducing a nesting concept the epic doesn't require.
- **Cursor version bump `b3.` → `b4.`.** Adding `memory_type` to the cursor body changes its shape. The **prefix** (`b3.` → `b4.`) is the cross-version boundary: any cursor whose prefix differs from the current one (a legacy `b3.` browse cursor, a future `b5.` browse cursor, or a cross-type `v3.` search cursor) is rejected at the prefix gate in `decode_browse_cursor` and maps to `CursorStale` — the client restarts from page 1. The inner `version: u8` field is kept as a same-prefix integrity backstop only (decode does NOT validate it; `browse()`'s `version != BROWSE_CURSOR_VERSION` check catches a `b4.`-prefixed body whose inner version is wrong, e.g. a hand-repaired or buggy encoder). This preserves 3.1's V3 rule (decode stays version-agnostic) while the prefix carries the cross-version responsibility.
- **Why `no_indexable_memory` for filter-narrows-to-zero.** At the contract level, "this source has no records of the requested type" is indistinguishable from "this source has no indexable memory" — both are a zero-row first page on a scanned-OK source. Reusing 3.1's state keeps the three-state space intact (no fourth state) and the derivation logic unchanged. The UI copy is the same distinct message 3.1 already ships.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests plus new browse memory_type tests pass; http_api browse cases green.
- `cargo clippy --manifest-path server/Cargo.toml --tests -- -D warnings` -- expected: clean.
- `npm run build` -- expected: TypeScript compiles, production build succeeds.

**Manual checks (if no CLI):**
- The Playwright memory-type-filter case in `tests/ui/accessibility.spec.ts` ships as the committed AD-21 artifact; CI-validated if not run locally.

## Auto Run Result

Status: done

**Summary:** Implemented Story 3.2 — a single real filter dimension (`memory_type`) on the per-source `BrowseRequest`, reusing Search 2.4's `PROVIDER_MEMORY_TYPES` vocabulary and the same present-flag SQL predicate; the filter is bound into the browse cursor (version `b3.` → `b4.`) so a filter change or generation change invalidates an in-flight cursor → `409 cursor_stale`. The existing `observed_at DESC` order is surfaced in the Browse UI as an explicit "Recent scan first" readout (scan-recency label, AD-7-honest). Provider / Native Project / time are honestly demoted to heading context — they are single-valued within a source, so filter controls for them would be dishonest (documented product decision).

**Files changed:**
- `server/src/domain/query.rs` — `memory_type: Option<ProviderMemoryType>` on `BrowseRequest` + `new_with_memory_type`; doc softened in review (P8).
- `server/src/index/scan_store.rs` — `browse_records` SQL gains `AND (?6 = 0 OR m.provider_memory_type = ?7)` present-flag predicate.
- `server/src/application/query.rs` — `BROWSE_CURSOR_VERSION` 3→4, prefix `b3.`→`b4.`; `BrowseCursor.memory_type` field; cursor filter-mismatch → `CursorStale`; comments corrected in review (P1/P2/P3/P4).
- `server/src/http/server.rs` — `parse_browse_query` accepts optional `memory_type` (vocabulary-validated, strict on duplicates/unknown keys → 400).
- `server/tests/browse.rs` — memory_type filter tests (happy, narrows-to-zero, paginates-in-snapshot, symmetric filter-change → stale, vocabulary acceptance, legacy `b3.` → stale, future-prefix `b5.` → stale (P1), smuggled-`memory_type` cursor → stale (P6)); 24 tests total.
- `server/tests/http_api.rs` — wire tests (filter narrows + sidecar stays unfiltered, paginates, filter change → 409, unknown → 400, legacy `b3.` → 409).
- `src/api/browse.ts` — `browseMemories(sourceId, memoryType?, cursor?, limit?)`; validates `memoryType` against `PROVIDER_MEMORY_TYPES` before sending; message reworded in review (P7).
- `src/features/browse/Browse.tsx` — Memory-type `<select id="browse-filter-type">` inside `<fieldset aria-label="Browse filters">`; `data-testid="browse-effective-order"` "Recent scan first" readout; filter change re-fetches page 1.
- `tests/ui/accessibility.spec.ts` — Browse memory-type-filter keyboard case (enter, focus+selectOption, assert narrowed count + readout + Provenance + pagination).

**Review findings:** Review pass 1 — 0 intent_gap, 1 bad_spec (low, doc-only Design Note amendment — cursor-version note described an unreachable control flow; prefix gate is the real cross-version boundary), 8 patches applied (2 medium: P2 cursor-filter 400→409 funnel, P6 smuggled-cursor test; 6 low: P1 version-check comment + `b5.` test, P3 length bound, P4 typed comparison, P5 symmetric test, P7 message, P8 doc), 1 deferred (`''` schema-default `provider_memory_type`, pre-existing), 4 rejected. Patch score 3×2 + 6 = 12 ≥ 5 → follow-up review recommended.

**Follow-up review recommended:** true.

**Verification performed:**
- `cargo test --manifest-path server/Cargo.toml` — 16 binaries, 0 failed (browse 24, http_api 38, search 30, source_registry 24, lib 90; 1 pre-existing ignored).
- `cargo clippy --manifest-path server/Cargo.toml --tests -- -D warnings` — clean.
- `npm run build` — `tsc -b && vite build` clean (223.15 kB).
- Playwright (`tests/ui/accessibility.spec.ts`) — committed AD-21 artifact; not run by the local gate (CI-validated).

**Residual risks:**
- Playwright memory-type-filter test is CI-validated, not executed locally.
- Under P2's normalization, a hand-edited cursor carrying a bogus `memory_type` against an *unfiltered* request is accepted (the bogus value normalizes to `None`, matching the unfiltered request) — documented and intentional; only a *filtered* request yields `CursorStale`. No security impact: the sort key and revision binding are intact; the bogus value carries no actionable filter.
- A 3.1 client holding a `b3.` cursor across the server upgrade gets `cursor_stale` on its first "Load more" and re-runs page 1 — the documented, intentional recovery path.
