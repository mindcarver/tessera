---
title: "Sprint Change Proposal — Epic 6 Readiness Refinement"
status: approved
date: 2026-07-27
workflow: bmad-correct-course
mode: batch
approval: approved
approved_by: Carver
approved_at: 2026-07-27
scope_classification: moderate
trigger: implementation-readiness-report-2026-07-27
---

# Sprint Change Proposal — Epic 6 Readiness Refinement

## Decision Record

| Decision | Proposed value |
| --- | --- |
| Change trigger | The 2026-07-27 Implementation Readiness assessment found four Major Story-quality blockers in Epic 6. |
| Review mode | Batch |
| Product scope | Preserve the approved read-only multi-Vault Obsidian scope and FR-19 through FR-25. |
| Architecture scope | Preserve final AD-37 through AD-40; no new architecture direction is required. |
| Backlog change | Replace six oversized Epic 6 Stories with twelve bounded Stories. |
| Evidence change | Add explicit UX, note-size policy, benchmark privacy, and real-Obsidian evidence contracts before implementation. |
| Approval state | Approved by Carver on 2026-07-27. Canonical planning edits were authorized and applied. |

## 1. Issue Summary

### 1.1 Trigger

The original 2026-07-26 Correct Course proposal was approved and its product,
architecture, requirements, Epic, and sprint-backlog changes were applied.
The subsequent Implementation Readiness assessment on 2026-07-27 confirmed:

- all 25 Functional Requirements are represented;
- Agent Memory and Obsidian Knowledge remain separate domains;
- the Vault zero-write boundary is explicit;
- final AD-37 through AD-40 satisfy the required architecture gate.

The assessment nevertheless returned `NEEDS WORK` because the six new Epic 6
Stories are too broad or contain acceptance conditions whose scope depends on
later measurements.

### 1.2 Core problem

The current Epic 6 plan asks a Developer to make several independently risky
changes inside one Story and to make design decisions while implementing:

1. Story 6.1 combines persisted `source_kind` compatibility, registry parsing,
   native picker delivery, lifecycle behavior, overlap ownership, and Agent
   regression protection.
2. Story 6.2 combines Knowledge schema/migration/identity, file enumeration,
   parsing, bounded reads, generation safety, zero-write proof, and Agent
   migration safety.
3. Story 6.3 combines Knowledge Inventory with watcher/reconcile/rebind and
   requires a measured cadence before its measurement evidence exists.
4. Story 6.6 conditionally introduces Knowledge FTS if literal search fails,
   turning an acceptance gate into result-dependent implementation.

This is a delivery-decomposition defect, not a change to the approved product
vision.

### 1.3 Evidence

Primary evidence:

- `implementation-readiness-report-2026-07-27.md`
  - FR coverage: 25/25
  - current Major issues: 4
  - missing FRs: 0
  - unmet architecture gates: 0
- `epics.md`
  - Story 6.1 spans lines 571–603
  - Story 6.2 spans lines 604–640
  - Story 6.3 spans lines 641–672
  - Story 6.6 embeds conditional FTS implementation at lines 762–763
- `sprint-change-proposal-2026-07-26.md`
  - requires a measured maximum note size before indexing implementation
  - requires reconcile cadence to follow real measurement
  - requires human-visible Obsidian-open evidence

## 2. Impact Analysis

### 2.1 Epic impact

| Epic | Impact |
| --- | --- |
| Epics 1–5 | Remain `done`; no Story is reopened. |
| Epic 6 | Product goal, FR coverage, architecture, and priority remain unchanged. Replace Stories 6.1–6.6 with Stories 6.1–6.12. |
| Future Epics | None are invalidated or required by this correction. |

Epic 6 remains the next product Epic. Its corrected order is:

```text
UX + readiness decision artifacts
  → 6.1 Safe source-kind upgrade
  → 6.2 Registry discovery
  → 6.3 Vault confirmation / picker / overlap resolution
  → 6.4 Knowledge schema and stable identity
  → 6.5 Read-only Markdown indexing
  → 6.6 Inventory ───────────────┐
  → 6.7 Reconcile / rebind ──────┼→ 6.8 Failure isolation / rebuild
  → 6.9 Browse / search ─────────┼→ 6.10 Provenance / open
                                  └→ 6.11 Measurement decision
                                      → 6.12 Pure acceptance gate
```

### 2.2 Artifact impact

