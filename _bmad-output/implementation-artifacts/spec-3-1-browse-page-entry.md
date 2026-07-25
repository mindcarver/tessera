---
title: 'Story 3.1: BrowsePage Query Contract & No-Query Browse Entry'
type: 'feature'
created: '2026-07-25'
status: 'done'
review_loop_iteration: 0
followup_review_recommended: true
baseline_revision: '13fa5399834add9200dfb8a917de14b02b8379e0'
final_revision: '66f8855fc3898d573852bed20138788cffbd86b3'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-3-context.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** Carver cannot see what a given Agent Memory source actually contains without guessing a keyword. Search requires a query; there is no query-less way to enter a source's indexed memory and skim its scope, so "what's in here?" is unanswered.

**Approach:** Add a `BrowsePage` query contract on the Query Service that mirrors `SearchPage` (shared cursor/limit/sort/EmptyState/Coverage/Health), entered per-source from the Source Inventory with no keyword. Browse reads only through Query Service (never direct index access), reuses the search result-card/Provenance presentation in the UI, and distinguishes three empty-collection states.

## Boundaries & Constraints

**Always:**
- `BrowsePage` is served by the Query Service (`application::query`) over the `QueryStore` port. Browse reads the index ONLY through this port.
- `BrowsePage` shares `SearchPage`'s contract mechanics: `api_version` envelope, `cursor + limit`, deterministic stable sort, a revision-bound cursor that returns `cursor_stale` (HTTP 409) when any confirmed source's active generation changes, per-result Coverage Level + Source Health, the per-confirmed-source `SourceQueryStatus` sidecar, and the **same `SearchResult` DTO** reused verbatim for result rows.
- Browse is entered for a single confirmed source from its Source Inventory row; the request is scoped to that `source_id`.
- An empty first page (no cursor, zero results) returns exactly one `BrowseEmptyState`: `not_yet_scanned` | `no_indexable_memory` | `source_unavailable`, derived from the **browsed source's** scan facts (active generation + latest run state), reusing the per-source facts already aggregated by `list_inventory`/`ScanStore`.
- The browse list excludes raw chat, session/transcript, human-instruction files, and any non-confirmed source — guaranteed by existing parse boundaries plus the `lifecycle_state = 'confirmed'` SQL filter.
- The UI reuses Search's result-card / Provenance / Coverage / Health / EmptyState / pagination via shared components under `src/components/` (no duplication). The Browse view is keyboard-reachable (AD-21), and a Browse case is added to `tests/ui/accessibility.spec.ts`.

**Block If:**
- None. (The apparent "shared EmptyState enum" question is resolved by the intent: Search and Browse name *different* three-state sets, so the contract is shared at the mechanism level, not as one literal enum type. No human decision required.)

**Never:**
- Never read SQLite/index tables directly from HTTP or any path outside the `QueryStore` port. No browse-specific direct-SQL bypass of Query Service.
- Never duplicate result-card / Provenance / EmptyState / pagination into the browse feature — reuse the shared components (extract from `Search.tsx` first if not yet shared).
- Never collapse Browse's three empty states into fewer, and never reuse Search's query-bound `no_match` for browse (browse is query-less).
- Never include raw chat, transcript, human-instruction files, or records from disabled/rejected/unconfirmed sources in browse results.
- Never build dimension grouping/filter UI, drill-down navigation, knowledge-graph, auto-inferred relations, or AI summaries — those are Story 3.2/3.3 and explicit FR-17 non-goals.
- Never add the Tessera-Project browse/projection path (reserved for Epic 5); 3.1 keeps native-project scope only.
- Never change `SearchPage`'s existing wire contract or break existing search tests — the component extraction must keep Search green.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Happy path | `GET /api/browse?source=src_N` on a confirmed source with an active generation and records | `BrowsePage`: `results` (SearchResult rows), `next_cursor` when more remain, `sources` sidecar, no `empty_state` | No error |
| Continuation | `GET /api/browse?source=src_N&cursor=<next>` | Next page, deterministic order (observed_at DESC → coverage_full → record_id ASC), cursor bound to revision | No error |
| Stale cursor | source rescanned mid-pagination (revision changed) then client sends prior `cursor` | `409 cursor_stale` (`CursorStale`) | Client restarts from fresh snapshot |
| Empty — not yet scanned | confirmed source, no active generation, no successful run | `empty_state = not_yet_scanned` (page 1 only) | No error |
| Empty — no indexable memory | confirmed source, active generation (successful scan), zero records | `empty_state = no_indexable_memory` | No error |
| Empty — unavailable | confirmed source, latest run Failed/Retry, no usable active generation | `empty_state = source_unavailable` | No error |
| Non-confirmed source | `GET /api/browse?source=<disabled/rejected/unknown id>` | `400 bad_request` (phase `browse`); never returns rows from non-confirmed sources | Trust-boundary validation |
| Bad input | missing `source`, malformed cursor, invalid `limit` | `400 bad_request` (phase `browse`) | Same as search's validation |

