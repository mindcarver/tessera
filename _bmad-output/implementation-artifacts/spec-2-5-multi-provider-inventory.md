---
title: 'Story 2.5: Multi-provider Source Inventory panorama & cross-source health'
type: 'feature'
created: '2026-07-25'
status: 'done'
baseline_revision: '5a08028'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-2-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-2-4-cross-provider-filters.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** The Source Inventory already lists every confirmed source regardless of provider (it has been multi-provider at the row level since Story 2.1 added Claude rows to the registry) and every AC field — Provider, path, Native Project, Coverage Level, Source Health, last successful scan, record count, last error — is already on the wire and rendered per card, with counts honest per Coverage. What is missing is the **panorama / cross-source comparison**: cards render as a flat list ordered by insert id (providers interleaved), there is no grouping or summary, and there is **no test** pinning a multi-provider inventory or the "one source down, the rest still show" guarantee on the inventory endpoint.

**Approach:** Add the comparison affordance in the UI (group cards by provider, a health-summary header, a `data-provider` hook so multi-provider rendering is pinnable), keep the honest per-Coverage counts and per-source isolation already in the backend, and add the missing multi-provider + one-source-down inventory tests (Rust + HTTP + Playwright). No schema or backend-aggregation change.

## Boundaries & Constraints

**Always:**
- The inventory shows **every** confirmed source (all providers) — already the case; 2.5 must not narrow it.
- Each card shows Provider, path/root, Native Project, Coverage Level, Source Health, last successful scan, record count, last error — already rendered; 2.5 keeps it and adds a `data-provider` attribute on each inventory card (mirroring the 2.3 search-card convention) so multi-provider rendering is selectable.
- **Cross-source health is comparable**: cards are grouped by provider, and a health-summary header states the totals (e.g. "2 sources · 1 healthy · 1 degraded") so Carver can compare providers' health/coverage/counts at a glance.
- Record counts stay honest per Coverage: `complete_record_count` is shown only for `CoverageLevel::Full`; a non-Full source shows an "unavailable (coverage is limited)" message, never a disguised zero (already the case; keep it).
- One source's scan failure / `error` health does not affect another source's display or status — already true at the scan/health layer (per-source `set_health`, per-source row assembly); 2.5 pins it with an inventory-level test.
- The inventory stays keyboard-reachable with readable status labels; the accessibility contract holds.

**Block If:** A requirement emerges to isolate the inventory aggregation itself from a SQLite/infrastructure failure (any per-source SQL error currently surfaces as a single inventory-wide `internal`). That is Epic 4 (connector failure isolation) territory, not 2.5 — stop and re-plan if it is required here.

**Never:**
- Change the inventory backend aggregation, the `SourceInventory` DTO field set, or the schema. The backend is already multi-provider and per-source-isolated at the row level.
- Disguise a non-Full coverage count as zero, or hide a failed source's row.
- Introduce server-side persistence of a "panorama" or grouping state — grouping/sorting is a client-side render decision.
- Re-add the 2.2-removed `provider_not_scannable` vocabulary (the negative assertion at `http_api.rs:714` must keep passing).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Multi-provider inventory | 1 Codex + 1 Claude confirmed | Both cards render in one inventory; grouped by provider; `data-provider` set on each; summary header shows "2 sources · …" | No error |
| One source down | source A `error`/failed latest run, source B healthy | Both rows return; A shows its health + latest_error; B unaffected (healthy, its own count/last-scan) | No error |
| Honest count (non-Full) | a `search_only`/`existence_only` source | `complete_record_count` shown as unavailable ("coverage is limited"), not 0 | No error |
| Honest count (Full) | a `Full` source with an active generation | `complete_record_count = N` (active-generation records) | No error |
| Health ordering within a group | two sources under one provider, one `degraded`/`error` | the worse-health source sorts first within its provider group (attention-first) | No error |
| No sources confirmed | empty registry | inventory empty-state ("No sources have been confirmed yet.") | No error |
| Disabled/rejected source present | a `disabled` or `rejected` row | it still appears (lifecycle shown); the panorama reflects real registry state | No error |

