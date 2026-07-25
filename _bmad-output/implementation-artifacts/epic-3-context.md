# Epic 3 Context: 无查询浏览与记忆结构可视化

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Let Carver explore indexed Agent Memory without guessing a keyword — browse collections by Provider / Tessera Project / Native Project / time / Agent Memory type, then drill down from a Provider through Native Projects into individual memory entries and their original locations, to understand each source's scope, hierarchy, and recent changes. This epic turns the Derived Index into a navigable panorama (user journey UJ-4) and reuses the same Provenance / Coverage / Health surfaces built for search, so browsing and searching present one consistent view of memory.

## Stories

- Story 3.1: BrowsePage query contract & query-less browse entry
- Story 3.2: Dimension grouping & recent-changes browse
- Story 3.3: Memory-structure drill-down navigation & visualization

## Requirements & Constraints

- **FR-16 — Browse Agent Memory collections:** enter from Source Inventory (or a Tessera Project once Epic 5 lands) into a paged list with recent changes and dimension filtering; browse results must reuse the **same** Provenance, Coverage Level, and Source Health fields as search results.
- **FR-17 — Visualize memory structure:** users can move from Provider → Native Project → memory entry → original location; views show recent scan, recent changes, and Source Health without disguising Derived-Index state as source-data state.
- **Empty-collection three-state:** distinguish "not yet scanned", "no indexable Agent Memory", and "Source currently unavailable". The UI must never collapse these into a single "empty".
- **Exclusion contract on browse lists:** raw chat, session/transcript, human-instruction files (`CLAUDE.md`, `AGENTS.md`, rules), and any unconfirmed Source must never appear in a browse list.
- **Explicit non-goals of the first version:** no knowledge graph, no auto-inferred relationships, no AI-generated summaries. Group/list/status views are sufficient.
- **NFR-13 keyboard reachability:** core browse, filter, and open-source operations must be completable by keyboard alone.
- **SM-4 offline:** browse must work with no external network path.

## Technical Decisions

- **AD-23 (A-9) — Browse and Search share one bounded query contract.** Query Service exposes versioned `BrowsePage` / `SearchPage` with a unified `cursor`, `limit`, stable sort, `EmptyState` enum, Coverage Level, and Source Health metadata. **Browse must not bypass Query Service to read index tables directly** — no separate read path that could drift from search's Provenance/Health interpretation.
- **AD-17 / AD-26 / AD-31 — Versioned, bounded, snapshot-bound cursors.** All DTOs carry `api_version`; queries are server-bound `cursor + limit`; cursor binds to active generation + projection revisions + sort key + `record_id`; any revision change returns `stale_snapshot` and the caller restarts pagination from the new snapshot.
- **AD-6 — Canonical identity & Provenance are the browse entry's source of truth.** Records carry `record_id`, `source_id`, `provider`, native id/scope, `origin_locator`, `source_revision/hash`, `parser_version`, `coverage_level`, `observed_at`. Tessera Project is an additional projection only; drill-down by Tessera Project must not overwrite native identity.
- **AD-7 — Separate the states.** Source lifecycle, Health, Coverage Level, scan state, and active generation are distinct fields; the browse UI consumes them as structured status, never as a boolean `connected`.
- **AD-21 (A-17) — Shared accessibility contract.** Inventory, Browse, Search, Health, and Provenance share semantic focus order, keyboard-reachable commands, readable status labels, and EmptyState; visual components are never the only entry point. Acceptance artifact: `tests/ui/accessibility.spec.ts` (Playwright).
- **Code layout:** Query/read ports serve BrowsePage from `application::query`; browse UI lives under `src/features/` (Structural Seed). Result-card / Provenance components are shared with search, not duplicated.

## UX & Interaction Patterns

- **UX-DR7 — Browse & structure visualization:** paged list, grouping by time / Native Project / Agent Memory type, navigation Provider → Native Project → memory entry → original location.
- **UX-DR8 — Keyboard & shared interaction contract:** focus order, keyboard-reachable commands, readable status labels, and EmptyState are shared across Inventory / Browse / Search / Health / Provenance.
- **UJ-4 entry path:** from Source Inventory (or Tessera Project) → browse by time/Native Project/Memory type → open any card's Provenance → locate the original file. The value moment is seeing scope and recent change without guessing a keyword.
- **Drill-down "open original location"** reuses Epic 1's mechanism (server resolves `record_id` → origin locator, re-validates allowlisted root, calls OS to open at the line range); failures show an understandable error plus current Source Health, with no body/credentials in the message.

## Cross-Story Dependencies

- **Hard dependency on Epic 1 / Epic 2:** assumes Query Service, Derived Index, multi-Provider indexed content, Provenance result cards, and the open-original-location path are already in place; Epic 3 adds the query-less browse surface on top.
- **Within epic:** 3.1 (BrowsePage contract + entry) → 3.2 (dimension grouping & recent changes) → 3.3 (drill-down & structure visualization); 3.2 and 3.3 both build on the 3.1 contract.
- **Forward to Epic 5:** the Tessera Project drill-down branch is reserved here and populated once Epic 5 ships explicit Native-Project → Tessera-Project mapping. Do not implement project federation in this epic; preserve the native-project path now.
- **Reuse, do not re-implement:** result cards, Provenance views, EmptyState, and open-original-location flow are shared with Epic 1/2 surfaces via the AD-21 accessibility contract.
