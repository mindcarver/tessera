---
title: 'Knowledge Search: cross-vault keyword search'
type: 'feature'
created: '2026-07-28'
status: 'done'
review_loop_iteration: 0
baseline_commit: 'd418597'
context:
  - '{project-root}/AGENTS.md'
  - '{project-root}/_bmad-output/planning-artifacts/epics.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** The user can browse notes within a single Vault, but cannot keyword-search across all confirmed Vaults at once — the core "find knowledge without switching Vaults manually" value of the Obsidian integration (FR-22).

**Approach:** Add a Knowledge Search surface that runs `instr()` substring matching over `knowledge_records.title || body` across all confirmed Vaults with a usable active generation, with optional vault/folder-prefix/modified-time filters, returning paginated results with provenance. Mirrors the Agent-Memory search SQL pattern but reads the independent `knowledge_records` table (AD-19). Reuses the same `record_id`-cursor pagination approach already proven in Knowledge Browse.

## Boundaries & Constraints

**Always:**
- Search defaults to every non-disabled confirmed Obsidian Vault with a usable current or stale last-success generation (FR-22). Agent-Memory records are NEVER searched (AD-19 domain separation).
- Read-only: search reads `knowledge_records` only, never writes Vault files (NFR-14).
- Substring match via `instr(title || char(10) || body, ?1) > 0` — no FTS5, no external model (NFR-2), consistent with the Agent-Memory search.
- Results carry Knowledge-domain provenance: Vault name (derived from source_registry path), Vault-relative path, derived title/excerpt, source modification time, observed time, coverage, health. No Agent-Memory-specific fields (`native_project`, `provider_memory_type`).
- Honest empty states: `no_match` / `not_indexed` / `source_unavailable` (mirrors SearchEmptyState).

**Ask First:** None.

**Never:**
- Do NOT reuse the Agent-Memory `QueryStore::search_records` or its cursor envelope (`v4.`) — Knowledge search gets its own independent `ks.` cursor.
- Do NOT mix Agent-Memory results into Knowledge search results (UX-DR9 domain separation).
- Do NOT implement FTS5, semantic search, tag/property/backlink filters (FR-22 out-of-scope facets).
- Do NOT call external models or remote search (NFR-2).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Keyword matches across 2 vaults | `q=foo`, 2 confirmed+scanned vaults with matches | Results from both vaults, interleaved by relevance (title-match first, then recency) | N/A |
| Filter to single vault | `q=foo&source=src_3` | Only vault src_3's matching notes | N/A |
| Folder-prefix filter | `q=foo&folder=Notes/sub` | Only notes whose Vault-relative path starts with `Notes/sub` | N/A |
| Modified-time filter | `q=foo&since=1780000000` | Only notes with observed_at >= threshold | N/A |
| No matches | `q=zzz`, vaults indexed | Empty results + `no_match` empty state | N/A |
| Vault not scanned | `q=foo`, vault confirmed but never scanned | Empty results + `not_indexed` empty state | N/A |
| Vault scan failed | `q=foo`, vault's latest run failed | Empty results + `source_unavailable` empty state | N/A |
| Pagination | `q=foo`, >limit matches | First page of `limit` results + `next_cursor`; second page appends | N/A |
| Empty query | `q=` (empty) | `bad_request` 400 | Error envelope |

</frozen-after-approval>

## Code Map

- `server/src/index/scan_store.rs` -- add `search_knowledge_records` method (instr-based SQL over knowledge_records, cursor + filter predicates); reuse `knowledge_excerpt`
- `server/src/application/query.rs` -- add `search_knowledge` orchestrator + `KnowledgeSearchPage`/`KnowledgeSearchResult`/`KnowledgeSearchEmptyState` DTOs; `ks.<hex>` cursor
- `server/src/http/mod.rs` -- add `search_knowledge` handler
- `server/src/http/server.rs` -- add `GET /api/knowledge/search` route + `parse_knowledge_search_query`
- `src/api/obsidian.ts` -- add `searchKnowledge` client + DTOs
- `src/features/obsidian/Obsidian.tsx` -- add Search input + results view

## Tasks & Acceptance