| Artifact | Assessment | Proposed change |
| --- | --- | --- |
| PRD | Aligned | No requirement change. |
| PRD Addendum | Aligned | No supported-artifact or trust-boundary change. |
| Architecture Spine | Aligned and final | Keep AD-37..AD-40 unchanged. |
| SPEC | Aligned | Keep CAP-12..CAP-18 unchanged. |
| Requirements Matrix | 25/25 coverage | Preserve FR/NFR mappings; update only Story references if the matrix later carries Story IDs. |
| `epics.md` | Needs refinement | Replace six Stories with twelve; add explicit evidence and interaction boundaries; fix historical wording drift. |
| Standalone UX | Missing | Add focused Phase C.0 UX contract. |
| Readiness decisions | Missing | Add a planning artifact for note-size policy and evidence/privacy contracts. |
| `sprint-status.yaml` | Six obsolete Epic 6 keys | Replace them with twelve backlog keys after approval. |
| Feature code | Not started | No code change in this workflow. |

### 2.3 Technical impact

The refined Stories do not change the selected technology or architecture.
They make ownership and acceptance boundaries explicit:

- Source Registry compatibility is separate from discovery.
- Registry discovery is separate from user confirmation and OS picker delivery.
- Knowledge schema/identity is separate from note enumeration and indexing.
- Inventory is separate from watcher/reconcile and recovery/rebuild.
- Measurement records a decision; it does not silently implement an
  alternative search engine.
- Final acceptance is verification-only.

## 3. Change Analysis Checklist

### 3.1 Understand the trigger and context

| Item | Status | Finding |
| --- | --- | --- |
| 1.1 Triggering Story | `[x]` | Epic 6 Stories 6.1, 6.2, 6.3, and 6.6 were identified during readiness review before implementation. |
| 1.2 Core problem | `[x]` | Planning-quality defect: oversized Stories and measurement-dependent scope. |
| 1.3 Evidence | `[x]` | Readiness report, exact Epic lines, and approved preimplementation gates provide concrete evidence. |

### 3.2 Epic impact

| Item | Status | Finding |
| --- | --- | --- |
| 2.1 Epic viability | `[x]` | Epic 6 remains viable after decomposition. |
| 2.2 Epic-level change | `[x]` | Preserve scope; replace six Stories with twelve bounded Stories. |
| 2.3 Remaining Epic review | `[x]` | Epics 1–5 remain complete and reusable. |
| 2.4 New/obsolete Epics | `[N/A]` | No new Epic and no obsolete Epic. |
| 2.5 Order/priority | `[x]` | Epic 6 remains next; only its internal critical path changes. |

### 3.3 Artifact conflict analysis

| Item | Status | Finding |
| --- | --- | --- |
| 3.1 PRD conflict | `[N/A]` | FR-19..FR-25 remain correct and complete. |
| 3.2 Architecture conflict | `[N/A]` | AD-37..AD-40 already define the required boundary. |
| 3.3 UX conflict | `[!]` | Embedded UX decisions lack a standalone screen/state/accessibility contract and an explicit Rust-owned picker interaction. |
| 3.4 Other artifacts | `[!]` | Epic, sprint keys, note-size decision, benchmark privacy, and manual Obsidian-open evidence contracts require updates. |

### 3.4 Path-forward evaluation

| Option | Viability | Effort | Risk | Assessment |
| --- | --- | --- | --- | --- |
| Option 1 — Direct adjustment | **Selected** | Medium | Low | Reorganize Epic 6 and add planning/evidence contracts without changing product scope. |
| Option 2 — Rollback | Not viable | High disruption | High | No completed Agent work caused the planning defect. |
| Option 3 — MVP review | Not required | Medium | Medium | The approved read-only Obsidian scope remains bounded and valuable. |

**Recommended path:** Option 1. The correction is **Moderate** because it
requires backlog reorganization and planning artifacts, but no fundamental
product or architecture replan.

## 4. Detailed Change Proposals

### 4.1 Replace the Epic 6 Story set

**Artifact:** `epics.md`

**OLD**

```text
6.1 Discovery + source-kind migration + picker + confirmation + overlap
6.2 Schema + migration + identity + enumerate + parse + index + safety proof
6.3 Inventory + watcher + reconcile + rebind + recovery
6.4 Browse + search + filters
6.5 Provenance + open
6.6 Measurement + conditional FTS implementation + final acceptance
```

**NEW**

