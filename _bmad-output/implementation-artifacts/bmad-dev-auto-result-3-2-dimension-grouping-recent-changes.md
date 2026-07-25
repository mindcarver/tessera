---
status: blocked
---

# BMad Dev Auto Result — Story 3.2 (dimension grouping & recent-changes browse)

Status: blocked
Blocking condition: intent gap

## Invocation

`bmad-dev-auto story 3.2` — epic-story path (artifacts use `spec-{slug}.md` convention;
not folder+id dispatch). `spec_file` was resolved to
`{implementation_artifacts}/spec-3-2-dimension-grouping-recent-changes.md` but never
written — planning halted at step-02 (intent could not be resolved to a single
defensible reading).

## Intent under analysis

> As Carver, I want to browse grouped/filtered by **Provider / Native Project /
> Memory type / time**, and see **recent changes**, so that I quickly locate the
> memory slice I care about.
> AC: browse results share the same Provenance / Coverage Level / Source Health
> fields as search (A-23).

(epics.md lines 409–420; FR-16/FR-17 in implementation-readiness-report-2026-07-21.md)

## Verified data-model facts (why the intent splits)

Story 3.1 **locked** `BrowseRequest` to a single confirmed `source_id` (required).
Within that locked single-source scope, the four named dimensions behave as:

| Dimension | Column | Within one source |
|---|---|---|
| Provider | `memory_records.provider` | Constant (one source = one provider). Degenerate. |
| Native Project | `memory_records.native_project` — copied from `source.native_project` per record (`server/src/application/scan.rs:371`) | Constant within a source; NULL for Codex. Degenerate. |
| Time | `memory_records.observed_at` — `unix_seconds_now()` set once per scan (`scan.rs:330`), stamped on every record of that generation | Constant across all records in the active generation. Degenerate at record granularity. |
| Memory type | `memory_records.provider_memory_type` | Genuinely varies (memory / memory_summary / raw_memories / rollout_summary / topic_memory). The only real per-record dimension. |

`memory_records` holds only the **active** generation; prior generations are not
retained on the read path. A true per-record "recent changes" (added/removed/changed
since last scan) would require diffing against a prior generation that `QueryStore`
does not expose — a new data path, not a filter on existing rows.

## Unanswered questions (the gap)

1. **Is browse scope still single-source, or widened to cross-source?**
   - Reading A (keep single-source): only `memory_type` is a real filter; Provider
     / Native Project / time are honest singletons or no-ops.
   - Reading B (widen to cross-source panorama): Provider / Native Project become
     meaningful, but this **breaks the `BrowseRequest` contract Story 3.1 just
     locked and marked `done`** (different DTO, cursor, revision binding, empty-state
     semantics). Cross-source inventory grouping already exists at the *Inventory*
     surface (Story 2.5); browse is currently per-source.
   - Epic 3's dependency note ("3.2 builds on the 3.1 contract") leans toward A;
     FR-17's cross-provider framing and the AC's "group by Provider/Native Project"
     lean toward B. Nothing in 3.2's own text selects between them.

2. **What does "recent changes" mean against this data model?**
   - (a) A filter/sort on `observed_at` within active records — degenerate (all
     records in a generation share one timestamp).
   - (b) A generational-diff view (added/removed/changed since the prior scan) —
     requires exposing prior-generation data not currently in `QueryStore`; a new
     SQL/DTO/data path.
   - (c) A "sort recent-first" re-label of the `observed_at DESC` order Story 3.1
     already ships — essentially a no-op.
   - These produce observably different work (new data path vs none).

3. **"Group by" vs "filter by"?** The AC says both 分组/筛选 (group/filter) and
   可按...分组 (group by). Group-by (render headers / nest) and filter-by (narrow
   the set) are different UI + contract work. Search (Story 2.4) shipped *filters*,
   not groups. The intent does not say which 3.2 means.

## Why this is not resolvable by picking a reading

The three readings produce incompatible `BrowseRequest` shapes, cursors, data
sources, and UIs. Declaring Provider/Native-Project/time filters as degenerate
no-ops (Reading A) is itself a product decision that narrows the epic's stated
promise and should be owned by Carver, not inferred. Per the dev-auto step-02 rule
("do not resolve one by picking a reading"), this halts for a human decision.

## What a human needs to decide

- Confirm browse scope for 3.2: **single-source** (extend 3.1's contract in place)
  or **cross-source** (break/replace the single-source contract).
- Define "recent changes": generational diff (new data path), observed_at filter
  (degenerate), or sort-recent-first label (no-op).
- Clarify whether 3.2 is **group-by** (nesting/headers) or **filter-by**
  (narrowing), or both.

## Context loaded for this run

- `_bmad-output/implementation-artifacts/epic-3-context.md` (valid cache; primary).
- `_bmad-output/implementation-artifacts/spec-3-1-browse-page-entry.md`
  (status: done — previous-story continuity; its locked single-source contract is
  the load-bearing constraint behind this gap).
- Backend survey (browse contract, cursor, SQL, record columns) and frontend
  survey (Browse view, shared components, App view state, a11y test) — distilled
  facts in this run's investigation, not persisted as separate files.

## Auto Run Result

Status: blocked
Blocking condition: intent gap (see "Unanswered questions" above).
