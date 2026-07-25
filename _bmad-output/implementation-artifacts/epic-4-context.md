# Epic 4 Context: 健康诊断、失败隔离与索引重建 (Health Diagnostics, Failure Isolation & Index Rebuild)

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Make Tessera trustworthy under failure. When a Connector breaks (path moved, permission changed, format unsupported, scan failure), the other Sources remain searchable, the failed Source's cause and last-success time are shown explicitly with a stale marker, and the user can trigger a rescan or delete-and-rebuild the entire Derived Index without losing Confirmed Sources or Tessera Project mappings. This epic establishes resilience as the trust foundation before cross-Agent project federation (Epic 5).

## Stories

- Story 4.1: File-change watcher hint and reconcile auto-refresh
- Story 4.2: Connector failure isolation and stale last-success results
- Story 4.3: Rediscovery and degraded handling for path/permission/identity change
- Story 4.4: Full Derived Index rebuild

## Requirements & Constraints

- **Change detection must be reconcile-driven.** Watcher events are debounce hints only; truth comes from a bounded reconcile pass using size/mtime/hash and parser_version. Watcher events must never directly add/modify/delete canonical records, and missed events must be repaired by periodic reconcile.
- **Failures are source-scoped, never global.** A single Connector failure must not block search or browse of other Sources. The failed Source's previous successful generation stays queryable and is marked with last-success time + stale; a half-failed scan must never overwrite the last good generation.
- **Source Health is structured, not boolean.** Each Confirmed Source carries `unknown | healthy | degraded | error` with a human-readable cause distinguishing at minimum: path missing, permission denied, format unsupported, scan failed. Lifecycle, health, coverage, scan state, and active generation are modeled as separate fields.
- **Path/identity change preserves confirmation.** When a Source root moves or its filesystem identity changes, the old Source is retained as degraded (with cause + last-success time) and a new Candidate is produced. No automatic merge or copy of Tessera Project mappings; only explicit rebind changes the root. Ambiguous/colliding fingerprints stay separate Candidates.
- **Rebuild is non-destructive to user state.** Reset Index clears canonical body, FTS, and scan_runs but preserves Source Registry and Tessera Project mappings. Before rebuild, the user must be told only Tessera-derived data is deleted. After rebuild, stable record IDs and Provenance for the same sources must be restored. Rebuild failure must leave original Agent Memory untouched.
- **Scans never mutate sources.** Source file set/content/size/mtime must be unchanged before/after any scan or rebuild (zero-source-mutation gate).
- **Error displays are redacted.** No memory body, query text, or credentials in error messages or logs.
- **Scans must not block queries.** The previous successful Derived Index remains queryable while a scan/reconcile is in progress.
- **Success metrics gating this epic:** failure isolation (one Source forced unreadable, the other still queryable, failed Source shows explicit state, last-success results survive a half scan) and rebuildability (delete index, rebuild restores stable record identity, Tessera Project mappings, and Provenance).

## Technical Decisions

- **Watcher-as-hint, reconcile-as-truth (AD-8).** `notify` (8.2.x) produces per-Source debounced dirty hints; a bounded reconcile decides actual changes. Periodic reconcile self-heals missed/dropped events.
- **Single fenced owner per Source (AD-5, AD-16, AD-28, AD-32).** Each Source has one queued Scan/Reconcile owner. `scan_runs` persists `queued/running/staging/committing/succeeded/failed/retry`. On startup, stale `running/staging/committing` runs are recovered into retryable failures and unactivated staging is cleaned. Scan/reconcile carries a durable monotonic fencing token + generation intent; commit is a same-transaction compare-and-swap on token+intent, so a cancelled/timed-out/retried old owner can never commit over a newer one.
- **Atomic generational visibility (AD-5, AD-34, AD-36).** Scans write a staging generation; only a fully successful, validated generation becomes active in one transaction. Consistency level is `snapshot-at-validation`: a final fence/manifest check (size/mtime/hash + parser_version) runs in the commit transaction. Any mutation detected after validation or during commit marks the generation `dirty_after_validation` — it never becomes active/visible and schedules a bounded retry. Failed scans leave the previous active generation visible.
- **Structured source-scoped errors (AD-13).** One shared error envelope: stable `code`, safe `message`, `source_id`, phase. A Source failure never invalidates unrelated Source generations. Diagnostics are local and redacted.
- **Versioned fingerprint survives path change (AD-33, AD-35).** `source_id` is persistent; re-discovery matches on `provider + canonical root fingerprint`. Fingerprint format is versioned (`root-fingerprint/v1`) built from provider + root kind + normalized root path + filesystem identity `(device, file_id)`, with normalized path as explicit fallback when identity is unavailable. No fuzzy merge.
- **Reset boundary is explicit (AD-29).** Reset Index cleans canonical body/FTS/scan_runs, retains Source Registry and Tessera Project mappings. Removing a Source cleans its derived records. Migrations are atomic; a failed migration leaves the last usable index intact. Body never enters logs/snapshots.
- **Health lives in core, surfaced via HTTP/SSE (AD-7, AD-9, AD-17).** All health/reconcile state is owned by the Rust core (not UI); the browser UI consumes it via the versioned loopback-only HTTP API. Scan progress uses SSE with monotonically increasing sequence and a cancellation token. (Epic 4 logic is entirely in core and is unaffected by the Tauri→local-web transport pivot; only command entries are HTTP handlers.)
- **Adapter recovery tests are mandatory (AD-14).** Codex and Claude adapters must pass fixture contract, zero-source-mutation, parser-version, reconcile-recovery, and capability-honesty tests before being enabled — reconcile-recovery is the directly Epic-4-relevant gate.

## UX & Interaction Patterns

- **Structured status over boolean.** Source Inventory must render health as the structured `unknown/healthy/degraded/error` enum with a readable cause, last-success time, and stale marker — never a single "connected" flag. Cause must distinguish path missing / permission denied / format unsupported / scan failed.
- **Three-way empty/error distinction.** Empty result states must distinguish "genuinely no match" / "Source not indexed" / "Source currently unavailable". A failed Source's previously successful results, if still shown, must carry last-success time + stale.
- **Explicit, warned destructive action.** Rebuild must be preceded by a clear notice that only Tessera-derived data is deleted (Sources and project mappings are kept).
- **Shared accessibility contract (AD-21).** Inventory/Health views share focus order, keyboard-reachable commands, readable status labels, and EmptyState with the rest of the app; visual indicators must not be the only way to perceive health state. Acceptance artifact: `tests/ui/accessibility.spec.ts`.

## Cross-Story Dependencies

- **Depends on Epic 1 and Epic 2.** Reuses the atomic generation pipeline, Source Registry, fingerprint identity, and structured error envelope established there. Codex and Claude Code adapters and their contract suites (including reconcile-recovery) must already be in place.
- **Story 4.1 (watcher/reconcile) is the foundation for 4.2 and 4.3.** Failure isolation and degraded handling both rely on the reconcile pass and the persisted scan-run state machine introduced here.
- **Story 4.4 (rebuild) is independent of 4.1–4.3** but reuses the Reset Index boundary (AD-29) and re-activates the same scan pipeline.
- **Feeds Epic 5.** Resilience and stable identity under failure are prerequisites for cross-Agent project federation; degraded-path and rebind behavior protects mapping integrity.