```text
6.1 Safe local_knowledge Source Registry upgrade
6.2 Discover registered Obsidian Vault Candidates
6.3 Confirm Vaults through a Rust-owned picker and resolve root overlap
6.4 Add independent Knowledge schema and stable note identity
6.5 Enumerate, parse, and index Markdown with zero Vault writes
6.6 Show Knowledge Inventory and truthful health
6.7 Reconcile Vault changes and handle explicit rebind
6.8 Isolate Vault failures and rebuild Knowledge independently
6.9 Browse, keyword-search, and filter across Vaults
6.10 Show Knowledge Provenance and open the note in Obsidian
6.11 Measure real-corpus performance and record gate decisions
6.12 Run the pure multi-Vault acceptance gate
```

**Rationale:** Each Story owns one reviewable outcome. Decisions and evidence
are produced before a dependent Story becomes `ready-for-dev`.

### 4.2 Proposed Story boundaries

#### Story 6.1 — Safe local_knowledge Source Registry upgrade

**User value**

> As Carver upgrading an existing Tessera installation, I want existing Agent
> Sources to remain stable while Knowledge becomes a valid separate source
> kind, so that Obsidian support cannot corrupt my current federation.

**Acceptance boundary**

- Additive persistence accepts `local_knowledge`.
- Unknown persisted `source_kind` fails closed with a safe corruption error.
- Existing Agent Source IDs, fingerprints, lifecycle state, records, mappings,
  and queries remain byte-for-byte or semantically unchanged as applicable.
- Source-kind dispatch cannot route Knowledge through `ProviderAdapter`.
- Migration rollback preserves the last usable Agent index and registry.

**Traceability:** FR-19, FR-25; NFR-1, NFR-6, NFR-8..NFR-10, NFR-14.

#### Story 6.2 — Discover registered Obsidian Vault Candidates

**User value**

> As Carver, I want Tessera to discover registered Vaults without reading note
> content, so that I can see what is available before granting access.

**Acceptance boundary**

- Read only OS-specific Obsidian registry metadata.
- Produce deterministic Candidates with Vault ID/name, canonical root,
  discovery basis, provider, and coverage.
- Same-name Vaults remain distinct.
- Missing, corrupt, and unknown registry shapes produce visible diagnostics and
  never appear as an empty Vault set.
- Discovery reads no note body and never blocks Agent Memory startup.
- Registry fixtures cover observed and unsupported shapes.

**Traceability:** FR-19; AD-37.

#### Story 6.3 — Confirm Vaults through a Rust-owned picker and resolve overlap

**User value**

> As Carver, I want to confirm only intended existing Vaults and resolve
> conflicting roots explicitly, so that Tessera never widens its read boundary.

**Acceptance boundary**

- The browser sends only an action request to open a picker; it sends no path,
  URI, browser directory handle, or filesystem token.
- A Rust-owned OS-dialog adapter returns a validated existing Obsidian Vault
  Candidate or cancellation.
- Confirm/reject/disable decisions persist and survive restart.
- Same-name different-root Vaults remain independent.
- Nested or overlapping roots show both conflicting roots and block the second
  confirmation until the user keeps one root unconfirmed/disabled.
- No free-form path input or HOME-wide scan exists.

**Traceability:** FR-19, FR-20; NFR-5, NFR-6.

#### Story 6.4 — Add independent Knowledge schema and stable note identity

**User value**

> As Carver, I want Obsidian notes represented independently from Agent Memory,
> so that provenance and future rebuilds remain trustworthy.

**Acceptance boundary**

- Add `knowledge_records`, Vault metadata, Knowledge parser version, and
  additive migration history.
- Use `krec_` identity based on Source + normalized Vault-relative path +
  `unit_kind=note`.
- Do not reuse `memory_records`, `ProviderMemoryType`, `native_project`, or
  Agent record fingerprints.
- Rename/move changes locator and identity; no fuzzy merge is introduced.
- Migration success/failure preserves Agent records, Sources, mappings, and the
  last usable Agent index.

**Traceability:** FR-21, FR-25; AD-19, AD-38.

#### Story 6.5 — Enumerate, parse, and index Markdown with zero Vault writes

**Precondition**

`obsidian-knowledge-readiness-decisions-2026-07-27.md` is approved and contains
an exact `max_note_bytes` value derived from stat-only corpus measurement and a
security rationale.

**User value**

