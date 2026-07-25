# Epic 2 Context: 跨 Agent 联邦（Claude Code + 跨 Provider 搜索与全景 Inventory）

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Bring Claude Code on board as the second Agent Memory provider alongside Codex, so the Source Inventory shows both providers' scope/coverage/health in one panorama, a single query fans out across Codex + Claude Code, results are filterable across providers with a visible effective range, and each result's origin is comparable side-by-side. This epic delivers Tessera's core structural differentiation: continuous read-only federation over multiple Agents' native memory without migrating, taking over, or rewriting any of them.

## Stories

- Story 2.1: Claude Code Candidate Source discovery & confirmation
- Story 2.2: Claude Code memory parsing, boundary restriction & read-only indexing
- Story 2.3: Cross-provider keyword search & provenance comparison
- Story 2.4: Cross-provider combined filtering & range visibility
- Story 2.5: Multi-provider Source Inventory panorama & cross-source health

## Requirements & Constraints

- Claude Code must integrate through the **same adapter contract** that Epic 1 locks down on Codex; it is the second instance of an already-fixed contract, not a new design.
- Default Claude Code roots: official `~/.claude/projects/<project>/memory/` plus any user-configured `autoMemoryDirectory`; respect `CLAUDE_CONFIG_DIR`.
- **Supported Artifact Matrix for Claude Code:** ingest only auto-generated `MEMORY.md` and topic Markdown under a project memory dir. Explicitly reject `CLAUDE.md`, `AGENTS.md`, `.claude/rules`, session/transcript, and any manually added directory. Unknown files surface as `unsupported_artifact` diagnostics, never indexed.
- Cross-provider search defaults to **all healthy, successfully indexed Confirmed Sources**; queries must not call external models or remote search services (privacy / local-only).
- A single Source being unavailable must not break the global query — its results are marked unavailable while other Sources return normally (foundation of connector failure isolation; full stale-generation handling lands in Epic 4).
- Coverage Level and Source Health are shown **per Source** and never disguised: counts are displayed as complete only when the adapter can fully enumerate; `search_only` / `existence_only` / `unsupported` never display as "fully synced". Health changes never delete the user's confirmation.
- Empty results must distinguish three states across providers: "genuinely no match", "Source not indexed", "Source currently unavailable".
- Zero source mutation: scanning must not change Claude Code files' set/content/size/mtime.
- Claude Code's five-class adapter contract tests (fixture contract, zero-source-mutation, parser-version, reconcile-recovery, capability-honesty) must pass before it is enabled in the default build.
- Performance regression gate applies: adding Claude Code must be reported against the same Phase 0 fixture (`tests/benchmarks/memory-index.json`) and pass the gate.

## Technical Decisions

- Provider name is the stable lowercase ID `claude_code` (Codex is `codex`); domain IDs keep the opaque prefixed convention (`src_`, `rec_`).
- Adapter contract lives at `server/src/domain/ports/provider_adapter.rs`; Claude fixtures live at `server/tests/fixtures/providers/claude_code`. Adapter must declare `discover` / `enumerate` / `search` / `watch` / `stable_native_ids` / `coverage_level` and emit the normalized canonical envelope (`unit_kind`, `native_unit_id`, normalized `native_locator`, title/body, scope, `source_revision`, `parser_version`).
- **Source identity & registry reuse:** discovery produces only Candidate metadata; confirmation runs through the same core canonicalization + allowlisted root + persistent `source_id` + versioned fingerprint (`root-fingerprint/v1`) as Codex. Claude and Codex Sources coexist in one Source Registry; confirmation state survives restart; path changes keep the old Source as degraded and produce a new Candidate (no auto-merge).
- **Canonical record identity:** `record_id = source_id + provider + native locator + unit kind`; content hash detects change, parser version triggers reparse, neither rewrites identity. Claude records carry `provider=claude_code`, topic/heading-based `native_unit_id`, and fall back to file-level unit when stable splitting is impossible.
- **Native Project mapping is intentionally not reverse-solved here:** the Claude `<project>` directory key → real repo path is not a stable public protocol. Preserve the original key as Native Project, show as unmapped when unverifiable, never guess. Cross-project federation via Tessera Project is Epic 5.
- Reuse Epic 1's atomic generational pipeline (staging → CAS commit with durable monotonic fencing token; `dirty_after_validation` generations never activate) — Claude scanning inherits this unchanged.
- Transport stays loopback-only HTTP with `api_version`-versioned DTOs; queries are server-bound `cursor + limit` and cursors bind to active generation + projection revisions (return `stale_snapshot` on revision change).

## UX & Interaction Patterns

- **Multi-provider Source Inventory:** structured status cards showing Provider, path, Native Project, Coverage Level, Source Health (`unknown`/`healthy`/`degraded`/`error`), last successful scan time, record count, last error; honest EmptyState. One Source failing must not affect another's display.
- **Combined filtering range visibility:** when Carver stacks filters (Provider + Memory type + time, etc.), the UI must show the currently effective range (e.g. "Codex + Claude Code, type=MEMORY, last 7d"); clearing filters restores the full Confirmed Source scope. Native Project filter works across providers. The Tessera Project filter slot is **reserved but not populated** here (filled by Epic 5).
- **Result cards & Provenance:** each cross-source result shows Provider + full Provenance (source, native project, original locator, update time, Coverage Level, Source Health), enabling side-by-side comparison of what each Agent remembered for the same query. Inferred titles or project mappings must not masquerade as Provider-native facts.
- Shared accessibility contract (focus order, keyboard-reachable discover/search/filter/open, readable status labels, EmptyState); acceptance artifact `tests/ui/accessibility.spec.ts`.

## Cross-Story Dependencies

- **Hard dependency on Epic 1:** this epic assumes the adapter contract, Derived Index, Query Service, atomic generational pipeline, Source Registry, and loopback HTTP are already in place and Codex-real-format parsing has locked the contract. Epic 2 is the risk-downhill slice after that lock-in.
- Within the epic: 2.1 (discovery/confirm) → 2.2 (parse/index) → 2.3 / 2.4 / 2.5 (cross-source search, filter, inventory) — the later three depend on 2.2 having produced indexed Claude records.
- Forward to Epic 5: the reserved Tessera Project filter slot and the explicit Native-Project→Tessera-Project projection are filled there; do not implement project federation in this epic.
- Forward to Epic 4: full failure isolation, stale previous-success results, and index rebuild are owned there — Epic 2 only needs the foundational shape (one Source down does not break the query).