**Execution:**
- [x] `server/src/index/scan_store.rs` -- add `search_knowledge_records(source_rowids, query, limit, cursor_key, folder_prefix, since)` returning `(Vec<KnowledgeRecordRow>, bool has_more)` with instr-based SQL over knowledge_records joined to active generation, ORDER BY title-match-rank then native_locator then record_id -- the core search query
- [x] `server/src/application/query.rs` -- add `search_knowledge(registry, conn, query, limit, cursor, source_filter, folder_prefix, since)` returning `KnowledgeSearchPage`; encode `ks.<record_id>` cursor; compute honest empty state on page 1
- [x] `server/src/http/mod.rs` + `server/src/http/server.rs` -- add `GET /api/knowledge/search?q=&limit=&cursor=&source=&folder=&since=` endpoint with `parse_knowledge_search_query`; empty `q` → bad_request
- [x] `server/tests/knowledge_scan.rs` -- add search integration tests: cross-vault match, vault filter, folder-prefix filter, no-match empty state, pagination
- [x] `src/api/obsidian.ts` -- add `searchKnowledge(query, limit, cursor?, source?, folder?, since?)` client with shape validation
- [x] `src/features/obsidian/Obsidian.tsx` -- add search input + results list (reuses note-card rendering from Browse); results show which vault each note belongs to

**Acceptance Criteria:**
- Given 2 confirmed+scanned vaults with notes containing "foo", when searching `q=foo`, then results from both vaults appear, each labeled with its vault name.
- Given a single-vault filter `source=src_3`, when searching, then only that vault's notes appear.
- Given `q=zzz` with indexed vaults, when searching, then empty results with `no_match` state.
- Given a confirmed-but-never-scanned vault, when searching, then `not_indexed` empty state.
- Given more than `limit` matches, when searching page 1 then page 2 via cursor, then all matches are returned across pages without duplicates.
- Given empty `q`, when searching, then HTTP 400 `bad_request`.

## Design Notes

The search SQL mirrors the Agent-Memory `search_records` pattern but simplified for Knowledge:
- No `provider_memory_type` or `native_project` filters (Obsidian Vaults have no Agent-Memory project/type).
- Folder-prefix is a new `LIKE ?V || '%'` predicate on `native_locator` (Vault-relative path), using the same `?N = 0 OR ...` idiom.
- Cursor is a simple `ks.<record_id>` opaque string (the last result's record_id), consistent with Knowledge Browse's `kb.` cursor. The ORDER BY is `(title_match_rank, native_locator, record_id)` so the cursor predicate is a lexicographic strictly-after on record_id within the same title-match tier.
- Empty-state derivation mirrors `knowledge_browse_empty_state` but checks all confirmed Knowledge sources (not just one).

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- all suites green, new knowledge_scan search tests pass
- `REAL_VAULTS=1 cargo test --manifest-path server/Cargo.toml --test obsidian_real_e2e -- --nocapture` -- real-vault scan + search smoke (optional)
- `npm run build` -- frontend compiles

## Suggested Review Order

**Search query + cursor orchestration**

- Entry point: search_knowledge orchestrator — scope hash, cursor decode/encode, empty state
  [`query.rs:800`](../../server/src/application/query.rs#L800)

- Core SQL: instr() substring match, 3-key cursor predicate, folder LIKE with ESCAPE
  [`scan_store.rs:956`](../../server/src/index/scan_store.rs#L956)

- HTTP route + query param parser (percent-decode, limit/since, bad_request)
  [`server.rs:353`](../../server/src/http/server.rs#L353)

**Review hardening**

- Cursor \x1f delimiter + scope hash (filter-swap detection)
  [`query.rs:825`](../../server/src/application/query.rs#L825)

- LIKE wildcard escaping (folder_prefix % and _ are literal)
  [`scan_store.rs:988`](../../server/src/index/scan_store.rs#L988)

**Frontend**

- Search input + results view with vault-name labels + pagination
  [`Obsidian.tsx:248`](../../src/features/obsidian/Obsidian.tsx#L248)

- Typed API client with envelope validation
  [`obsidian.ts:375`](../../src/api/obsidian.ts#L375)

**Tests**

- 12 search tests: cross-vault, filters, empty states, pagination, LIKE escaping, title_match
  [`knowledge_scan.rs:282`](../../server/tests/knowledge_scan.rs#L282)