> As Carver, I want supported Vault Markdown indexed without modification, so
> that I can query my knowledge while the Vault remains authoritative.

**Acceptance boundary**

- Include regular `.md` notes under allowed non-hidden paths.
- Exclude `.obsidian/**`, dot paths, `.git/**`, trash, Canvas, attachments,
  binaries, plugin data, symlink directories, and root-escaping aliases.
- Enforce `max_note_bytes` before allocating/reading a note body.
- Oversized notes receive safe diagnostics and never replace last-success data.
- File-level records include relative locator, source revision, parser version,
  modified time, and Knowledge Provenance.
- Generation/fencing/manifest validation remains atomic.
- Success, failure, cancellation, retry, and drift fixtures prove zero Tessera
  change to Vault paths, bytes, sizes, and mtimes.

**Traceability:** FR-21; NFR-1, NFR-9, NFR-12, NFR-14.

#### Story 6.6 — Show Knowledge Inventory and truthful health

**User value**

> As Carver, I want one truthful card per Vault, so that I know the indexed
> scope, freshness, and reliability before searching.

**Acceptance boundary**

- Show source kind, Vault name/ID, path, coverage, health, supported Markdown
  count, last success, stale state, and latest safe error.
- Counts exclude every out-of-matrix artifact.
- Empty, not-indexed, disabled, degraded, and error states remain distinct.
- Inventory actions and status are keyboard reachable and screen-reader
  announced.
- One Vault state change does not alter another Vault or Agent Source.

**Traceability:** FR-20; NFR-8, NFR-13.

#### Story 6.7 — Reconcile Vault changes and handle explicit rebind

**User value**

> As Carver, I want supported note changes refreshed without scan loops or
> silent root changes, so that the Knowledge Index stays current and bounded.

**Acceptance boundary**

- Only in-matrix Markdown events create dirty hints.
- `.obsidian/**`, plugin, attachment, and excluded events are filtered before
  scheduling.
- Burst saves coalesce and disabled Sources do not reconcile.
- The no-op path is deterministic and bounded; cadence is configurable and is
  not declared a performance success until Story 6.11 measures it.
- Move, permission, or identity change degrades the old Source and produces a
  Candidate requiring explicit rebind.
- Watcher events never directly mutate canonical records.

**Traceability:** FR-25; AD-8, AD-38; NFR-12, NFR-14.

#### Story 6.8 — Isolate Vault failures and rebuild Knowledge independently

**User value**

> As Carver, I want one broken Vault isolated and Knowledge rebuilds separated
> from Agent data, so that other sources remain usable and recoverable.

**Acceptance boundary**

- One Vault failure leaves other Vaults and Agent Memory searchable/browsable.
- The affected Vault retains last-success results marked stale.
- Knowledge reset/rebuild clears only Knowledge records/query index/scan state.
- Source Registry, Vault metadata, Agent records/index, and Tessera Project
  mappings remain intact.
- Failed/cancelled rebuild preserves source files and the previous visible
  generation.

**Traceability:** FR-25; NFR-8..NFR-10, NFR-14.

#### Story 6.9 — Browse, keyword-search, and filter across Vaults

**User value**

> As Carver, I want to browse and search all or selected Vaults with visible
> scope, so that I can find knowledge without switching Vaults.

**Acceptance boundary**

- Browse supports all confirmed Vaults, one Vault, and
  Vault → folder → note navigation.
- Search defaults to confirmed Obsidian Vaults only; Agent Memory is not
  implicitly included.
- Filters support Vault/Source, folder prefix, and absolute modified-time
  threshold with visible effective scope.
- Search/Browse share stable ordering, cursor, EmptyState, coverage, health,
  and Knowledge Provenance.
- Cursor binds generation and policy/filter revisions.
- Chinese two/three-character and operator-like literal queries are tested.
- Tag/property/backlink/semantic filters are absent rather than simulated.

**Traceability:** FR-22, FR-23; AD-39.

#### Story 6.10 — Show Knowledge Provenance and open the note in Obsidian

**User value**

> As Carver, I want exact Vault/path provenance and a safe Open in Obsidian
> action, so that I can verify and continue in the original note.

**Acceptance boundary**

- Result cards show Knowledge domain, Vault, Source, relative path, derived
  title/snippet, source modification time, observed time, coverage, and health.
- Browser submits only a trusted `krec_` ID.
- Rust revalidates active membership and containment and constructs only
  encoded `obsidian://open`.
