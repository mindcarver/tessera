---
title: 'Story 5.2: Browse and Search by Tessera Project (Projection)'
type: 'feature'
created: '2026-07-26'
status: 'done'
baseline_revision: '6f8b69d'
final_revision: '36358d6f89ab3753524720d760ef5088a60d5373'
review_loop_iteration: 0
followup_review_recommended: false
context:
  - '{project-root}/CLAUDE.md'
  - '{project-root}/_bmad-output/implementation-artifacts/epic-5-context.md'
warnings:
  - oversized
---

<intent-contract>

## Intent

**Problem:** Story 5.1 built the Tessera Project mapping layer (`tessera_projects` + `project_mappings`, six `/api/projects*` endpoints), but the reserved `tessera_project` search filter is still a no-op: it is accepted on the wire/DTO/cursor struct but produces no SQL predicate, is not bound into the cursor, is absent from the TS filter type, and renders as a disabled `<select>` in the UI. There is also no signal that invalidates in-flight pagination when a mapping changes. So Carver still cannot browse/search the aggregate of one Tessera Project's mapped Native Projects.

**Approach:** Fill the Search-side projection end-to-end: add a `tessera_project` SQL predicate over `memory_records` × `project_mappings` (no copy of canonical rows — AD-2); introduce a monotonic `project_mapping_revision` scalar in `tessera_meta`, bumped inside the existing project transaction whenever the mapped scope set changes; fold that revision into `current_index_revision()` so any mapping change makes every outstanding cursor return `cursor_stale` (HTTP 409) and the caller restarts from page 1 (AD-26/AD-31); and replace the disabled UI slot with a live control fed by `listProjects()`. Browse is intentionally left single-source (decision Q1=A — see Design Notes); the AC's "browse" is satisfied by Search with an empty query + `tessera_project` filter. `source_status_sidecar` narrows to the project's mapped sources when the filter is set (Q3=A).

## Boundaries & Constraints

