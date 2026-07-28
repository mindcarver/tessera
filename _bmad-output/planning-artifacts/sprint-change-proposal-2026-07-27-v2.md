---
title: "Sprint Change Proposal — Epic 6 Cross-Artifact Readiness Closure"
status: approved
date: 2026-07-27
workflow: bmad-correct-course
mode: batch
approval: approved
approved_by: Carver
approved_at: 2026-07-27
scope_classification: minor
trigger: implementation-readiness-report-2026-07-27-v2
supersedes: null
---

# Sprint Change Proposal — Epic 6 Cross-Artifact Readiness Closure

## Decision Record

| Decision | Proposed value |
| --- | --- |
| Change trigger | The second Implementation Readiness assessment found four Major cross-artifact ambiguities and three Minor documentation concerns. |
| Review mode | Batch |
| Product scope | Preserve the approved read-only multi-Vault Obsidian scope and FR-19 through FR-25. |
| Architecture scope | Preserve AD-37 through AD-40 and the separate Agent Memory / Obsidian Knowledge domains. |
| Backlog scope | Preserve Epic 6 and the approved Story 6.1–6.12 ordering. |
| Default stale-search policy | Include current and stale last-success generations from non-disabled Confirmed Vaults by default; mark stale scope and results explicitly. |
| Provisional cadence policy | Story 6.7 provides an injected/test-configured scheduler with normal-runtime enablement off; Story 6.11 measures, selects, records, and applies only the approved cadence configuration. |
| Approval state | Approved by Carver on 2026-07-27. Canonical planning edits were authorized and applied. |

## 1. Issue Summary

### 1.1 Trigger

`implementation-readiness-report-2026-07-27-v2.md` reports:

- 25/25 Functional Requirements covered;
- zero Critical issues;
- four Major issues;
- three Minor concerns;
- overall status `NEEDS WORK`.

The findings are planning consistency defects introduced while refining Epic 6
from six broad Stories into twelve bounded Stories. They do not invalidate the
product goal, the strict Vault zero-write boundary, or the approved architecture.

### 1.2 Correction objective

Make every implementation owner and acceptance reviewer derive the same answer
for:

1. which Story owns CAP-12 through CAP-18;
2. whether degraded Vaults with stale last-success results participate in the
   default Knowledge query;
3. which real-corpus snapshot and maximum note size are authoritative;
4. what Story 6.7 can implement before Story 6.11 measures a normal-use
   reconcile cadence;
5. the canonical breadcrumb, picker failure outcomes, and architecture
   provenance inputs.

No feature implementation is authorized by this proposal.

## 2. Batch Checklist Assessment

### 2.1 Trigger and context

- [x] Trigger is understood: the v2 readiness report blocks `create-story`.
- [x] The issue is reproducible from current planning artifacts.
- [x] PRD, SPEC, requirements matrix, Architecture, UX, Epic 6, readiness
  decisions, and sprint status are available.
- [x] The correction can be completed without reading or changing source code.
- [N/A] No implementation failure, dependency outage, or production incident
  caused the change.

### 2.2 Epic and Story impact

- [x] Epic 6 remains necessary and viable.
- [x] Stories 6.1–6.12 remain correctly ordered.
- [x] No Story must be deleted, renumbered, split, or moved.
- [x] Story 6.3 needs explicit native-picker negative outcomes.
- [x] Story 6.7 needs an objectively testable pre-measurement cadence boundary.
- [x] Story 6.9 needs an explicit default stale-search policy.
- [x] Story 6.11 needs a bounded configuration-only cadence activation rule.
- [x] Story 6.12 remains the verification-only final acceptance gate.
- [N/A] No future Epic is invalidated and no new Epic is required.

### 2.3 Artifact consistency

- [!] PRD contains the obsolete `1,813` baseline and an already-resolved
  note-size question.
- [!] SPEC still describes the note-size bound as unresolved.
- [!] Requirements Matrix points CAP-12 through CAP-18 to obsolete Story IDs.
- [!] UX and Epic 6 disagree on default stale-search behavior and breadcrumb
  wording.
- [!] Architecture metadata does not name the final UX and readiness-decision
  inputs.
- [x] The substantive Architecture decisions remain aligned.
- [x] Sprint status and Story ordering require no change.

### 2.4 Path evaluation

| Path | Viability | Reason |
| --- | --- | --- |
| Direct adjustment | Viable and recommended | All blockers are bounded planning inconsistencies with known target documents. |
| Rollback the Obsidian scope | Not viable | The approved feature goal remains covered and architecturally sound. |
| Reduce the MVP | Not required | No missing implementation capacity or oversized Story remains. |
| Add a new Epic | Not required | No new product outcome or dependency is introduced. |

### 2.5 Scope classification