- Write-capable actions/parameters are structurally unrepresentable.
- Missing/moved targets, same-name Vault ambiguity, missing URI handler, and
  dispatch failure return safe errors without false success.
- Automated dispatch evidence and human-visible correct-note evidence remain
  separate.

**Traceability:** FR-23, FR-24; AD-40; NFR-5..NFR-7, NFR-14.

#### Story 6.11 — Measure real-corpus performance and record gate decisions

**User value**

> As Carver, I want aggregate performance evidence from my real Vault scale, so
> that acceptance thresholds are based on reality rather than guesses.

**Acceptance boundary**

- Record cold scan, no-op reconcile, single-note freshness, query P50/P95, RSS,
  index size, file descriptors, and thread count.
- Store aggregate results and approved thresholds in
  `tests/benchmarks/knowledge-index.json`.
- Committed evidence contains no note bodies, private filenames, Vault paths,
  registry payloads, or search text.
- Record an explicit decision for reconcile cadence and literal search.
- If literal search or another metric fails the proposed gate, create a named
  remediation Story; this Story does not implement FTS or another optimization.
- Remeasurement uses the same bounded evidence schema.

**Traceability:** NFR-11, NFR-12; SM-12.

#### Story 6.12 — Run the pure multi-Vault acceptance gate

**Precondition**

Story 6.11 has an approved passing gate, including any required remediation
Story and remeasurement.

**User value**

> As Carver, I want the complete feature proven against real multi-Vault use,
> so that I can trust it before normal use.

**Acceptance boundary**

- Verify the approved 0/1/6 Vault, corrupt-registry, duplicate/same-name/nested
  root, move, permission-loss, rebind, traversal, symlink, drift, oversized
  note, failure-isolation, offline, keyboard, stale-data, and rebuild matrix.
- Verify cross-Vault browse/search/filter and exact Provenance.
- Record human-visible correct-Vault/correct-note evidence at
  `_bmad-output/test-artifacts/obsidian-open-e2e.md`.
- Distinguish Tessera-originated mutation from concurrent Obsidian/sync-tool
  changes.
- This Story only passes or fails. It contains no fallback implementation,
  schema change, search-engine migration, or hidden remediation.

**Traceability:** FR-19..FR-25; NFR-2, NFR-8..NFR-14; SM-8..SM-12.

### 4.3 Add a focused UX contract

**NEW artifact**

`_bmad-output/planning-artifacts/ux-obsidian-knowledge-2026-07-27.md`

The contract will define:

- separate top-level Agent Memory and Obsidian Knowledge destinations;
- Vault onboarding, registry-error, picker-cancel, confirm/reject/disable, and
  root-overlap resolution states;
- the Rust-owned picker action boundary;
- Knowledge Inventory loading/empty/degraded/stale/error states;
- Vault → folder → note navigation and visible search/filter scope;
- Provenance and truthful dispatch/error feedback for Open in Obsidian;
- keyboard order, focus restoration, live-region announcements, contrast,
  200% zoom/reflow, and reduced-motion behavior;
- explicit distinction between automated URI dispatch and human-visible open
  evidence.

**Rationale:** Removes screen-state and accessibility guesswork without changing
the approved product scope.

### 4.4 Add readiness decision and evidence contracts

**NEW artifact**

`_bmad-output/planning-artifacts/obsidian-knowledge-readiness-decisions-2026-07-27.md`

It will contain:

1. a stat-only real-Vault size distribution with no body reads and no committed
   private path/name data;
2. an exact `max_note_bytes` value and rejection behavior approved before Story
   6.5 becomes `ready-for-dev`;
3. the aggregate-only schema and privacy rules for
   `tests/benchmarks/knowledge-index.json`;
4. the manual operator, pass/fail fields, and redaction rules for
   `_bmad-output/test-artifacts/obsidian-open-e2e.md`;
5. the rule that failed measurement creates a separate remediation Story and
   never expands Story 6.11 or 6.12.

### 4.5 Update sprint status

**Artifact:** `_bmad-output/implementation-artifacts/sprint-status.yaml`

**OLD**

```yaml
  6-1-obsidian-vault-discovery-confirmation: backlog
  6-2-obsidian-readonly-knowledge-index: backlog
  6-3-obsidian-inventory-health-reconcile: backlog
  6-4-cross-vault-browse-search-filters: backlog
  6-5-obsidian-provenance-open-original: backlog
  6-6-obsidian-multi-vault-acceptance-gate: backlog
```

