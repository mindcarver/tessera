---
title: 'Story 3.3: Memory-Structure Drill-Down Navigation & Visualization'
type: 'feature'
created: '2026-07-25'
status: 'done'
review_loop_iteration: 0
followup_review_recommended: true
baseline_revision: '1df6b585b55e8a2fdcc75db1f03e1b95a8d9de59'
final_revision: '6834a14'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-3-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-3-1-browse-page-entry.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-3-2-dimension-grouping-recent-changes.md'
warnings: []
---

<intent-contract>

## Intent

**Problem:** Within a single source's Browse view, the Provider → Native Project hierarchy Carver navigated through is imperceptible: the heading is a flat sub-line, "Back to inventory" is an unrelated button, and Source Health is buried per-card. The cross-view drill-down path (Inventory provider group → single-source Browse → memory entry → open original) already exists from 2.5 + 3.1 + 3.2 + Epic 1, but it is implicit — so FR-17's "understand each source's scope, hierarchy, and status" is not satisfied.

**Approach:** Make the hierarchy explicit as a keyboard-reachable Breadcrumb at the top of Browse (`Sources › <Provider> › <Native Project | Global memory>`), where the Sources segment is the back affordance and the Native Project leaf is honest about Codex's global store. Consolidate the scattered status readouts into one structured hierarchy-status view ("Recent scan first" + Source Health from the existing sidecar). Pure front-end: no server contract change, no new data path — reuses the 3.1 `BrowsePage`, the 3.2 ordering readout, and Epic 1's open-original-location verbatim.

## Boundaries & Constraints