</intent-contract>

## Code Map

- `src/features/sources/Sources.tsx` — `InventoryCard`: add `data-provider={item.provider}`; group the inventory `<ul>` by provider (one group per provider, ordered), sort within group by health severity (error/degraded before healthy/unknown); add a health-summary header above the list ("N sources · X healthy · Y degraded · Z error"). Keep every existing field + the honest-count copy + per-card Rescan/Cancel/Disable.
- `src/api/sources.ts` — no DTO change (`SourceInventory` already carries every field); confirm `asInventory` keeps validating the honest-null count contract.
- `server/tests/inventory.rs` — add a multi-provider inventory test (one Codex + one Claude row: both present, each with its own provider/health/count) and a one-source-down test (one `error`, one `healthy`: both rows return, the failed one carries `latest_error`, the healthy one's count/last-scan intact).
- `server/tests/http_api.rs` — assert `GET /api/sources/inventory` returns both providers' rows over HTTP for a mixed fixture; preserve the `provider_not_scannable` negative assertion.
- `tests/ui/accessibility.spec.ts` — mock a two-provider inventory payload (Codex + Claude), assert both cards render with `data-provider`, the provider grouping, and the health-summary header; keep the existing single-source inventory coverage.

## Tasks & Acceptance

**Execution:**
- `src/features/sources/Sources.tsx` -- on `InventoryCard`, add `data-provider={item.provider}`; render the inventory grouped by provider (one section/group per provider), within each group sort by health severity (`error` > `degraded` > `healthy` > `unknown`), and add a health-summary header above the list counting sources by health state -- deliver the "cross-source health comparable" panorama without touching the field set or honest counts.
- `server/tests/inventory.rs` -- add `inventory_lists_multiple_providers_together` (one Codex + one Claude row, each with its own provider/coverage/count) and `inventory_one_source_down_does_not_affect_others` (one source `error` with a failed latest run + `latest_error`, one `healthy` with its count + last-successful-scan intact) -- pin the multi-provider and one-source-down guarantees at the inventory endpoint.
- `server/tests/http_api.rs` -- assert `GET /api/sources/inventory` returns both providers' rows over HTTP for a mixed Codex+Claude fixture (each row's provider/health/count); keep the `provider_not_scannable`-absent negative assertion green.
- `tests/ui/accessibility.spec.ts` -- mock a two-provider inventory (Codex + Claude), assert both cards render with the correct `data-provider`, the provider grouping, and the health-summary header; keep keyboard reachability and the existing inventory coverage.

**Acceptance Criteria:**
- Given multiple confirmed sources (Codex + Claude Code), when Carver opens the Inventory, then every source's card shows Provider, path, Native Project, Coverage Level, Source Health, last successful scan, record count, and last error; cards carry `data-provider` and are grouped by provider.
- Given a multi-provider inventory, when rendered, then a health-summary header states the totals (e.g. "2 sources · 1 healthy · 1 degraded"), so cross-source health is comparable at a glance.
- Given one source whose latest scan failed (`error` health) alongside a healthy source, when Carver opens the Inventory, then both rows appear — the failed one shows its health + latest error, the healthy one's status/count/last-scan are unaffected.
- Given a source whose Coverage is not `Full`, when rendered, then its record count shows as unavailable (coverage-limited), never as a disguised zero.
- Given the inventory, then it is keyboard-reachable with readable status labels and the accessibility contract holds.

## Spec Change Log

## Review Triage Log

