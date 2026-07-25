---
title: 'Story 2.4: Cross-provider combined filtering & range visibility'
type: 'feature'
created: '2026-07-25'
status: 'done'
baseline_revision: '98940a6'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-2-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-2-3-cross-agent-search.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** Cross-provider search (2.3) returns everything matching a keyword across all confirmed sources, ordered by relevance — but Carver cannot narrow to a slice (one provider, one specific source, one memory type, one native project, a recent time window), and there is no visible statement of the currently-effective scope.

**Approach:** Add optional combined filters to `SearchRequest` (`provider`, `source`, `memory_type`, `native_project`, `since` — all `Option`, combined with AND; plus a reserved `tessera_project` slot that is accepted but ignored), apply them as additional `AND` predicates in the existing single-statement search SQL (no schema change — the columns already exist), bind them into the cursor (v2→v3 so a filter change invalidates an in-flight cursor), parse them as `/api/search` query params, and in the UI add keyboard-reachable filter controls + an effective-range readout ("Codex + Claude Code, type=memory, last 7d") + a clear-filters action that restores the full confirmed-source scope.

## Boundaries & Constraints

**Always:**
- Filters compose with AND: `provider` (single, a known provider id or `None`=all), `source` (single, a confirmed source's `source_id`/`src_<n>` or `None` — narrows to one specific source, e.g. one Claude project among several; distinct from the coarser provider filter), `memory_type` (single, a `ProviderMemoryType` or `None`), `native_project` (single, exact string match or `None`), `since` (absolute Unix-epoch seconds, `observed_at >= since`, or `None`). `None` everywhere = the full confirmed-source scope (today's default).
- The native-project filter matches `native_project` exactly and works **across providers** (any confirmed source whose records carry that project); a `NULL` `native_project` (Codex's global store) does not match a project filter — that is the honest behavior, not a bug.
- The cursor binds to the active filters (v3): a cursor whose filters differ from the request is rejected, so a mid-pagination filter change cannot page through a stale result set. The UI clears results and re-issues page 1 on any filter change (the existing invalidation pattern).
- The effective-range readout states the currently-applied scope in plain text, derived from the active filters + the confirmed providers (e.g. "Codex + Claude Code" when no provider filter, "Codex" when filtered to codex, with type/time appended when set).
- Clear-filters resets every filter to `None` and restores the full confirmed-source scope.
- The per-source availability sidecar (2.3) stays **unfiltered** — it reports availability of all confirmed sources regardless of result filters (it is availability info, not result info).
- Filter controls are keyboard-reachable with readable labels; the accessibility contract (region name, focus order, `aria-live`) is preserved.
- **`since` is stable across a pagination session:** the absolute value is resolved once on page 1 and reused for every "Load more" (never recomputed per page), so a time-preset filter does not break pagination.
- **Filter-aware empty states:** when filters are active and yield zero results, the UI does not blame the keyword — it names the active filters (or suppresses the keyword-blaming "no match" copy).
- **`tessera_project` UI slot is rendered disabled** (reserved for Epic 5), not merely absent.

**Block If:** A multi-select (OR-within-dimension) filter semantics is required. This spec uses single-select-per-dimension AND-combination (the minimal reading of "combine filters"); multi-select is a broader UX decision — stop and re-plan if OR-within-dimension is required.

**Never:**
- Add a `tessera_project` schema column or implement Tessera-Project projection (Epic 5). The `tessera_project` param/DTO field/UI slot is **reserved** — accepted, ignored at the SQL layer, shown disabled in the UI.
- Add filter-column indexes under this story (MVP scale: the active-generation set is small; a covering index is a separate migration if scale demands).
- Change the relevance ORDER BY or the literal-substring/CJK recall (2.3's contract) — filters only narrow the `WHERE`.
- Persist filter state server-side or compute relative time server-side — `since` is an absolute Unix-seconds value computed client-side.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Provider filter | filter `provider=codex`, both providers indexed+matching | Only Codex results; range readout shows "Codex"; Claude records excluded | No error |
| Source filter | filter `source=src_2` (one of several confirmed sources, same provider) | Only that source's records; range readout names it; ANDs with a provider filter if both set | No error |
| Memory-type filter | filter `memory_type=memory` | Only `memory`-type records (both providers' `MEMORY.md`); range shows "type=memory" | No error |
| Native-project filter (cross-provider) | filter `native_project=<P>` | Records from any provider carrying `<P>`; Codex (NULL project) excluded | No error |
| Time filter | filter `since=<now-7d>` (client-computed) | Only records with `observed_at >= since`; range shows "last 7d" | No error |
| Combined filters | `provider=codex` AND `memory_type=memory` AND `since=…` | AND-narrowed results; range shows the combination | No error |
| Clear filters | any active filters, click Clear | All filters → None; full confirmed-source scope restored | No error |
| Filter change mid-pagination | page 2 cursor held, user changes a filter | UI re-issues page 1 (cursor cleared); a stale filtered cursor from an older client is rejected as `cursor_stale` | Structured (cursor_stale) |
| `tessera_project` param | request carries `tessera_project=X` | Accepted, ignored at SQL layer (no filtering); UI slot disabled | No error |
| Unknown provider/memory_type | `provider=unknown` / `memory_type=bogus` | `SearchRequest` validation fails → 400 `bad_request` | Structured error |
| No filters (default) | all filters None | Today's behavior: all confirmed sources, relevance-ordered, full sidecar | No error |

</intent-contract>

## Code Map

- `server/src/domain/ports/provider_adapter.rs` — add `ProviderMemoryType::from_str(&str) -> Option<Self>` (reverse of `as_str()`) for validation.
- `server/src/domain/query.rs` — add filter fields to `SearchRequest` (`provider`, `source`, `memory_type`, `native_project`, `since`, reserved `tessera_project`) with validation in `new()` + accessors; `SearchPage` unchanged.
- `server/src/application/query.rs` — bump `CURSOR_VERSION` 2→3; embed filters (incl. `source`) in the `Cursor` struct; reject a v2 cursor (and a filter mismatch) as `CursorStale`/`BadRequest` per the existing pattern.
- `server/src/index/scan_store.rs` — `search_records`: append `AND` predicates for each `Some(filter)` (`m.provider`, `m.source_id`, `m.provider_memory_type`, `m.native_project`, `m.observed_at >= ?`) inside the existing `WHERE`; bind via the existing `params!` array; keep the relevance ORDER BY + cursor predicate unchanged.
- `server/src/http/server.rs` — `parse_search_query`: add `provider`/`source`/`memory_type`/`native_project`/`since`/`tessera_project` params (percent-decode + forward; `source` is a `src_<n>` id); reject unknown keys as today; validation lives in `SearchRequest::new`.
- `src/api/search.ts` — `searchMemories`: optional filters arg → append `URLSearchParams` entries (incl. `source`); envelope guard unchanged.
- `src/features/search/Search.tsx` — filter controls (provider, source, memory-type, native-project, time preset) + effective-range readout + Clear button + a disabled `tessera_project` slot; filter change → `++request.current` + reset to `idle` (clears cursor); keep the 2.3 banner (unfiltered) and provider badges.
- `server/tests/search.rs`, `server/tests/http_api.rs`, `tests/ui/accessibility.spec.ts` — filtered-query cases (each dimension + combined), clear-filters, cursor-stale-on-filter-change, unknown-value rejection, and a UI filter-controls test.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/ports/provider_adapter.rs` -- add `ProviderMemoryType::from_str(&str) -> Option<Self>` reversing `as_str()` -- lets `SearchRequest` validate the memory-type vocabulary from one source of truth.
- `server/src/domain/query.rs` -- add `provider: Option<String>`, `source: Option<SourceId>`, `memory_type: Option<ProviderMemoryType>`, `native_project: Option<String>`, `since: Option<i64>`, and a reserved `tessera_project: Option<String>` to `SearchRequest`; validate in `new()` (known provider id; `source` is a confirmed source's `src_<n>`; valid memory type; `since >= 0`); add accessors -- the filter contract on the request.
- `server/src/application/query.rs` -- bump `CURSOR_VERSION` to 3, embed the filter values in the `Cursor` envelope, and on decode reject a v2 cursor (or a filter mismatch vs the request) via the existing `CursorStale`/`BadRequest` path -- so a filter change cannot page through a stale result set.
- `server/src/index/scan_store.rs` -- in `search_records`, append `AND` predicates for each `Some(filter)` (`m.provider = ?`, `m.source_id = ?`, `m.provider_memory_type = ?`, `m.native_project = ?`, `m.observed_at >= ?`) inside the existing `WHERE`, bound through the existing `params!` array; `tessera_project` is accepted but produces no predicate -- narrow the result set without touching the relevance ORDER BY or CJK recall.
- `server/src/http/server.rs` -- extend `parse_search_query` with `provider`/`source`/`memory_type`/`native_project`/`since`/`tessera_project` query params (percent-decode text, parse `since` as i64; `source` is a `src_<n>` id), forward to `SearchRequest::new`, reject unknown keys as today -- surface filters on the wire.
- `src/api/search.ts` -- `searchMemories` takes an optional filters object and appends `provider`/`source`/`memory_type`/`native_project`/`since` to `URLSearchParams` when set.
- `src/features/search/Search.tsx` -- add filter controls (provider `<select>`, source `<select>`, memory-type `<select>`, native-project `<select>`/input, time preset `<select>` last-7d/30d/all) plus a disabled `tessera_project` slot, an effective-range readout region stating the active scope, and a Clear-filters button that resets all filters; on any filter change, `++request.current` and reset to `idle` (clears any held cursor) so a fresh first-page query runs. The time preset resolves `since = now - N*86400` once on page 1 and reuses that absolute value for every "Load more" in the session (do NOT recompute per page). Keep the 2.3 partial-unavailability banner (unfiltered) and provider badges.
- `server/tests/search.rs` -- add filtered-query tests (each dimension + combined AND; native-project excludes Codex's NULL), a clear-filters-equivalent default-scope test, and a cursor-stale-on-filter-change test.
- `server/tests/http_api.rs` -- assert `/api/search` accepts the filter params, applies them (filtered result set on the wire), rejects unknown provider/memory_type with 400 `bad_request`, and ignores `tessera_project`.
- `tests/ui/accessibility.spec.ts` -- add a filter-controls test: keyboard-set a provider + memory-type filter, assert the range readout + narrowed results, then Clear and assert full scope returns; keep existing tests green.

**Acceptance Criteria:**
- Given cross-source results, when Carver sets a filter (provider, source, memory-type, native-project, or time), then results converge immediately to the AND-narrowed set and the effective-range readout states the active scope (e.g. "Codex, type=memory, last 7d").
- Given active filters, when Carver clicks Clear, then all filters reset and the full confirmed-source scope is restored (range readout reflects "Codex + Claude Code" with no filters).
- Given a native-project filter, when applied, then records from any provider carrying that project match (cross-provider), and Codex's `NULL`-project records do not match.
- Given a filter change while paginating, when the next page is requested, then the UI re-issues page 1 (a stale filtered cursor from an older client is rejected as `cursor_stale`).
- Given a request with `tessera_project`, when processed, then it is accepted and ignored (no filtering; the UI slot is disabled) — reserved for Epic 5.
- Given an unknown `provider` or `memory_type` value, when the request is parsed, then it is rejected with 400 `bad_request`.
- Given the filter controls, then they are keyboard-reachable with readable labels and the search accessibility contract (region name, focus order, `aria-live`) still holds.
- Given several confirmed sources under one provider, when Carver sets a source filter to one `src_<n>`, then only that source's records return (distinct from the coarser provider filter), ANDed with any other active filters.
- Given a time-preset filter, when Carver pages with "Load more", then `since` stays constant across the session (no `cursor_stale` from a recomputed clock).
- Given active filters that yield zero results, when rendered, then the UI does not blame the keyword — it names the active filters (filter-aware empty state).
- Given the filter panel, then a disabled `tessera_project` slot is visible (reserved for Epic 5).

## Spec Change Log

### 2026-07-25 — Source filter scope correction + mooted-patch fold-in (loopback after intent-gap HALT)
- **Triggering finding:** intent_gap from pass 1 — the verbatim Story 2.4 AC title lists **Source** as a filter dimension alongside Provider/Native Project/type/time; the first spec draft silently omitted it (no per-source filter, no explicit exclusion). Human decision (2026-07-25): add a per-Source (`source_id`) filter dimension.
- **Amended:** `source` (single, a confirmed source's `src_<n>` or `None`, AND-combined) is now in scope (Always) across Boundaries, I/O matrix, Code Map, Tasks, AC, Design Notes. Also folded in the pass-1 mooted patches so re-derivation does not recreate them: (a) `since` stable across a pagination session (HIGH — was breaking "Load more" under a time preset); (b) filter-aware empty states; (c) `tessera_project` disabled UI slot (spec required, impl had missed); (d) UI test asserts results clear on filter change; (e) low defense/doc/trim items.
- **Known-bad state avoided:** shipping 2.4 without the Source dimension named in the AC; a time-preset filter that breaks pagination; a filter-induced empty set blaming the keyword.
- **KEEP (must survive re-derivation):** the verified provider/memory_type/native_project/since filters + AND-combination + cursor v3 filter-binding + effective-range readout + Clear + sidecar-stays-unfiltered (from the attempted change, re-applied via `story-2-4-attempted-change.patch`).

## Review Triage Log

### 2026-07-25 — Review pass
- intent_gap: 1: (high 1)
- bad_spec: 0
- patch: 13: (high 1, medium 3, low 9)   [mooted by the intent_gap — NOT applied this pass]
- defer: 1: (low 1)
- reject: 0
- addressed_findings:
  - none

Notes for the next run (the patches below were mooted by the intent_gap and are NOT applied; the full attempted change is saved at `_bmad-output/implementation-artifacts/story-2-4-attempted-change.patch`. Re-derivation must address these — they will recur):
  - `[high]` `[intent_gap]` The verbatim Story 2.4 AC title lists **Source** as a filter dimension alongside Provider/Native Project/type/time, but this spec implemented only `provider`/`memory_type`/`native_project`/`since` (+ reserved `tessera_project`) — it neither implements a per-source (`source_id`) filter nor explicitly excludes "Source". Per the workflow's scope-authority rule the spec's silence cannot defer a verbatim-named dimension; resolving requires editing the intent-contract (add a source filter, or record Source as folded/out-of-scope). HALTs for a human decision.
  - `[high]` `[patch]` `src/api/search.ts` `sinceFromPreset` recomputes `now` on every page, so a time-preset filter breaks pagination: page-1 cursor binds `since=S1`, "Load more" sends `since=S2` → `cursor_stale` → cannot advance. Stabilize `since` across a pagination session (resolve once on page 1, reuse for loadMore).
  - `[medium]` `[patch]` `empty_state` is not filter-aware: a filter-induced zero-result set returns `NoMatch` and the UI blames the keyword, not the active filters. Thread filters into `empty_state` (or suppress the keyword-blaming copy when any filter is set).
  - `[medium]` `[patch]` `tessera_project` disabled UI slot — the spec requires "shown disabled in the UI" (Boundaries + I/O matrix + AC); the diff rendered no slot. Add a disabled control + assertion.
  - `[medium]` `[patch]` UI test `filter-change-resets-to-idle` asserts only the range readout, not that results clear — add `toHaveCount(0)` so the reset is observed.
  - `[low]` `[patch]` `decode_cursor` skips `cursor.since` range check and `cursor.provider` vocabulary check (defense-in-depth, inconsistent with `memory_type`); `effectiveRangeText` doesn't escape quotes in `native_project`; `MAX_SINCE` doc vs value; `parse_str` doc rationale + test name `from_str`→`parse_str`; `loadMore` vs `submit` `confirmedProviders` asymmetry; `native_project` not trimmed.
  - `[defer]` `tessera_project` cursor-binding has no Epic-5 guard — drop a TODO at the cursor sites so Epic 5 binds it when it implements the predicate.

### 2026-07-25 — Review pass (pass 2: after Source filter + folded-in patches re-derivation)
- intent_gap: 0
- bad_spec: 0
- patch: 12: (high 0, medium 4, low 8)
- defer: 0
- reject: 0
- addressed_findings:
  - `[medium]` `[patch]` Added UI tests pinning the `since`-stability fix (Load more under a time preset reuses the page-1 `since`) and the filter-aware empty-state copy (a filter-induced zero-result set names the active filters, not the keyword) — both pass-1 HIGH/MEDIUM fixes that had landed without coverage.
  - `[medium]` `[patch]` Source `<select>` now scoped by the active provider filter (no longer offers a guaranteed-empty source+provider combination); a dangling selected source is cleared when it leaves the sidecar.
  - `[medium]` `[patch]` `cursor_filters_match` compares `source` by normalized rowid (`src_2`/`src_02` no longer spuriously stale); `to_rowid()` rejects non-positive rowids.
  - `[medium]` `[patch]` `native_project` trimmed at the UI evaluation sites (whitespace-only matches the server's trim-to-None); `emptyCopy` threads `confirmedProviders`.
  - `[low]` `[patch]` `decode_cursor` defense-in-depth tests; `KNOWN_PROVIDER_IDS` doc corrected + a sync test; UI mock ANDs provider+source; `ProviderMemoryType` round-trip test (already present).

## Design Notes

- **Filters narrow the existing query; they do not restructure it.** `search_records` is one SELECT; each filter is one more `AND` predicate on columns already present (`provider`, `provider_memory_type`, `native_project`, `observed_at`). The 2.3 relevance ORDER BY and the literal-substring/CJK-recall contract are untouched. No index is added (MVP scale; a covering index is a separate migration if scale demands).
- **Cursor v3 binds filters (mirror of 2.3's v1→v2).** 2.3 binds the cursor to the query string + index revision; 2.4 additionally binds it to the active filters, so a filter change invalidates an in-flight cursor (`cursor_stale`, the UI's existing recovery re-runs page 1). The UI also clears its local cursor on any filter change, so the common path never sends a stale cursor.
- **Single-select AND-combination is the minimal reading of "combine filters".** Each dimension is one value or `None`; `None` everywhere = today's default. Multi-select (OR-within-dimension) is a broader UX decision and is escalated via Block If.
- **Native-project across providers.** `native_project` is `NULL` for Codex (global store) and per-project for Claude Code. A project filter matches the exact value across all providers; Codex's `NULL` honestly does not match — the UI already labels `NULL` as "Unmapped" on cards, but the filter itself is exact-match (no virtual "Unmapped" filter value in 2.4; YAGNI).
- **Time is client-side presets → absolute seconds.** The UI offers "last 7d / 30d / all", computes `since = now - N*86400` and sends an absolute Unix-seconds `since`; the server stays stateless and deterministic (`observed_at >= since`). Free-form date input is YAGNI.
- **Sidecar stays unfiltered.** The 2.3 `sources` sidecar reports availability of every confirmed source; it is not narrowed by result filters (availability info, not result info).
- **`tessera_project` is reserved, not implemented.** No schema column, no SQL predicate; the param/DTO field/UI slot exist and are inert, so Epic 5 can fill them without a contract change here.
- **Source filter (`source_id`) is distinct from the provider filter.** A provider may own several confirmed sources (e.g. several Claude projects); the source filter narrows to one `src_<n>`. Implemented as `m.source_id = ?` (the registry rowid already on `memory_records`), bound into the v3 cursor like the other filters; `SourceId` round-trips on the wire as `src_<n>`.
- **`since` stability + filter-aware empty states** are correctness guards folded in from pass-1 review: the absolute `since` is resolved once on page 1 and reused for every "Load more"; a filter-induced zero-result set is not misreported as a keyword no-match.
- **Pass-1 low items** (decode_cursor `since`/`provider` checks, `native_project` trim + readout quote-escaping, `parse_str` naming, `MAX_SINCE` doc, loadMore/submit asymmetry, a tessera Epic-5 cursor-binding TODO) are to be addressed in this re-derivation, not deferred.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests pass plus new filter tests.
- `cargo test --manifest-path server/Cargo.toml search` -- expected: filter + cursor-stale-on-filter-change tests green.
- `npm run build` -- expected: TS compiles with the filters arg + controls.
- `npx playwright test tests/ui/accessibility.spec.ts` -- expected: filter-controls test green; existing a11y contract holds.
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- expected: clean.

**Manual checks:**
- With Codex + Claude confirmed and indexed, set a provider filter then a memory-type filter then a time preset in the app; confirm results narrow by AND, the range readout states the combination, Clear restores the full scope, and the partial-unavailability banner still reflects all confirmed sources.

## Auto Run Result

Status: done
Follow-up review recommended: true (pass-2 patches: medium 4, low 8 → score 20 ≥ 5).

**Summary:** Cross-provider combined filtering with range visibility. `SearchRequest` gains five AND-combined filters — `provider`, `source` (per-`source_id`), `memory_type`, `native_project`, `since` — plus a reserved `tessera_project` slot (accepted, ignored, shown disabled). The SQL appends flag-short-circuit `AND` predicates; the cursor moves v2→v3 and binds all filters (a filter change invalidates an in-flight cursor); HTTP parses the params; the UI adds filter controls + an effective-range readout + Clear + a disabled tessera slot. The pass-1 intent gap (Source dimension omitted from the verbatim AC) is resolved by adding the `source` filter; the pass-1 mooted patches are folded in (since-stable pagination, filter-aware empty states, decode_cursor defenses).

**Files changed:** `server/src/domain/{query.rs,ports/provider_adapter.rs,source.rs}`, `server/src/application/query.rs`, `server/src/index/scan_store.rs`, `server/src/http/server.rs`, `src/api/search.ts`, `src/features/search/Search.tsx`, `server/tests/{search,http_api}.rs`, `tests/ui/accessibility.spec.ts`.

**Review findings:** pass 1 → intent_gap 1 (Source filter — resolved by re-derivation); pass 2 → patches applied 12 (medium 4, low 8), deferred 0, rejected 0.

**Verification:** `cargo test` (skip flaky perf gate) → 286 passed, 0 failed; isolated perf gate → 8 passed (no Codex regression); `cargo clippy --all-targets -D warnings` → clean; `npm run build` → clean; `npx playwright test tests/ui/accessibility.spec.ts` → 7 passed (incl. since-stability, filter-aware empty-state, and source-scoping tests).

**Residual risks:** (1) Relevance ranking is a heuristic, not FTS5/bm25 (unchanged from 2.3; filters only narrow the `WHERE`). (2) `KNOWN_PROVIDER_IDS` is an explicit allowlist (the domain layer can't import adapters per the hexagonal rule), kept in sync by a test. (3) The perf gate remains parallel-flaky (pre-existing, deferred).