</intent-contract>

## Code Map

- `server/src/application/query.rs` — search orchestrator, `Cursor` (CURSOR_VERSION=3), `empty_state()` walker; mirror for browse (single-source three-state).
- `server/src/domain/ports/query_store.rs` — `QueryStore` port; add the browse/list method here.
- `server/src/index/scan_store.rs` — search SQL: `instr()` predicate + sort + `source_registry` JOIN + `lifecycle='confirmed'` + active-generation JOIN. Adapt: drop `instr` + `title_match`, simpler ORDER BY.
- `server/src/domain/query.rs` — `SearchPage`/`SearchResult`/`SearchEmptyState`/`SourceQueryStatus` DTOs to reuse/mirror.
- `server/src/domain/source.rs`, `server/src/domain/ports/provider_adapter.rs` — `HealthState`/`SourceLifecycle`/`CoverageLevel`.
- `server/src/application/scan.rs` + `server/src/domain/scan.rs` — `list_inventory`/`SourceInventory` per-source facts for the three-state derivation.
- `server/src/http/server.rs`, `server/src/http/mod.rs` — route table (`/api/<resource>`, no `/v1/`), handler pattern, Host/Origin/CSP, `cursor_stale→409`, `bad_request→400`.
- `server/tests/search.rs`, `server/tests/http_api.rs`, `server/tests/inventory.rs` — test patterns to mirror.
- `src/api/search.ts`, `src/api/client.ts` — API client mirror template (`Envelope`, `API_VERSION`, runtime guards).
- `src/features/search/Search.tsx` — inlined result card / Provenance `<dl>` / Coverage-Health / Load-more / `emptyCopy()` to extract.
- `src/features/sources/Sources.tsx` — `InventoryCard` action cluster (entry point for the Browse button).
- `src/App.tsx` — static `<Sources/>`+`<Search/>` composition; add hand-rolled view state (no router).
- `tests/ui/accessibility.spec.ts` — AD-21 keyboard contract; add a Browse case.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/ports/query_store.rs` (+ `server/src/domain/query.rs`) -- add `BrowseRequest`, `BrowsePage`, `BrowseEmptyState`, browse cursor types; extend `QueryStore` with a browse/list method -- defines the shared browse contract on the Query Service boundary (reuses `SearchResult` + `SourceQueryStatus`).
- `server/src/index/scan_store.rs` -- implement the browse `QueryStore` method by adapting the search SQL (drop `instr` predicate and `title_match` rank; ORDER BY observed_at DESC, coverage_full, record_id ASC; keep the `source_registry` JOIN + `lifecycle='confirmed'` + active-generation JOIN + existing filter predicates) -- the only indexed read path, through `QueryStore`.
- `server/src/application/query.rs` -- add `browse()` orchestrator: validate `source_id` is confirmed, compute revision, decode/validate cursor (reject wrong-version/cross-type → `CursorStale`), call store, truncate `limit+1`, build next cursor, derive single-source `BrowseEmptyState` from scan facts, attach `sources` sidecar -- mirrors `application::search`; owns cursor + empty-state logic.
- `server/src/http/server.rs` + `server/src/http/mod.rs` -- add `(Method::Get, "/api/browse")` route + `browse()` handler + `parse_browse_query` (mirror search); map `CursorStale→409`, non-confirmed/bad input `→400` (phase `browse`) -- versioned endpoint under loopback + Host/Origin + CSP.
- `server/tests/browse.rs` (new) + cases in `server/tests/http_api.rs` -- application + wire tests mirroring search/http_api/inventory: pagination+cursor stability, the three empty states, stale cursor on new generation, lifecycle exclusion, non-confirmed-source `400`, wire contract + sidecar -- verify the contract.
- `src/api/browse.ts` (new) -- `BrowsePage` client mirroring `src/api/search.ts` (interface, `isBrowseEnvelope` runtime guard, `browseMemories(sourceId, cursor?, limit?)`) -- TS mirror of the Rust DTO.
- `src/components/` -- extract shared `ResultCard` (Provenance `<dl>` + Coverage/Health), `EmptyState`, and `LoadMore` from `Search.tsx`; fold the duplicated provider-display-name helper -- enable reuse without duplication.
- `src/features/search/Search.tsx` -- refactor to consume the shared components; behavior and existing tests unchanged -- preserve the shared surface Search already has.
- `src/features/browse/Browse.tsx` (new) + `src/App.tsx` -- Browse view using hand-rolled view state in `App` (no router): fetch `browseMemories(sourceId)`, render shared `ResultCard` list + `EmptyState` + `LoadMore`, keyboard-reachable, `aria-live` status, "Back to inventory" -- the no-query browse entry surface.
- `src/features/sources/Sources.tsx` -- add a "Browse" button inside `InventoryCard` (only when `lifecycle_state === "confirmed"`) that switches the App view to Browse for that `source_id` -- the Source Inventory entry affordance.
- `tests/ui/accessibility.spec.ts` -- add a Browse keyboard test (enter from inventory via keyboard, paginate, observe Provenance fields, render a three-state empty) following the existing `focus()`+Enter pattern -- AD-21 acceptance artifact.

**Acceptance Criteria:**
- Given a confirmed, successfully-scanned source with records, when Carver activates "Browse" on its Inventory card, then the Browse view shows a paginated list from `GET /api/browse?source=<id>`, each card reusing Search's Provider/Provenance/Coverage/Health components, with working cursor "Load more".
- Given a confirmed source with no active generation and no successful scan, when browsing page 1, then `empty_state` is `not_yet_scanned` and the UI shows that distinct message (never "no match").
- Given a confirmed source that scanned successfully but indexed zero records, when browsing, then `empty_state` is `no_indexable_memory` (distinct from the other two).
- Given a confirmed source whose latest run is Failed/Retry with no usable active generation, when browsing, then `empty_state` is `source_unavailable`.
- Given pagination mid-flight and the browsed source's generation changes, when the client requests the next cursor, then the API returns `409 cursor_stale` and the UI restarts from the new snapshot.
- Given a disabled/rejected/unknown `source_id`, when browsing is requested, then the API returns `400 bad_request` (phase `browse`) and returns no rows from non-confirmed sources.
- Given browse results, when the list is inspected, then it contains no raw chat, transcript, human-instruction files, or unconfirmed-source records.
- Given the Browse view, when operated by keyboard alone, then Carver can enter, paginate, and read Provenance without a pointer (asserted in `tests/ui/accessibility.spec.ts`).

## Spec Change Log

## Review Triage Log

### 2026-07-25 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 10: (high 0, medium 2, low 8)
- defer: 0
- reject: 4
- addressed_findings:
  - `[medium]` `[patch]` F1: `Browse.tsx` hardcoded `openInFlight={false}`, so the Open button never disabled during an in-flight open (double-fire). Threaded `openingRecordId` from `openState` through `renderState`/`renderResults` into `ResultCard`.
  - `[medium]` `[patch]` F2: `browse_empty_state` mis-mapped a `succeeded` run with no active generation (the `complete_without_activation` path) to `NotYetScanned`. Made run-state matching exhaustive → `NoIndexableMemory`; pinned with a test.
  - `[low]` `[patch]` F3: in-flight run states fell through silently in `browse_empty_state`; now explicit (`NotYetScanned`) in the exhaustive match.
  - `[low]` `[patch]` F4: browse's 409 `cursor_stale` carried `phase:"search"`. Parameterized `ErrorEnvelope::cursor_stale(phase)` → browse emits `phase:"browse"`; updated the wire assertion.
  - `[low]` `[patch]` F6: `partialUnavailableBanner` copy implied other sources' records could be absent from a single-source list. Reworded to scope-honest informational copy; added Playwright coverage.
  - `[low]` `[patch]` V2: the `coverage_full` ORDER-BY tiebreak was unverified. Added a test (full before search_only at equal `observed_at`; pagination across the boundary).
  - `[low]` `[patch]` V3: the future-version `b3.` cursor → `CursorStale` branch was dead code (`decode_browse_cursor` rejected the version → `BadRequest`, defeating the documented recovery). Removed the version check from decode so `browse()`'s check maps it to `CursorStale`; added a test.
  - `[low]` `[patch]` V4: malformed `b3.` cursor → `BadRequest` was unverified. Added a test.
  - `[low]` `[patch]` V5: Browse UI `cursor_stale` recovery + Restart button were dead code under mocks. Added a Playwright test (409 mock + restart re-fetch).
  - `[low]` `[patch]` IA2: `no_indexable_memory`/`source_unavailable` were not asserted at the HTTP wire. Added `src_4`/`src_5` fixtures + 2 wire tests.
  - Rejected (4): F5 — a search cursor leaking to search returning `400 bad_request` is acceptable, not a defect, and the spec forbids changing Search's wire contract. IA1 — raw-chat / human-instruction-file exclusion is a scan-layer invariant (the `memory_records` table never holds them), already covered by Stories 1.5/2.2 contract tests; re-asserting at browse tests the wrong layer. IA3 — mechanism-level (not literal-type) sharing of EmptyState/sort is the documented, intent-correct reading (Search and Browse name different state sets). V1 — a zero-row continuation page is unreachable through `application::browse` (cursors are issued only when `has_more`; records change only via generation activation → `CursorStale`), so the `cursor.is_none()` guard defends an impossible state.

## Design Notes

- **Why a separate `BrowseEmptyState`, not Search's enum.** The intent names different three-state sets for Search ("no match / not indexed / unavailable") vs Browse ("not yet scanned / no indexable Agent Memory / unavailable"). Browse is query-less so `no_match` is meaningless, and it needs `no_indexable_memory` (scanned OK, zero records) which Search lacks. "Shares EmptyState with SearchPage" is therefore honored at the mechanism level — an enum field on the page, computed only on page 1 when results are empty, communicated identically via the envelope — not as one literal shared type.
- **Why reuse `SearchResult` as-is.** Maximally honors "shares with SearchPage" + "reuse, do not re-implement"; Story 3.1 needs no field that `SearchResult` lacks. Richer per-record fields, if ever needed, are a Story 3.3 concern and would trigger an `api_version` bump then, not now.
- **Cursor.** The browse cursor binds to the same index revision (FNV-1a over confirmed sources) so any generation change → `cursor_stale`. It omits the query/`title_match` components. Ship a distinct cursor version prefix so decoding a cross-type (search) cursor yields `CursorStale`, mirroring the existing v1/v2 rejection path.
- **Sort.** `observed_at DESC → coverage_full → record_id ASC` (drops `title_match`). Deterministic and cursor-compatible — this is what "shares sort with SearchPage" means for a query-less read; it also previews 3.2's "recent first" ordering without building 3.2's grouping UI.
- **Why extract components first.** The epic mandates shared, not duplicated, result cards. `src/components/` is empty today and the card is inlined in `Search.tsx`; extraction + a Search refactor that stays green is the prerequisite for lawful reuse. Golden shape: `ResultCard({result, onOpen})`, `EmptyState({message})`, `LoadMore({onClick, disabled})`.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests plus new browse tests pass.
- `npm run build` -- expected: TypeScript compiles, production build succeeds.

**Manual checks (if no CLI):**
- If a local server can be started, run the Playwright browse case in `tests/ui/accessibility.spec.ts`; otherwise it ships as the committed AD-21 artifact for CI.

## Auto Run Result

Status: done

**Summary:** Implemented Story 3.1 — a query-less `BrowsePage` query contract on the Query Service that mirrors `SearchPage` (shared cursor/limit/sort/EmptyState/Coverage/Health, reusing `SearchResult` + `SourceQueryStatus`), entered per-source from the Source Inventory, reading only through Query Service. Browse distinguishes the three empty-collection states (`not_yet_scanned` / `no_indexable_memory` / `source_unavailable`) and reuses Search's result-card/Provenance presentation via newly-extracted shared components.

**Files changed:**
- `server/src/domain/query.rs` — `BrowseRequest`/`BrowsePage`/`BrowseEmptyState` DTOs.
- `server/src/domain/ports/query_store.rs` — `browse_records` port + `BrowseCursorKey`.
- `server/src/index/scan_store.rs` — `browse_records` SQL (observed_at DESC → coverage_full → record_id; confirmed + active-generation JOIN).
- `server/src/application/query.rs` — `browse()` orchestrator, `BrowseCursor` (b3 envelope), `browse_empty_state` (exhaustive over `ScanRunState` after review).
- `server/src/application/mod.rs`, `server/src/lib.rs` — re-exports.
- `server/src/http/mod.rs` — `browse()` handler (`cursor_stale("browse")` after review).
- `server/src/http/server.rs` — `/api/browse` route + `parse_browse_query`.
- `server/src/http/envelope.rs` — `cursor_stale(phase)` parameterized (after review).
- `server/tests/browse.rs` — 16 tests (4 added in review: Succeeded-no-gen, future-version cursor, malformed cursor, coverage tiebreak).
- `server/tests/http_api.rs` — 7 browse wire tests + `src_4`/`src_5` fixtures (after review).
- `src/api/browse.ts` — `BrowsePage` client; `src/api/search.ts` — exported shared guards.
- `src/components/{ResultCard,EmptyState,LoadMore}.tsx` + `providerDisplayName.ts` — extracted shared atoms.
- `src/features/search/Search.tsx` — refactored to consume shared components (behavior preserved).
- `src/features/sources/Sources.tsx` — Browse button on confirmed cards.
- `src/features/browse/Browse.tsx` — Browse view (openInFlight threaded + scope-honest banner copy after review).
- `src/App.tsx` — hand-rolled view state (no router).
- `tests/ui/accessibility.spec.ts` — browse keyboard test + stale/banner test (after review).

**Review findings:** 10 patches applied (2 medium: F1 open-in-flight, F2 Succeeded-no-gen empty-state; 8 low), 0 deferred, 4 rejected (F5 search cursor handling, IA1 scan-layer exclusion, IA3 mechanism-share, V1 unreachable continuation). The V3 patch's test exposed a dead-code defect (decode short-circuited future-version `CursorStale`), fixed in code.

**Follow-up review recommended:** true (10 patched findings; score 3×2 + 8 = 14 ≥ 5).

**Verification performed:**
- `cargo test --manifest-path server/Cargo.toml` — all 16 test binaries, 0 failed (browse 16, http_api 33, search 30; 1 pre-existing ignored).
- `npm run build` — `tsc -b && vite build` clean.
- `cargo clippy --manifest-path server/Cargo.toml --tests -- -D warnings` — clean.
- Playwright (`tests/ui/accessibility.spec.ts`) — committed AD-21 artifact; not run by the local gate (CI-validated).

**Residual risks:**
- Playwright browse tests (incl. the new stale/banner case) are CI-validated, not executed locally.
- Browse does not auto-focus the first new result after "Load more" (Search does); minor UX, intentionally not added to keep the Search refactor a pure extraction.
- `final_revision` records the substantive pre-amend commit sha (the self-reference is resolved by a single `--amend`).