**Always:**
- Browse reuses ONLY the existing `BrowsePage` contract (3.1), the bound `memory_type` filter cursor `b4.` (3.2), the shared `ResultCard` / `EmptyState` / `LoadMore`, and Epic 1's open-original-location. No server change, no DTO change, no new endpoint, no cursor version bump.
- The hierarchy is surfaced as a Breadcrumb (`<nav aria-label="Breadcrumb">` + `<ol>`) with three segments: `Sources` (keyboard-reachable `<button type="button">` → `onBack` to Inventory), `<Provider>` (presentational `<span>` — the Inventory's own provider grouping already IS the provider layer, so this segment has no separate click target), `<Native Project | Global memory>` (leaf, `aria-current="page"`).
- The Native Project leaf is honest: Codex sources (`native_project == null`) show "Global memory" (never a fake project name, never "All projects"); Claude sources show the `native_project` string verbatim (no reverse-mapping to a repo path — that is Epic 5 federation, explicitly out of scope per epic-3-context.md:45).
- A structured hierarchy-status view (`data-testid="browse-structure-status"`) consolidates the "Recent scan first" ordering readout (3.2, scan-recency — never implying content-change tracking, AD-7) AND the current source's Source Health, derived from the existing `sources` sidecar (`SourceQueryStatus[]`) by filtering on the browsed `sourceId`. No new fetch.
- The Breadcrumb and status view are keyboard-reachable (AD-21): Tab reaches the Sources segment, Enter activates back to Inventory; the status view lives inside the existing `aria-live` region so transitions are spoken without moving focus.
- `npm run build` (tsc + vite) stays green; Playwright `tests/ui/accessibility.spec.ts` gains a Breadcrumb case as the AD-21 acceptance artifact.

**Block If:** None.

**Never:**
- Never change the server-side `BrowsePage` / `BrowseRequest` / cursor contract. 3.1 is `done` and 3.2 is `in-review` with the cursor locked at `b4.`; re-opening either is forbidden. No new endpoint, no DTO field, no cursor version bump.
- Never add a Native Project layer that aggregates across sources. Project federation is Epic 5 (AD-24/AD-27), forbidden here (epic-3-context.md:45). The breadcrumb's Native Project segment is per-source scope only.
- Never introduce a per-record generational diff / added-removed "recent changes" data path. "Recent changes" honors 3.2's binding decision (confirmed with Carver): it IS the "Recent scan first" ordering readout (scan recency), nothing more (AD-7 no-disguise).
- Never add group-by / nesting / header rendering inside the single-source browse list (3.2's `Never`, locked). The breadcrumb is navigation chrome above the list, not list grouping.
- Never fake a Codex project. Codex is a global store (`server/src/adapters/codex.rs:188` hard-codes `native_project: None`); the leaf says "Global memory", never an invented project.
- Never make the Breadcrumb the sole entry path or sever the existing back semantics. The Sources segment IS the existing `onBack` action, restated as navigation — one back action, not two.
- Never build the Tessera-Project breadcrumb segment (reserved for Epic 5).
- Never change Search's wire contract, DTO, or tests.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Claude source breadcrumb | Browse entered for a Claude source with `native_project = "<project>"` | Breadcrumb `Sources › Claude Code › <project>`; Sources segment keyboard-reachable; leaf carries `aria-current="page"` | No error |
| Codex source breadcrumb (global) | Browse entered for a Codex source (`native_project == null`) | Breadcrumb `Sources › Codex › Global memory`; leaf honest about the global store, never a fake project | No error |
| Source health present | sidecar carries the browsed source's `SourceQueryStatus` (`available`/`degraded`/`unavailable`) | Hierarchy-status view shows the health alongside "Recent scan first" | No error |
| Keyboard back | user focuses Sources segment and activates Enter / Space | Returns to Source Inventory (same `onBack` semantics as today) | No error |
| Stale snapshot mid-browse | source rescanned mid-pagination → `cursor_stale` (409) | Existing 3.1/3.2 recovery path unchanged; breadcrumb + status view re-render on the fresh snapshot | Existing 409 path |

</intent-contract>

## Code Map

- `src/features/browse/Browse.tsx` — replace the flat `<p>{subheading}</p>` + standalone "Back to inventory" `<button>` with a `<nav aria-label="Breadcrumb">` + `<ol>` (Sources button → `onBack`, Provider span, Native Project / "Global memory" leaf `aria-current="page"`); add a `data-testid="browse-structure-status"` region combining 3.2's "Recent scan first" readout with the current source's Source Health from the existing sidecar.
- `src/App.tsx` — no change. Already passes `sourceId`, `providerLabel`, `nativeProject`, `onBack`; the breadcrumb is internal to Browse.
- `server/**` — no change. All data is already on the wire (`BrowsePage` from 3.1, `SourceQueryStatus` sidecar, the bound cursor from 3.2).
- `tests/ui/accessibility.spec.ts` — add a Browse breadcrumb case (keyboard-enter, read the three segments, activate Sources to return, assert `aria-current`, assert the Global-memory leaf for a Codex source vs the project leaf for a Claude source, read the hierarchy-status view).

## Tasks & Acceptance

**Execution:**
- `src/features/browse/Browse.tsx` -- replace the flat subheading + standalone Back button with a keyboard-reachable Breadcrumb (`<nav aria-label="Breadcrumb">` + `<ol>`: Sources `<button type="button">` → `onBack`, Provider `<span>` presentational, `<Native Project | "Global memory">` leaf `aria-current="page"`); fold 3.2's "Recent scan first" readout and the current source's Source Health (from the existing `sources` sidecar filtered by `sourceId`) into a single `data-testid="browse-structure-status"` region inside the existing `aria-live` area -- makes the Provider → Native Project hierarchy explicit and surfaces view-level status without a new data path.
- `tests/ui/accessibility.spec.ts` -- add a Browse breadcrumb keyboard case (enter from inventory via keyboard, assert the three breadcrumb segments and `aria-current="page"` on the leaf, activate Sources to return to Inventory, assert the "Global memory" leaf for a Codex source vs the project leaf for a Claude source, read the hierarchy-status view) following the existing Browse a11y pattern -- AD-21 acceptance artifact.

**Acceptance Criteria:**
- Given a confirmed source browsed from the Inventory, when the Browse view renders, then a Breadcrumb shows `Sources › <Provider> › <Native Project | Global memory>` with the Sources segment a keyboard-reachable button back to the Inventory and the leaf carrying `aria-current="page"`.
- Given a Codex source (`native_project == null`), when its Browse view renders, then the breadcrumb leaf says "Global memory" (never a fake project name, never "All projects").
- Given a Claude source with `native_project = "<project>"`, when its Browse view renders, then the breadcrumb leaf shows that project string verbatim (no reverse-mapping to a repo path).
- Given the browsed source's `SourceQueryStatus` in the sidecar, when the Browse view renders, then a structured hierarchy-status view shows Source Health alongside the "Recent scan first" ordering readout, without a new fetch.
- Given the Browse view operated by keyboard alone, when the user focuses the Sources segment and activates it, then focus returns to the Source Inventory (asserted in `tests/ui/accessibility.spec.ts`).
- Given any Browse view, when inspecting the breadcrumb and status copy, then no Derived-Index state is disguised as source-data state (AD-7): "Recent scan first" names scan recency, and Codex's global store is named honestly.

## Spec Change Log

## Review Triage Log

### 2026-07-25 — Review pass 1
- intent_gap: 0
- bad_spec: 0
- patch: 13: (high 0, medium 2, low 11)
- defer: 0
- reject: 1
- addressed_findings:
  - `[medium]` `[patch]` P2 (blind-hunter): the Source Health readout read "Loading…" forever when the sidecar arrived WITHOUT the browsed source (a terminal mislabel violating AD-7). State-gated the health copy so "Loading…" is reserved for the genuinely-pending initial fetch; a sidecar that arrived without the browsed source, or an errored/stale read, renders "unknown" instead. (Combined with P3.)
  - `[medium]` `[patch]` P3 (blind-hunter): on the `error` state `state.sources` is optional, so the health line read "Loading…" on a page that had already errored. Folded into P2's state-gating — `error`/`stale` now render "unknown", never "Loading…".
  - `[low]` `[patch]` P1 (blind-hunter + edge-case): `leafLabel = nativeProject ?? "Global memory"` only caught null/undefined; the prop type is `string | null` and the validator accepts `""`, so an empty-string native_project rendered a blank leaf. Changed to `nativeProject || "Global memory"` (all falsy → global).
  - `[low]` `[patch]` P4 (blind-hunter): `aria-current="page"` is site-nav semantics; the breadcrumb leaf is an in-app drill-down location. Changed to `aria-current="location"`; updated the two test assertions.
  - `[low]` `[patch]` P5 (blind-hunter): dead `aria-hidden="false"` on the Provider span (the default; an explicit value suggested an abandoned decision). Dropped the attribute.
  - `[low]` `[patch]` P6 (blind-hunter + verification-gap): the Codex sidecar was mocked `degraded` with a comment claiming a guard against hard-coding "available", but only Claude's "available" was asserted. Added a `toContainText("degraded")` assertion on the Codex branch so a hardcoded status fails loudly.
  - `[low]` `[patch]` P7 (blind-hunter): the "first focusable = Sources segment" contract was repeated in comments but never tested. Added a `sourcesButton` ref + an auto-focus-on-entry `useEffect`, and a `toBeFocused()` assertion on first Browse entry.
  - `[medium]` `[patch]` P8 (blind-hunter): the back affordance changed from "Back to inventory" to a breadcrumb segment labeled only "Sources" — no visible cue for sighted/non-SR users. Added a `← ` glyph inside an `aria-hidden="true"` span so the accessible name stays "Sources" (name-based selectors survive) while sighted users see the back cue.
  - `[low]` `[patch]` P9 (blind-hunter): both the `<h2>Browse memories</h2>` and the results `<ol aria-label="Browse memories">` exposed the same accessible name; SR users heard it twice. Changed the list's `aria-label` to "Browse results"; updated the 4 list selectors in the test.
  - `[low]` `[patch]` P10 (blind-hunter): `.find()` assumed sidecar uniqueness the wire contract does not guarantee. Documented (code comment) that a duplicate `source_id` is a server-side bug the UI tolerates by taking the first occurrence.
  - `[low]` `[patch]` P11 (blind-hunter + edge-case): the static "Recent scan first" readout and the dynamic health line (both `<p role="status">`) were inside the outer `<div aria-live="polite">`, double-wrapping the live regions and re-announcing the static label on every re-render. Split into `renderOrderReadout` (static, OUTSIDE any `aria-live` ancestor, no `role="status"`) and `renderHealthReadout` (dynamic, inside the live region, no inner `role="status"` since the outer region covers it). This removed the single `browse-structure-status` wrapper testid; the test now asserts the two readouts directly. The intent's "one structured hierarchy-status view" is honored at the semantic level (the two readouts together ARE the view), not as one literal container testid.
  - `[low]` `[patch]` P12 (blind-hunter): comments called `onBack` "navigation" while the prop stayed `onBack`, and an a11y comment cited `selectOption` (a Playwright API) as the keyboard mechanism. Fixed the comments to describe the prop as "the back action surfaced as the breadcrumb's Sources segment" and the select as "keyboard-operable by default".
  - `[low]` `[patch]` P13 (edge-case): empty-string `providerLabel` rendered a blank middle breadcrumb segment. Added `providerLabel || "(unknown provider)"` fallback.
- rejected_findings:
  - IA-finding (intent-alignment): "consolidate the per-card Source Health scattering" only half-done (view-level readout added, per-card field in `ResultCard` not removed). Rejected because the intent's `Always` bullet 1 mandates reusing `ResultCard` *verbatim* — removing its per-card field would violate that binding constraint. The permissive reading (add a view-level summary alongside the verbatim-reused per-card detail) is the only reading the intent's `Always` permits; the stricter reading is forbidden by the same intent. The IA auditor also explicitly did not prescribe additional work.

## Design Notes

- **Why a breadcrumb, not a new structure view.** FR-17 names "列表、分组和状态视图" (lists, grouping, status views). Lists = 3.1 Browse; grouping = 2.5 Inventory provider grouping + 3.2 `memory_type` filter; the missing piece is the *navigation* that makes the Provider → Native Project hierarchy perceptible across those views. A breadcrumb is the minimal honest form: it visualizes the cross-view drill-down path that already exists (Inventory provider group → single-source Browse → memory entry → open original) without inventing a parallel structure view, a new data path, or a federation layer. This reads AC1's "Provider → Native Project → 记忆条目 → 打开原始位置" as a *navigation path to make explicit*, not as a single nested view to build — consistent with 3.2 having locked single-source scope (cursor `b4.`) and demoted Native Project to heading context.
- **Why "Global memory" for Codex.** Codex's adapter hard-codes `native_project: None` (`server/src/adapters/codex.rs:188`) because it is a global store with no project segmentation. Inventing a project name would violate AD-7 (no disguise) and the epic's honesty contract. "Global memory" names the situation truthfully. Claude sources show the `native_project` string verbatim — the adapter copies the project directory name without reverse-mapping to a repo path (explicitly Epic 5, `server/src/adapters/claude_code.rs:265-267`), so the breadcrumb does not pretend to know the repo either.
- **Why no server change.** 3.1 (done) ships `BrowsePage` + the `SourceQueryStatus` sidecar; 3.2 (in-review) ships the bound cursor and the "Recent scan first" ordering readout; Epic 1 (done) ships open-original-location. Every datum the breadcrumb and status view needs (`providerLabel`, `nativeProject`, the sidecar's current-source health, the 3.2 ordering readout) is already on the wire. A server change here would either re-open a locked contract or duplicate an existing surface — both forbidden by the Boundaries.
- **Readings selected for the three planning gaps (user authorized "按你的想法来").** These were open intent gaps at step-02; the user authorized the agent to pick. The choices and the rejected alternatives are recorded here so step-04 review can audit them:
  - **Gap 1 (Native Project layer): reading (c).** Native Project is NOT a standalone navigable aggregate; it is a per-source leaf in the breadcrumb, honest about Codex's null. (a)/(b) imply Native Project is a real aggregate layer, which the data model does not support without federation (forbidden). (d) is rejected (violates the federation ban, epic-3-context.md:45).
  - **Gap 2 (recent changes): reading (a).** Honor 3.2's binding decision — "recent changes" IS the "Recent scan first" readout. (b) rejected (conflicts with 3.2's confirmed-with-Carver decision, out of epic scope).
  - **Gap 3 (increment): reading (a).** Breadcrumb + structured status view, pure front-end. (b) rejected (a parallel structure view is over-engineering given Browse already is the list surface); (c) rejected (2.5's provider grouping already is the entry); (d) rejected (zero increment does not satisfy an independent story).

## Verification

**Commands:**
- `npm run build` -- expected: `tsc -b && vite build` clean (no server change, so the build surface is TS-only).
- `cargo test --manifest-path server/Cargo.toml` -- expected: unchanged, run as regression to confirm no accidental contract drift (no server file is touched).

**Manual checks (if no CLI):**
- The Playwright breadcrumb case in `tests/ui/accessibility.spec.ts` ships as the committed AD-21 artifact; CI-validated if not run locally.

## Auto Run Result

Status: done

**Summary:** Implemented Story 3.3 — a keyboard-reachable Breadcrumb (`Sources › <Provider> › <Native Project | Global memory>`) that makes the Provider → Native Project drill-down hierarchy explicit in the single-source Browse view, plus a structured hierarchy-status readout pairing 3.2's "Recent scan first" (scan-recency, AD-7-honest) with the browsed source's Source Health (derived from the existing sidecar). Pure front-end: no server contract, DTO, endpoint, or cursor change. The Native Project leaf is honest about Codex's global store ("Global memory", never a fake project) vs Claude's per-project string verbatim. Honors 3.1's `done` and 3.2's `in-review` locked contracts; federation (Epic 5) and per-record generational diff are explicitly out of scope.

**Files changed:**
- `src/features/browse/Browse.tsx` — replaced the flat subheading + standalone "Back to inventory" button with a `<nav aria-label="Breadcrumb">` + `<ol>` (Sources `<button>` → `onBack`, auto-focused on entry; Provider `<span>`; leaf `<span aria-current="location">`); split the status view into `renderOrderReadout` (static "Recent scan first", outside `aria-live`) + `renderHealthReadout` (dynamic Source Health, state-gated, inside `aria-live`); Codex leaf "Global memory"; empty-string fallbacks for leaf and provider.
- `src/App.tsx` — doc-comment accuracy only (the exit affordance changed to the Breadcrumb Sources segment; prop stays `onBack`).
- `tests/ui/accessibility.spec.ts` — new breadcrumb case (Codex "Global memory" + Claude project leaf, `aria-current="location"`, Sources auto-focus + keyboard back, Codex `degraded` + Claude `available` health assertions proving per-sourceId filtering); updated 3 existing browse tests (list selector `Browse memories` → `Browse results`; back affordance → Breadcrumb Sources segment).

**Review findings:** Review pass 1 — 0 intent_gap, 0 bad_spec, 13 patches applied (2 medium: P2/P3 state-gated health "Loading…"→"unknown", P8 visible `←` back cue; 11 low: P1 `||` falsy leaf, P4 `aria-current="location"`, P5 drop dead `aria-hidden`, P6 Codex `degraded` assertion, P7 Sources auto-focus + assertion, P9 list `aria-label` dedup, P10 `.find` uniqueness comment, P11 split static/dynamic readouts out of nested live regions, P12 comment/code drift, P13 provider empty-string fallback), 1 rejected (IA per-card health consolidation — intent's `Always` mandates `ResultCard` verbatim reuse, forbidding the stricter reading). Patch score 3×2 + 11 = 17 ≥ 5 → follow-up review recommended.

**Follow-up review recommended:** true.

**Verification performed:**
- `npm run build` (`tsc -b && vite build`) — clean (223.86 kB).
- `cargo test --manifest-path server/Cargo.toml` — 16 binaries, 0 failed (regression: no server file touched).
- `npx playwright test` — 13 passed, including the new breadcrumb test and the 3 updated browse tests.
- Matrix Test Audit — all 5 I/O matrix rows covered by passing tests (Claude/Codex breadcrumb, source health, keyboard back, stale-snapshot recovery).

**Residual risks:**
- Playwright tests are CI-validated, not executed in production.
- The health readout shows the raw `status` wire string (`available`/`degraded`/`unavailable`/`unknown`) verbatim, consistent with `ResultCard`'s `health_state` rendering; friendlier copy is a separate future concern.
- P10: a duplicate `source_id` in the sidecar (a server-side bug, not a known production state) is tolerated by `.find()` taking the first occurrence — documented in code.
- P7's auto-focus on Browse entry is a small behavior change (focus now moves to the Sources segment on entry); the Inventory's Browse button retains its own focus model, so Tab order from the Inventory is unaffected.