**Minor.** The correction synchronizes existing approved decisions and makes
acceptance boundaries testable. It does not change the product vision,
architecture direction, Epic order, Story count, or implementation technology.

## 3. Recommended Adjustment

### 3.1 M1 — Repair CAP-to-Story ownership

Update only the delivery-story column for CAP-12 through CAP-18:

| Capability | Current mapping | Proposed mapping |
| --- | --- | --- |
| CAP-12 / FR-19 | Story 6.1 | Stories 6.1–6.3 and 6.12 |
| CAP-13 / FR-20 | Story 6.3 | Stories 6.3, 6.6, and 6.12 |
| CAP-14 / FR-21 | Story 6.2 | Stories 6.4–6.5 and 6.12 |
| CAP-15 / FR-22 | Story 6.4 | Stories 6.9 and 6.12 |
| CAP-16 / FR-23 | Story 6.4 | Stories 6.9–6.10 and 6.12 |
| CAP-17 / FR-24 | Story 6.5 | Stories 6.10 and 6.12 |
| CAP-18 / FR-25 | Stories 6.3 and 6.6 | Stories 6.1, 6.4, 6.7–6.8, and 6.12 |

Requirement text and Architecture-decision mappings remain unchanged.

### 3.2 M2 — Lock the default stale-search policy

Adopt one query policy across PRD, UX, Epic 6, and downstream Knowledge query
contracts:

- default scope contains every **non-disabled Confirmed Vault with a usable
  current or last-success generation**;
- a degraded Vault's last-success generation is included by default and every
  affected result is visibly marked `stale`;
- the effective-scope summary identifies which Vaults contribute stale data;
- a Confirmed Vault with no usable generation contributes no records and is
  represented in the scope/empty-state diagnostic as `not indexed` or
  `unavailable`;
- a disabled Vault is excluded from the default and cannot be selected until
  re-enabled;
- Vault filters can narrow the scope or exclude a degraded Vault;
- clearing filters restores the same default Knowledge-only scope;
- Agent Memory is never added implicitly.

This policy preserves the approved stale-last-success availability contract
instead of making valid prior Knowledge silently disappear during a
source-scoped failure.

### 3.3 M3 — Synchronize the authoritative corpus and note bound

Use the final stat-only readiness-decision artifact as the authority:

```text
Supported Markdown files = 1,796
max_note_bytes = 1,048,576
```

Proposed synchronization:

- PRD SM-12 uses the policy-qualified 1,796-note snapshot;
- PRD section 7.5 states the locked 1 MiB enforcement decision rather than a
  future requirement to define it;
- PRD risk mitigation for oversized notes references the locked policy;
- PRD open question 7 is removed because it is resolved;
- PRD open question 8 uses 1,796 and remains open only for measured thresholds
  and normal-use reconcile cadence;
- SPEC invariant text states that the note bound is locked by the readiness
  decision while cadence and performance thresholds remain measurement-driven;
- SPEC Open Questions removes the per-note bound and retains only the measured
  cadence/performance question.

The earlier 1,813-note snapshot remains historical evidence in the decision
artifact and is not rewritten.

### 3.4 M4 — Define Story 6.7's provisional cadence boundary

Replace subjective pre-measurement language with an executable boundary:

**Story 6.7**

- implements a scheduler whose cadence is injected/configured for deterministic
  tests and benchmarks;
- keeps periodic self-heal disabled by default in normal runtime until Story
  6.11 approves a cadence;
- under a controlled clock, one elapsed tick can enqueue at most one
  source-scoped reconcile for each eligible Confirmed Vault;
- no second periodic run is enqueued for a Vault while its current reconcile
  owner is queued or running;
- disabled Vaults and Vaults without an eligible Knowledge connector receive
  no periodic work;
- no-op reconcile correctness is accepted in Story 6.7, but performance and the
  normal-use interval are not.

**Story 6.11**

- measures candidate cadence behavior using the aggregate-only evidence schema;
- records the selected cadence and rationale;
- may apply only the selected cadence configuration to enable normal runtime;
- may not change reconcile algorithms, introduce FTS/tokenizer work, or perform
  another optimization;
- creates a named remediation Story if no candidate passes.

**Story 6.12**

- verifies that the approved cadence is enabled and matches the recorded gate
  decision before final acceptance.

This makes Story 6.7 testable without inventing an unmeasured production
default.

### 3.5 Minor corrections

#### m1 — Canonical breadcrumb

Use the shorter domain-owned sequence already defined by the dedicated UX
contract:

```text
Obsidian Knowledge → Vault → Folder → Note
```

Update UX-DR12 in `epics.md`; no route or architecture change is implied.

#### m2 — Native picker negative outcomes

Add the following Story 6.3 AC outcomes:

- cancellation restores focus and persists nothing;
- a selected directory that is not an Obsidian Vault returns a stable safe
  validation error and persists no Source;
- an unreadable or outside-policy selection returns a stable safe error without
  path leakage beyond the already user-visible selection;
- no invalid outcome auto-confirms a Candidate or widens the allowlisted root.

#### m3 — Architecture provenance

Update Architecture frontmatter metadata only:

- set `updated: '2026-07-27'`;
- add `../../ux-obsidian-knowledge-2026-07-27.md`;
- add `../../obsidian-knowledge-readiness-decisions-2026-07-27.md`;
- add the approved cross-artifact correction proposal after approval.

No Architecture Decision changes.

## 4. Artifact Change Matrix

| Artifact | Proposed edit | Requirement or issue closed |
| --- | --- | --- |
| `prds/prd-tessera-2026-07-20/prd.md` | Default stale-search policy; 1,796 baseline; locked 1 MiB bound; close obsolete open question | M2, M3 |
| `specs/spec-tessera/SPEC.md` | Lock note bound; keep only measured cadence/performance open; add final planning sources | M3 |
| `specs/spec-tessera/requirements-matrix.md` | Replace CAP-12..CAP-18 Story ownership | M1 |
| `planning-artifacts/ux-obsidian-knowledge-2026-07-27.md` | Default stale-search and scope/empty-state behavior | M2 |
| `planning-artifacts/epics.md` | UX-DR12/13; Story 6.3, 6.7, 6.9, 6.11, and 6.12 AC wording | M2, M4, m1, m2 |
| `architecture/.../ARCHITECTURE-SPINE.md` | Metadata provenance only | m3 |
| `implementation-artifacts/sprint-status.yaml` | No change | Story count/order/status remain valid |
| Source code, tests, runtime configuration | No change in this workflow | Planning correction only |

## 5. Detailed Old → New Edit Specification

### 5.1 Search default

**Old**

```text
Search defaults to all healthy/indexed Confirmed Obsidian Vaults.
```

and:

```text
Search all confirmed Vaults by default.
```

**New**

```text
Search defaults to all non-disabled Confirmed Obsidian Vaults that have a
usable current or stale last-success generation. Stale scope and results are
explicitly labeled; Confirmed Vaults without a usable generation contribute no
records and remain visible as not indexed or unavailable.
```

### 5.2 Corpus and note-size decision

**Old**

```text
verified real baseline of 1,813 Markdown notes
```

and:

```text
What maximum Markdown note size ...
```

**New**

```text
policy-qualified stat-only snapshot of 1,796 supported Markdown files
```

and:

```text
max_note_bytes = 1,048,576, locked by
obsidian-knowledge-readiness-decisions-2026-07-27.md
```

### 5.3 Cadence boundary

**Old**

```text
deterministic, bounded, lightweight, and configurable
```

**New**

```text
Injected/test-configured and disabled by default before Story 6.11; at most one
periodic source-scoped reconcile per eligible Vault per tick, with no overlap
while a run is queued or active. Performance and normal-use cadence are decided
only by Story 6.11.
```

## 6. Risk, Effort, and Compatibility

| Dimension | Assessment |
| --- | --- |
| Product risk | Low — no capability is added or removed. |
| Architecture risk | Low — no AD or trust boundary changes. |
| Backlog risk | Low — Story IDs and order remain stable. |
| Implementation risk reduced | Developers receive one deterministic query policy and one testable cadence boundary. |
| Documentation effort | Small — bounded edits across six canonical artifacts. |
| Compatibility | Existing Agent Memory behavior and completed Epics 1–5 are unchanged. |
| Rollback | Revert only the approved documentation correction; existing approved proposals remain intact. |

## 7. Validation After Approval

After canonical edits are authorized and applied:

1. validate YAML/frontmatter syntax;
2. verify CAP-12 through CAP-18 map only to current Story IDs;
3. verify Story numbering remains 6.1–6.12;
4. search canonical planning artifacts for stale authoritative uses of `1,813`
   and unresolved note-size language;
5. verify `1,796` and `1,048,576` reference the final decision artifact;
6. verify PRD, UX, and Story 6.9 state the same default stale-search policy;
7. verify Story 6.7/6.11/6.12 define one non-overlapping cadence handoff;
8. run whitespace/diff checks;
9. rerun `bmad-check-implementation-readiness`.

Builds, tests, commits, pushes, PR creation, and feature implementation remain
outside this planning correction.

## 8. Approval and Handoff

If approved, the Product/Architecture planning owner should apply the exact
artifact edits in Section 4. Epic 6 implementation must remain blocked until a
fresh readiness assessment returns `READY`.

Current state: **Approved and applied on 2026-07-27.**