**Always:**
- Projection is read-only over canonical records: `memory_records` joined to `project_mappings` via an `EXISTS` predicate on `(provider, COALESCE(native_project,''))` — never copies rows, never alters native identity (AD-2). The `COALESCE` collapse matches the Story 5.1 uniqueness index, so a Codex global (`native_project NULL`) maps correctly.
- `project_mapping_revision` is a single monotonic integer in `tessera_meta`, seeded `0` by migration id 8, bumped (+1) **inside** the existing `ProjectStore::with_transaction` on every operation that changes the mapped scope set: successful `insert_mapping` (new mapping), `remove_mapping` (a row deleted), and `delete` (a project's mappings removed). It is NOT bumped by `create`, `rename`, or idempotent re-add (those leave the scope set — and thus every projection result — unchanged).
- Reset Index (Story 4.4) preserves `project_mapping_revision`: the reset wipe keys on the `active_generation:` prefix only, so the mapping-revision key survives a rebuild (AD-29).
- Cursor snapshot binds mapping state via `current_index_revision()`: its FNV-1a input gains the `project_mapping_revision` value (read from `tessera_meta`, absent ⇒ `0`). Any mapping change ⇒ revision changes ⇒ every outstanding search **and** browse cursor returns `QueryError::CursorStale` ⇒ HTTP 409 `cursor_stale` (AD-31; the stable code `cursor_stale` is retained — Q2=A; AC's `stale_snapshot` is the concept name, not a literal code).
- The search cursor additionally binds the `tessera_project` filter (resolving the existing Epic-5 TODO at `application/query.rs:129-131`), and `cursor_filters_match` compares it, so changing the project mid-pagination also returns `cursor_stale`. Cursor envelope version rises search `v3`→`v4` (structural change); an old `v3` cursor is rejected as `CursorStale` via the same prefix-gate pattern used today for `v1.`/`v2.` (`application/query.rs:82-84`). Browse cursor structure is unchanged (`b4` retained — no `tessera_project` field); its `revision` simply now includes mapping state, so an outstanding browse cursor goes stale on mapping change by revision mismatch alone.
- `source_status_sidecar` narrows when `tessera_project` is set: it lists only confirmed sources whose `(provider, COALESCE(native_project,''))` is in that project's mapping set. With no `tessera_project` filter it is unchanged (all confirmed sources).
- `api_version` stays `"1"` (additive). Loopback-only (`127.0.0.1`). New migration is `STRICT`-consistent, snake_case, INTEGER Unix-seconds. Reuse `unix_seconds_now_i64()`, `unchecked_transaction()` + `with_transaction`, the `Envelope<T>` contract, and the existing `parse_search_query` parameter handling (the `tessera_project` URL param is already wired through `http/server.rs:511-513`). No new crate/npm dependencies (locked stack).
- NFR-3 redaction: errors/logs contain no memory body, query text, or credentials. Zero-source-mutation gate holds (projection is a pure read; source file set/content/size/mtime unchanged).

**Block If:** none. All design decisions resolved (Q1=A Search-only projection, Q2=A keep `cursor_stale`, Q3=A narrow sidecar).

**Never:**
- Do NOT copy canonical records into a projection table or materialize per-project result rows.
- Do NOT change `BrowsePage`'s single-source model or add a `tessera_project` filter/parameter to `BrowseRequest` / `parse_browse_query` (Q1=A). Browse stays query-less and source-scoped.
- Do NOT rename the stable error code `cursor_stale` (Q2=A). Do NOT add a `stale_snapshot` code.
- Do NOT bypass the Query Service (`application::search`/`browse` → `ScanStore` → SQL) to read index tables directly for projection (AD-23).
- Do NOT auto-merge scopes, auto-project unmapped entries, or silently override a mapping (AD-24).
- Do NOT delete or modify `memory_records` / `source_registry` rows. Do NOT introduce new dependencies.

## I/O & Edge-Case Matrix

`Envelope<T>` with `api_version:"1"`. `tessera_project` on the wire is the `proj_<n>` id (already accepted by `parse_search_query`, bounded, forwarded to `SearchFilters`; today ignored at SQL — this story makes it effective). Projection = `EXISTS (SELECT 1 FROM project_mappings pm WHERE pm.tessera_project_id=:rowid AND pm.provider=m.provider AND COALESCE(pm.native_project,'')=COALESCE(m.native_project,''))`.

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Project w/ 2 mappings, search by project | confirmed sources whose `(provider,native_project)` match the project's mappings; query "x" | only records from the mapped scopes, full Provenance intact, ranked as today | none |
| Codex-global scope mapped | project maps `(codex, null)`; `memory_records.native_project IS NULL` for Codex global | those Codex-global records ARE returned (NULL matched via `COALESCE`) | none |
| Empty keyword by project | `q=` empty string + `tessera_project` set (omitting `q` ⇒ 400, so the UI sends `q=`) | all records of mapped scopes, ranked (browse-equivalent for the project) | none |
| Project with 0 mappings | newly created project, search by it | empty results + `SearchEmptyState::NoMatch` on page 1 | none |
| Unknown project id | `tessera_project="proj_999"` (no such row) | empty results (no mapped scopes ⇒ predicate excludes all) — treated as a filter that matches nothing, NOT an error | none |
| Mapping changes mid-pagination | page-2 cursor issued, then an `add_mapping`/`remove_mapping`/`delete` bumps the revision | next page → `409 cursor_stale`; UI re-runs page 1 under the new snapshot | `cursor_stale` |
| Old `v3` search cursor after upgrade | client resends a `v3.<hex>` cursor | `409 cursor_stale` (prefix-gate rejection) | `cursor_stale` |
| Mid-pagination project-filter change | page-2 cursor for project A, request now targets project B | `409 cursor_stale` (cursor's bound `tessera_project` ≠ request) | `cursor_stale` |
| Sidecar with project filter | `tessera_project` set | `sources[]` lists only confirmed sources in the project's mapping set | none |
| Sidecar without project filter | no `tessera_project` | `sources[]` lists all confirmed sources (unchanged) | none |

</intent-contract>

## Code Map

- `server/src/index/migrations.rs` -- add migration `id:8` `v7_project_mapping_revision` that seeds `tessera_meta` key `project_mapping_revision` = `0` (`INSERT OR IGNORE INTO tessera_meta(key,value) VALUES('project_mapping_revision','0')`); the runner uses `MIGRATIONS` (current max id `7` from Story 5.1).
- `server/src/index/mod.rs` -- `CURRENT_SCHEMA_VERSION` `7`→`8`; keep `ProjectStore` re-export.
- `server/src/index/project_store.rs` -- add `bump_project_mapping_revision(&self) -> rusqlite::Result<()>` (`UPDATE tessera_meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key='project_mapping_revision'`) and `project_mapping_revision(&self) -> rusqlite::Result<i64>` (read). Both reuse the borrowed `conn`. No new table.
- `server/src/application/project.rs` -- call `bump_project_mapping_revision` inside the existing `with_transaction` for: successful new `insert_mapping`, `remove_mapping` that deleted a row, and `delete` (after the mappings delete). Do NOT bump on create / rename / idempotent re-add.
- `server/src/index/scan_store.rs` -- (a) `search_records` WHERE gains the `tessera_project` `EXISTS` predicate gated by a presence flag (same `?N = 0 OR …` idiom as the other filters at `:814-835`), binding the resolved project rowid; (b) `current_index_revision` (`:1038-1064`) folds `project_mapping_revision` (read via `tessera_meta`, absent ⇒ `0`) into the FNV-1a input alongside the `(source_id, active_generation)` pairs.
- `server/src/application/query.rs` -- `Cursor` gains `tessera_project: Option<String>` (resolve the TODO at `:52-55`/`:129-131`); encode/decode it; `cursor_filters_match` (`:447-455`) compares `tessera_project`; raise `CURSOR_VERSION` `3`→`4` and reject `v3.` prefixes as `CursorStale` (mirror the `v1.`/`v2.` gate at `:82-84`). `BrowseCursor` is unchanged structurally (its `revision` now carries mapping state via `current_index_revision`, so it goes stale on mapping change by revision mismatch — no version bump needed). `source_status_sidecar` (`:469-500`) narrows to the project's mapped `(provider, COALESCE(native_project,''))` set when `request.tessera_project()` is `Some`.
- `server/src/domain/query.rs` -- `SearchFilters.tessera_project` / `SearchRequest` accessor stay `Option<String>` (the `proj_<n>` wire form); resolution to rowid happens at the SQL-binding boundary (reuse `ProjectId::to_rowid`). No new validation beyond the existing length cap.
- `server/src/http/server.rs` -- `parse_search_query` already parses/forwards `tessera_project` (`:511-513`); confirm it reaches `SearchFilters` (no parse change expected). `parse_browse_query` is NOT extended (Q1=A).
- `src/api/search.ts` -- add `tessera_project?: string` to `SearchFilters` (`:89`); emit it in `buildSearchParams` (`:111`); extend the `isSearchEnvelope` guard if needed.
- `src/features/search/Search.tsx` -- replace the disabled `<select>` (`:332-335`) with a live control populated by `listProjects()` (`src/api/projects.ts`), wired to filter state + `updateFilter`; clear the local cursor on change (existing pattern). When a project is selected without a keyword, send `q=` (empty) so the backend accepts the browse-by-project request (search requires the `q` param — `server.rs:518`).
- Tests -- `server/tests/search.rs` (update the reserved-slot case at `:871-889` to assert the predicate now filters), `server/tests/http_api.rs` (`:1322-1336`), plus new cases: projection happy path + Codex-NULL scope, empty-query-by-project, unknown-project ⇒ empty, mapping-change ⇒ `cursor_stale`, old `v3` cursor ⇒ `cursor_stale`, sidecar narrowing, `project_mapping_revision` bump-on-add/remove/delete and no-bump-on-create/rename/idempotent, reset-index preserves the revision (AD-29). UI: `tests/ui/accessibility.spec.ts` — flip the reserved-filter assertion to assert the control is now enabled and keyboard-reachable.

## Tasks & Acceptance

**Execution:**
- `server/src/index/migrations.rs` + `server/src/index/mod.rs` -- add migration id 8 seeding `project_mapping_revision=0` and bump `CURRENT_SCHEMA_VERSION` to 8 -- foundational scalar for snapshot binding.
- `server/src/index/project_store.rs` + `server/src/application/project.rs` -- add read/bump helpers; bump inside the transaction on scope-set-changing ops only -- the single source of revision truth.
- `server/src/index/scan_store.rs` -- add the `tessera_project` `EXISTS` predicate to `search_records`; fold `project_mapping_revision` into `current_index_revision` -- the projection + the stale-on-change signal.
- `server/src/application/query.rs` -- bind `tessera_project` into the search `Cursor` + `cursor_filters_match`; raise cursor version `v3`→`v4` with stale rejection; narrow `source_status_sidecar` by project -- completes the query-service side of AD-23/AD-26/AD-31.
- `server/src/domain/query.rs` (+ confirm `http/server.rs`) -- keep the wire/DTO shape; resolve `proj_<n>`→rowid at the SQL boundary -- additive contract, no parse change.
- Rust tests (co-located) -- migration applies + `schema_version=="8"` + revision seeded 0; projection predicate; Codex-NULL match; empty/unknown-project ⇒ empty; bump-on-add/remove/delete and no-bump-on-create/rename/idempotent; mapping-change ⇒ `cursor_stale` (search and browse); old `v3` cursor ⇒ `cursor_stale`; sidecar narrowing; reset-index preserves revision; non-destruction + zero-source-mutation.
- `src/api/search.ts` + `src/features/search/Search.tsx` -- typed filter + live control fed by `listProjects()` -- the user-facing project filter (UX-DR4 default-scope-isolation display is served by showing mapped sources only).
- `tests/ui/accessibility.spec.ts` -- flip the reserved-slot assertion to enabled + keyboard-reachable -- AD-21/NFR-13 contract holds with the slot now active.

**Acceptance Criteria:**
- Given a Tessera Project with mappings to Native Projects across Codex and Claude Code, when a search sets `tessera_project` to that project, then every returned record's `(provider, native_project)` is in the project's mapping set, Provenance is complete, and records of unmapped scopes are excluded — with no canonical row copied or modified.
- Given the project maps `(codex, null)`, when searched by project, then Codex-global records (`native_project IS NULL`) are returned (NULL scope matched via `COALESCE`).
- Given the project has zero mappings (newly created), when searched by project, then the result is empty with `SearchEmptyState::NoMatch` on page 1.
- Given a page-2 cursor was issued, when any mapping is added/removed (or the project deleted) before the next page, then the next-page request returns `409 cursor_stale` and the caller restarts from page 1 under the new snapshot (AD-26/AD-31).
- Given an outstanding browse cursor, when a mapping changes, then the next browse page also returns `409 cursor_stale` (mapping revision is bound into the shared index revision).
- Given a `v3.<hex>` search cursor after upgrade, when it is replayed, then the response is `409 cursor_stale` (forward-compatible recovery).
- Given `tessera_project` is set, when the response's `sources[]` sidecar is inspected, then it lists only confirmed sources in the project's mapping set; with no `tessera_project` it lists all confirmed sources (unchanged).
- Given Reset Index is run, when `project_mapping_revision` is read after, then it is unchanged (mappings and their revision survive rebuild — AD-29).
- Given any sequence of add/remove/delete-mapping operations, when `GET /api/sources/inventory` and an unfiltered search are re-run, then record counts, health, and results are identical to before (projection is read-only; non-destruction), and source file sizes/mtimes are unchanged (zero-source-mutation).
- Given a keyboard-only user, when they focus the Tessera-project filter in Search, then they can pick a project via keyboard, the choice filters results, and status changes are announced via `aria-live`.
- Given the implementation, when `cargo test --manifest-path server/Cargo.toml` and `npm run build` are run, then both succeed (new Rust tests green; TypeScript compiles; pre-existing Windows-only Unix test failures ignored per project memory).

## Spec Change Log

<!-- Empty until the first bad_spec loopback from step-04. -->

## Review Triage Log

### 2026-07-26 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 6: (high 0, medium 0, low 6)
- defer: 5
- reject: 2
- addressed_findings:
  - `[low]` `[patch]` **Malformed `tessera_project` sidecar** (`application/query.rs` `source_status_sidecar`): a malformed id (`to_rowid()` None, e.g. `proj_x`) collapsed to `None` (no narrowing → full source sidecar) instead of an empty set, contradicting the doc comment and the SQL layer (which yields empty results for the same input). Fixed: `Some(id)` now always yields `Some(set)`, empty when unresolvable/zero-mapping. New test `malformed_tessera_project_id_yields_empty_results_and_empty_sidecar`. (adversarial + edge-case + verification-gap lenses.)
  - `[low]` `[patch]` **`bump_project_mapping_revision` silent no-op + NULL-write** (`index/project_store.rs`): UPDATE returned `Ok` on 0 rows (seed absent → AD-31 cursor invalidation never fires) and `CAST(NULL+1)` wrote NULL permanently with a self-masking read. Fixed: `COALESCE(CAST(value AS INTEGER),0)+1` + affected-rows check with `INSERT OR IGNORE` re-seed.
  - `[low]` `[patch]` **`new_with_filters` dead duplicate** (`domain/query.rs`): unreachable second `MAX_QUERY_BYTES` clause + misleading comment. Simplified to one authoritative gate; empty-`q` carve-out preserved.
  - `[low]` `[patch]` **Combined AC test** added: `mapping_change_invalidates_cursor_with_same_project_filter` (mapping-change × `tessera_project`-set), previously covered only in isolation.
  - `[low]` `[patch]` **`decode_cursor` length-cap test**: over-length `tessera_project` body variant added to `decode_cursor_rejects_tampered_bound_filters`.
  - `[low]` `[patch]` **Cursor version-byte regression test**: the adversarial lens flagged `Cursor.version` as decorative; on verification the `cursor.version != CURSOR_VERSION` check already exists (`application/query.rs:652`, Story 2.4) — the finding was a false positive (routed reject); added `decode_cursor_rejects_tampered_version_byte` to lock the behavior against future refactors.
  - Deferred to `deferred-work.md`: EXISTS predicate non-sargable O(N×mappings) cost (perf); `Search.tsx` `listProjects` fetch-on-mount staleness; `effectiveRangeText` leaking `proj_<n>` to screen readers before name resolves; Playwright UI test does not `selectOption` + assert narrowing/aria-live; no TS unit test for `buildSearchParams` `tessera_project` emission (no vitest in the locked stack).
  - Rejected: Playwright `/api/projects` route mock registered after mount fetch (noise); Cursor version-byte finding (false positive — defensive test kept, see above).

## Design Notes

**Why projection reuses the Search path and Browse stays single-source (Q1=A).** Story 2.4 reserved the `tessera_project` slot **only on Search** — an explicit Epic-2 architectural decision that project filtering is a Search concern. `BrowsePage` is single-source + query-less by design (`BrowseRequest` is `source/cursor/limit/memory_type`; `native_project` is constant within one source's generation). Forcing Browse to aggregate across a project's mapped sources would fight that model. The AC's "browse" is satisfied by **Search with an empty `q` parameter + `tessera_project`**: `instr(m.title, '')` matches every row, so `q=` ranks all in-scope records. Note `parse_search_query` rejects a request that **omits** `q` entirely (`server.rs:518` `q.ok_or(())?` → 400), so the UI must send `q=` (empty string) when browsing a project without a keyword — a request shape the backend already accepts. `project_mapping_revision` is still bound into the **shared** `current_index_revision`, so outstanding browse cursors go stale on mapping change too — that is the global snapshot hygiene AD-31 requires, and it is why Story 5.1's deferral note mentioned `BrowseCursor` without implying Browse accepts a project filter.

**Why `cursor_stale` is retained (Q2=A).** It is already the `api_version:"1"` stable code (`http/envelope.rs:100-106`), rendered by the frontend's `TESSERA_STABLE_ERROR_CODES`, and locked by tests. AD-31/AC name the concept `stale_snapshot`; treating that as the concept name (not a literal code) keeps the contract stable and avoids a cross-stack rename for no functional gain.

**Projection predicate shape.** `AND (:has_project = 0 OR EXISTS (SELECT 1 FROM project_mappings pm WHERE pm.tessera_project_id = :rowid AND pm.provider = m.provider AND COALESCE(pm.native_project,'') = COALESCE(m.native_project,'')))` — the `COALESCE` collapse mirrors the Story 5.1 scope uniqueness index, so a Codex global (`native_project NULL`) matches a `(codex, null)` mapping and vice-versa. Reuses the existing "presence-flag OR predicate" idiom so the no-filter path is unchanged.

**Sidecar narrowing (Q3=A).** Without narrowing, a project-filtered search would report Coverage/Health for sources the user cannot see results from — misleading. Narrowing to the project's mapped `(provider, COALESCE(native_project,''))` set keeps the sidecar consistent with the filtered result set.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: new migration/projection/cursor/sidecar/revision tests pass; `schema_version == "8"` post-migration; the ~20 pre-existing Unix-only failures ignored on Windows (project memory).
- `npm run build` -- expected: TypeScript + Vite clean with the updated `src/api/search.ts` and `src/features/search/Search.tsx`.
- `npm run test:e2e` (Playwright, focused) -- expected: the flipped Tessera-project-filter case (now enabled + keyboard-reachable) passes.

**Manual checks (if Playwright cannot run headless here):**
- Boot the core; create a project + two mappings via `curl` against `127.0.0.1:1420`; run a search with and without `tessera_project` and confirm only mapped-scope records appear; add a mapping between page 1 and page 2 and confirm the page-2 request returns `409 cursor_stale`; confirm `GET /api/sources/inventory` is byte-identical before/after.

## Auto Run Result

Status: done

**Summary:** Implemented Story 5.2 — projection browse/search by Tessera Project over the Search path. Filled the reserved `tessera_project` filter with a read-only `EXISTS` predicate joining `memory_records` to `project_mappings` on `(provider, COALESCE(native_project,''))` (no canonical-row copy — AD-2). Introduced a monotonic `project_mapping_revision` scalar (migration id 8, seeded `0` in `tessera_meta`), bumped inside `with_transaction` only on scope-set-changing ops (add/remove/delete-mapping), and folded it into `current_index_revision()` so any mapping change invalidates every outstanding search **and** browse cursor via `cursor_stale` (HTTP 409; AD-26/AD-31). The search cursor binds `tessera_project` (v3→v4) and `source_status_sidecar` narrows to the project's mapped sources. Browse stays single-source (Q1=A); "browse-by-project" is served by Search with an empty `q` + filter. Stable code `cursor_stale` retained (Q2=A). The React Search view replaces the disabled slot with a live `<select>` fed by `listProjects()`.

**Files changed (17):**
- Rust src: `index/migrations.rs` (id 8), `index/mod.rs` (schema 8), `index/project_store.rs` (revision read/bump), `index/scan_store.rs` (EXISTS predicate + revision fold + scope-set helper), `application/project.rs` (bump on scope-set change), `application/query.rs` (cursor v4 + sidecar narrowing + malformed-id fix + version/length guards), `domain/query.rs` (empty-q carve-out + dedup length gate), `http/server.rs` (doc).
- Rust tests: `search.rs`, `projects_api.rs`, `http_api.rs`, `rebuild.rs`, `scan_pipeline.rs`, `source_registry.rs`.
- Frontend: `api/search.ts`, `features/search/Search.tsx`.
- E2E: `tests/ui/accessibility.spec.ts`.

**Review findings (this pass):** patches applied 6 (all low); deferred 5; rejected 2. Follow-up review recommended: **true** (6 low patches → score 6 ≥ 5). See `## Review Triage Log`.

**Verification performed:**
- `cargo test --manifest-path server/Cargo.toml --test search --test projects_api --test http_api --test rebuild` → search 35, projects_api 19, http_api 46, rebuild 14 — all 0 failed.
- `cargo test --manifest-path server/Cargo.toml --lib query` → 8 passed, 0 failed.
- `npm run build` → TypeScript + Vite clean (236.52 kB).
- Pre-existing ~20 Windows-only Unix test failures unchanged (project memory); no new failures.

**Residual risks:**
- The EXISTS projection predicate is non-sargable on `native_project` (deferred — perf, needs benchmark/generated column); fine on small local DBs today.
- `Search.tsx` fetches projects once on mount; projects mutated elsewhere while Search stays mounted won't refresh the dropdown until remount (deferred UX).
- `effectiveRangeText` may briefly show the raw `proj_<n>` handle in the aria-live status before the project name resolves (deferred a11y edge).
- Playwright does not yet `selectOption` a project and assert result narrowing/aria-live (deferred); the HTTP path (`search_tessera_project_param_narrows_results_over_http`) covers the contract.
- No TS unit-test layer for `buildSearchParams` (no vitest in the locked stack); Playwright is the only UI guard (deferred).
- Untracked `_bmad-output/implementation-artifacts/bmad-dev-auto-result-5-2-project-projection.md` is a stale dev-auto HALT log from the earlier git-blocked run — left in place (residual artifact), not part of this change.