**NEW**

```yaml
  6-1-safe-local-knowledge-source-kind-upgrade: backlog
  6-2-obsidian-vault-registry-discovery: backlog
  6-3-obsidian-vault-confirmation-picker-overlap: backlog
  6-4-knowledge-schema-record-identity: backlog
  6-5-readonly-markdown-indexing: backlog
  6-6-knowledge-inventory-health: backlog
  6-7-vault-watcher-reconcile-rebind: backlog
  6-8-vault-failure-isolation-rebuild: backlog
  6-9-cross-vault-browse-search-filters: backlog
  6-10-knowledge-provenance-open-obsidian: backlog
  6-11-knowledge-performance-gate-decisions: backlog
  6-12-multi-vault-acceptance-gate: backlog
```

`epic-6` remains `backlog`. Every Epic 1–5 status line remains byte-for-byte
unchanged.

### 4.6 Clean historical planning drift

These are documentation corrections only:

1. Story 1.1 title/user statement: replace “desktop application shell” with
   “local Web application shell”.
2. Epic 2 dependency prose: replace obsolete `IPC` with versioned loopback HTTP
   Query Service/API.
3. Story 2.4: remove the forward promise that Epic 5 will later fill a filter;
   state that Tessera Project filtering is delivered by Story 5.2.

Completed implementation and status remain unchanged.

## 5. Implementation Handoff

### 5.1 Classification

**Moderate correction:** Product and architecture remain approved. Product
Owner/Developer ownership is required for backlog reorganization; UX and
planning-evidence owners must complete the two preimplementation artifacts.

### 5.2 Responsibilities

| Recipient | Responsibility |
| --- | --- |
| Product Owner | Approve the twelve-Story decomposition and sprint keys. |
| UX owner | Produce the focused Obsidian Knowledge UX contract. |
| Planning/evidence owner | Measure metadata only, lock `max_note_bytes`, and define benchmark/manual evidence schemas. |
| Developer | Implement only after the corrected Story is created and readiness is `READY`; use feature branch + PR. |
| Test/verification owner | Enforce zero-write, privacy-safe benchmark, human-visible open, and pure final-gate contracts. |

### 5.3 Success criteria for this correction

The correction is complete when:

1. the proposal is explicitly approved;
2. `epics.md` contains Stories 6.1–6.12 with no future-Epic dependency;
3. FR-19..FR-25 remain fully covered;
4. the two new planning artifacts exist and contain the required decisions;
5. `sprint-status.yaml` has exactly the twelve approved backlog keys;
6. Epic 1–5 statuses are unchanged;
7. Story 6.11 contains measurement/decision only;
8. Story 6.12 contains verification only;
9. formatting and cross-artifact checks pass;
10. Implementation Readiness is rerun before `create-story`.

## 6. Approval and Routing

This approved proposal does not authorize feature-code implementation.

Applied:

1. the Epic, UX, readiness-decision, historical wording, and sprint-key edits;
2. FR coverage, Story structure, dependency direction, YAML, and whitespace
   validation.

Next gate:

1. rerun `bmad-check-implementation-readiness`;
2. proceed to `create-story` only if the result is `READY`.

## 7. Workflow Execution Log

| Date | Workflow | Event | Result |
| --- | --- | --- | --- |
| 2026-07-27 | `bmad-correct-course` | Trigger and Batch mode confirmed | Readiness blockers accepted as the correction trigger; no product-scope change. |
| 2026-07-27 | `bmad-correct-course` | Checklist and impact analysis | Direct adjustment selected; PRD/Architecture/SPEC remain aligned. |
| 2026-07-27 | `bmad-correct-course` | Batch edit proposal drafted | Six oversized Stories proposed to become twelve bounded Stories with explicit UX and evidence contracts. |
| 2026-07-27 | `bmad-correct-course` | Approval | Approved by Carver (`C`). |
| 2026-07-27 | `bmad-correct-course` | Canonical planning edits | Epic 6 decomposed into twelve Stories; UX/readiness contracts and sprint keys applied. |
| 2026-07-27 | `bmad-correct-course` | Validation and handoff | YAML, 35-Story structure, 12-Story Epic 6 sequence, FR-1..FR-25 map, old-key removal, frontmatter, and whitespace checks passed. Moderate correction routed to Product Owner / Developer; Implementation Readiness remains the next gate. |