### 2026-07-25 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 13: (high 0, medium 4, low 9)
- defer: 0
- reject: 0
- addressed_findings:
  - `[medium]` `[patch]` Panorama UI test now exercises an `error`-health source (the "one source down" AC), the within-group health sort on a multi-card bucket, the group alphabetical order, and the summary-above-list DOM order — the core panorama claims that were previously unverified.
  - `[medium]` `[patch]` Added a non-Full (`search_only`) inventory UI fixture pinning the honest "Complete count unavailable: coverage is limited." copy (was only pinned at the DTO layer).
  - `[medium]` `[patch]` Summary order is now attention-first (`error`→`degraded`→`healthy`→`unknown`), matching the within-group sort; health nouns pluralize.
  - `[low]` `[patch]` Card heading uses `providerDisplayName`; `healthSeverityRank` gained a `default`; record-count copy pluralizes; group sort pins `localeCompare("en", base)`; `providerDisplayName` hoisted per group; HTTP inventory test pins `last_successful_scan` via `finished_at`; `data-testid="source-inventory"` host is a semantic `<section>`.

## Design Notes

- **The backend is already multi-provider.** `list_inventory` (`application/scan.rs`) iterates `registry.list()` with no provider filter; `SourceInventory` carries all eight AC fields; `complete_record_count` is `Some` only for `CoverageLevel::Full` (a missing value is never a disguised zero); `last_successful_scan` is deliberately separate from the latest run so a failed rescan doesn't erase the prior success. 2.5 adds none of this — it adds the panorama affordance and the missing tests.
- **"Cross-source health comparable" is a UI concern.** The data is already per-card; 2.5 groups by provider, surfaces a summary header, and adds `data-provider` so the comparison is visible and pinnable. No DTO or sort on the server.
- **Per-source isolation is already structural at the scan/health layer** (per-source `set_health`, per-source row assembly). The only residual global-failure mode is a SQLite/infrastructure error during aggregation — that is Epic 4 (connector failure isolation), explicitly out of scope (Block If), not a 2.5 regression.
- **Continuity from 2.1–2.4 (KEEP).** Multi-provider registry rows, per-source health/count/last-error, the honest-null count contract, the disabled/rejected rows still appearing, and the per-card Rescan/Cancel/Disable actions all survive unchanged.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests pass plus new multi-provider + one-source-down inventory tests.
- `cargo test --manifest-path server/Cargo.toml inventory` -- expected: multi-provider + one-source-down inventory tests green.
- `npm run build` -- expected: TS compiles (no DTO change; UI grouping/summary).
- `npx playwright test tests/ui/accessibility.spec.ts` -- expected: multi-provider inventory test green; existing a11y contract holds.
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- expected: clean.

**Manual checks:**
- With one Codex + one Claude source confirmed and scanned, open the Inventory in the app; confirm both providers' cards appear grouped by provider with a health-summary header, each card shows its full status, and forcing one source unhealthy (e.g. a failed rescan) leaves the other card's status/count/last-scan intact.

## Auto Run Result

Status: done
Follow-up review recommended: true (pass patches: medium 4, low 9 → score 21 ≥ 5).

**Summary:** Multi-provider Source Inventory panorama. The backend inventory was already multi-provider + per-source-isolated + honest-per-Coverage since 2.1, so 2.5 is UI + tests: cards grouped by provider, an attention-first health-summary header, within-group health sort, and `data-provider`/`data-provider-group` hooks. New tests pin a multi-provider inventory, one-source-down isolation, and (after review) the panorama's `error`-state UI, within-group sort, group/summary order, and the non-Full honest-count copy.

**Files changed:** `src/features/sources/Sources.tsx`, `server/tests/{inventory,http_api}.rs`, `tests/ui/accessibility.spec.ts`.

**Review findings:** patches applied 13 (medium 4, low 9); deferred 0; rejected 0.

**Verification:** `cargo test` (skip flaky perf gate) → 289 passed, 0 failed; isolated perf gate → 8 passed; `cargo clippy --all-targets -D warnings` → clean; `npm run build` → clean; `npx playwright test tests/ui/accessibility.spec.ts` → 9 passed.

**Residual risks:** (1) Inventory-aggregation isolation from a SQLite/infrastructure error is still Epic 4 territory (Block If) — a per-source SQL error surfaces as a single inventory-wide `internal`. (2) The panorama summary is a global health roll-up; per-provider side-by-side roll-up is not implemented (the grouping + sort enable visual comparison). (3) The perf gate remains parallel-flaky (pre-existing, deferred).
