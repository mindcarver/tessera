# Epic 5 Context: Tessera Project 跨 Agent 项目联邦视图

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Let the user create a Tessera Project and explicitly associate multiple Native Projects from Codex and Claude Code into one cross-Agent view, then browse and search that aggregate without ever modifying native identity or source data. Native identity is preserved as-is from Epic 1; this epic adds the explicit mapping layer and the projection query path on top of it.

## Stories

- Story 5.1: Tessera Project creation and Native Project explicit mapping
- Story 5.2: Browse and search by Tessera Project (projection)

## Requirements & Constraints

- **Explicit cross-Agent mapping (FR-5):** A Tessera Project can be created and associated with one or more Native Projects across Codex and Claude Code. Mappings live only in local Tessera state — Provider directories/files are never modified. Users can view, adjust, and remove mappings; removing a mapping never deletes any Agent Memory or Derived Index record.
- **Native identity preserved (FR-4, carried from Epic 1):** Each Agent Memory keeps its Provider-native Native Project as-is; unverifiable directory keys are shown as "unmapped" rather than guessed. Same Native Project's memories remain independently searchable.
- **Projection is read-only:** Projection must not copy canonical records or alter native identity; the project view is derived from the canonical index.
- **Rebuild safety (FR-15 / NFR-10):** Reset Index preserves Source Registry and Tessera Project mappings; rebuild restores stable record identities, Provenance, and existing mappings.
- **Privacy (NFR-2 / NFR-3):** No mappings, projection results, or diagnostics leave the machine; logs omit body/query/credentials.
- **Success metrics:** Provenance stays complete (Provider/Source/Native Project/original location/source status) on every projected result; rebuild recovers mappings and stable record identity.

## Technical Decisions

- **Tessera owns only a projection (AD-2):** Canonical records are the Source's truth; project mapping is an additional, removable layer that never replaces native identity.
- **Mapping cardinality and precedence (AD-27 / A-11):** Within one mapping scope, a Native Project belongs to at most one **active** Tessera Project. Only explicit mappings take effect; unmapped entries are never auto-projected. A Native Project cannot be claimed by multiple active Tessera Projects simultaneously, and explicit mapping never overrides an existing user decision silently.
- **Unknown scopes stay isolated (AD-24):** Native Project / Provider scope is isolated by default. Unknown or ambiguous scopes are never auto-merged; only an explicit Tessera Project mapping forms a federated view, and it never changes native identity.
- **Cursor binding (AD-26):** Every cursor carries the active generation, projection revisions, sort key, and `record_id`.
- **Snapshot covers projection revision (AD-31):** The query snapshot token binds active generation + `project_mapping_revision` + filter/policy revision + sort key. Any revision change makes an old cursor return `stale_snapshot`; the caller must restart pagination from the new snapshot. (See AD-6/AD-15: identity is locator-based, content hash only detects change.)
- **Source rebind only on explicit action (AD-33):** Path/identity change does not auto-merge or copy mappings to a new Candidate; mapping transfer requires explicit rebind.
- **Reset boundary (AD-29):** Reset Index wipes canonical body, FTS, and scan runs but keeps Source Registry and Tessera Project mappings intact.
- **Code location & naming:** Project mapping lives in `domain::project` with a local app-data repository. Domain IDs use the `proj_` prefix; Provider names stay lowercase (`codex`, `claude_code`); structured payloads use versioned JSON/serde with `api_version`.
- **Shared query contract (AD-23):** Project browse/search uses the same `BrowsePage` / `SearchPage` query service (shared cursor/limit/sort/EmptyState/Coverage/Health) as the rest of the app — it does not bypass the Query Service to read index tables directly. The Tessera Project filter slot reserved in Epic 2 (Story 2.4) is filled here.

## UX & Interaction Patterns

- **UX-DR3 (Tessera Project creation + Native Project association):** The PRD-deferred interaction for creating a project and multi-to-one association, and adjusting/removing mappings, is embedded in Story 5.1 as a to-be-decided-in-UX (or dev-stage) decision, with AC capturing the verifiable functional constraints (visual/interaction details to be refined in the UX pass).
- **UX-DR4 (Default scope isolation display):** Provider-native scope preservation and "unknown scope stays unmapped" presentation is surfaced in the project view per FR-4 / AD-24.
- **Keyboard accessibility:** Creation, mapping, and project-scoped browse/search flows must be keyboard-reachable per the shared interaction contract (NFR-13 / AD-21).

## Cross-Story Dependencies

- **Epic 1 / Epic 2:** Provides the multi-Provider native identity, Source Registry, canonical records, and Query Service that this epic projects over.
- **Story 2.4:** Reserves the Tessera Project filter slot that Story 5.2 fills.
- **Epic 4 (Reset Index):** Reset must preserve Tessera Project mappings (AD-29) so rebuild does not undo this epic's user-authored state.
