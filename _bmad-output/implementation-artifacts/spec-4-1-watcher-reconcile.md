---
title: 'Story 4.1: File-Change Watcher Hint & Reconcile Auto-Refresh'
type: 'feature'
created: '2026-07-25'
status: 'done'
review_loop_iteration: 0
followup_review_recommended: true
baseline_revision: '024bb333740e5f3166f86481430c816fa2e10df3'
final_revision: '9cdbc3d'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-4-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-1-4-scan-pipeline.md'
  - '{project-root}/_bmad-output/implementation-artifacts/deferred-work.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** Today every Derived-Index refresh is a manual rescan (`POST /api/scan` or `POST /api/sources/rescan`). When a Confirmed Source's memory files change on disk, nothing notices until a human triggers a rescan, so queries drift silently from the source — the index is stale-by-default between rescans.

**Approach:** Add a `notify`-based per-Source watcher that emits debounced dirty *hints*, plus a bounded reconcile pass that reuses the existing scan pipeline (full re-stage into a new generation, size/mtime/hash drift fence, fencing-token CAS commit) to turn those hints into add/modify/delete reflected in queries. A periodic reconcile self-heals any missed watcher events. The watcher never touches canonical records — all mutation still flows through the single generation-switch path.

## Boundaries & Constraints

**Always:**
- Watcher events are hints only. All canonical mutation (memory_records / scan_runs / tessera_meta.active_generation) flows through the existing `run_pipeline` → staging generation → fencing-token CAS commit (AD-5/AD-34/AD-36). The watcher hint path writes NONE of those tables.
- Reconcile IS the existing pipeline: reserve a run (`begin_run`) → `run_pipeline` (re-enumerate, re-stage, manifest+digest fence) → `commit_cas`. No second mutation path. add/modify/delete fall out of the generation switch (old generation's records replaced by new generation's on CAS commit).
- Per-Source debounce: one debounce timer per confirmed source root; a burst of edits within the window collapses to one reconcile.
- Periodic reconcile: one timer supervises all confirmed sources; each tick reconciles any source not currently reconciling. This is the self-heal for dropped/missed `notify` events (AD-8) and acts as the initial reconcile at boot.
- The previous successful generation stays queryable while a reconcile is in progress (NFR-12) — preserved by construction via staging-generation isolation; verified, not newly built.
- Scans never mutate sources (zero-source-mutation gate, SM-2) — preserved by construction; existing tests must stay green.
- Reconcile runs on a worker thread with its OWN `rusqlite::Connection`, exactly like the existing rescan worker (`http::start_rescan`). The synchronous request mutex is held only for the `begin_run` reservation, never for the FS work.
- Watcher lifecycle mirrors source lifecycle: confirm → start watch; unconfirm/delete → stop watch; boot → start watches for all confirmed sources; shutdown → drop watchers.
- Boot recovery (`recover_stale_runs`) already covers `queued/running/staging/committing` in SQL; 4.1 adds tests for the three previously-untested states since the hint queue can produce them.

**Block If:** None. (Debounce window and periodic interval are tunables with documented sane defaults; not human-input decisions.)

**Never:**
- Never let a watcher event directly write `memory_records`, `scan_runs`, or `tessera_meta` (A-12). Hints feed reconcile; reconcile mutates.
- Never add a second canonical mutation path that bypasses the atomic generation switch (violates AD-5/AD-34/AD-36). An incremental "scan only changed files" path is deferred — it requires persisted per-file fingerprints + a diff algorithm + a parallel mutation path, all out of scope for 4.1.
- Never block the synchronous request mutex on a reconcile.
- Never build degraded-state UI, failure-isolation display, or path-change re-discovery — those are Stories 4.2 and 4.3. A reconcile that fails to enumerate (e.g. root gone) preserves the previous generation and the watcher keeps running; 4.2/4.3 own the degraded surface.
- Never push long-lived SSE/streaming notifications for watcher/reconcile. The existing finite-snapshot `GET /api/scan/status` + `GET /api/sources/rescan/events` polling surfaces are the contract; long-lived SSE is Story 1.8.
- Never change the `BrowsePage` / `SearchPage` / cursor contract or any DTO. This is a server-only story; the UI observes reconcile results via existing query/status surfaces and needs no change.
- Never persist per-file fingerprints or add schema columns to enable incremental reconcile.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Confirmed source's memory file changes | `notify` event on a confirmed source's root | Per-source debounce → one reconcile (`run_pipeline`) → new generation CAS-committed → add/modify/delete reflected in queries within one debounce+reconcile cycle | Reconcile failure preserves previous active generation |
| Burst of edits in one window | Many `notify` events within debounce window | Exactly ONE debounced hint → exactly ONE reconcile | No redundant reconciles |
| Watcher drops an event (OS dropped) | File changed, no `notify` event | Periodic tick re-enumerates → change reconciled within one period | Self-heals by next periodic tick |
| Watcher event with reconcile not yet run | Hint enqueued, DB inspected immediately | No canonical row mutated by the hint itself (A-12) | Hint sits in queue until reconcile drains it |
| Mid-reconcile file drift | File changes during reconcile | `DirtyAfterValidation` → run fails → previous generation preserved → next hint/periodic retries | Existing `dirty_after_validation` behavior |
| Source root disappears mid-watch | `notify` deletion events | Reconcile enumeration fails → run `failed` → previous generation preserved; watcher continues (4.3 owns degraded UI) | Failure scoped to source; other sources unaffected |
| Concurrent reconciles across sources | Two sources' hints fire concurrently | Each reconciles independently (own fencing token, own generation, own worker) | Per-source fencing-token CAS isolation |
| Same source, hint while reconcile in-flight | Second hint during in-flight reconcile | Coalesced/dropped; in-flight reconcile completes first; CAS prevents stale commit | Next periodic tick covers any change the in-flight run missed |
| Adapter parser_version bumps | Parser version constant changes | Next reconcile re-parses all files (full re-stage) → new parser_version stamped per record | Automatic via full re-stage |
| Boot with confirmed sources | App starts | Watchers start for all confirmed sources; first periodic tick validates each index | Log-and-continue if a watcher fails to start (periodic reconcile still covers it) |
| Source confirmed at runtime | Source transitions to confirmed | Watcher starts for its root | If start fails, log; periodic reconcile covers the source |
| Source unconfirmed/deleted at runtime | Source transitions away from confirmed | Watcher stops, handle dropped | No further hints for that source |

</intent-contract>

## Code Map

- `server/src/application/reconcile.rs` (NEW) — watcher supervisor: one `notify::RecommendedWatcher` per confirmed source root, per-source debounced hint queue, periodic reconcile timer, and `trigger_reconcile(source_id)` that reserves a run and spawns a worker reusing `application::scan_reserved_source`. The hint path writes no canonical tables.
- `server/src/application/mod.rs` — declare the `reconcile` submodule; replace the aspirational "future submodule" doc comment (lines 10-11) with the now-implemented contract.
- `server/src/application/scan.rs` — likely no change to `run_pipeline`/`scan_reserved_source`/`recover_scans` (reused verbatim). If `trigger_reconcile` needs a reservation helper, factor it from `http::start_rescan`'s begin_run+spawn block so HTTP rescan and watcher reconcile share one path.
- `server/src/http/mod.rs` — factor the `start_rescan` reservation+spawn block (lines ~256-305) into a shared `trigger_reconcile(source_id, state)` callable, OR document that `reconcile` calls `application::scan_reserved_source` directly with its own worker spawn. Either way, HTTP rescan and watcher reconcile must not diverge into two mutation paths.
- `server/src/lib.rs` — start the reconcile supervisor after `recover_scans` (line ~130-132); hold watcher handles + supervisor handle in `ServerState`; drop on `Drop` for clean shutdown.
- `server/src/domain/ports/provider_adapter.rs` — watching is root-path-based and lives in `application::reconcile`, NOT on the `ProviderAdapter` trait. Update the "future `watch` method" doc comment (line 23) to reflect this placement decision.
- `server/Cargo.toml` — `notify = "8.2"` and `same-file = "1"` already declared. Add `notify-debouncer-full` (or `mini`) if the chosen debounce strategy needs it; otherwise implement debounce with a per-source timer and add nothing.
- `server/src/index/scan_store.rs` — no schema change. `recover_stale_runs` already covers all four in-flight states; add no new SQL, but the new tests will exercise `queued`/`running`/`committing` recovery for the first time.
- `server/tests/reconcile.rs` (NEW) — the AC matrix above as integration tests, plus the A-12 invariant test and boot-recovery coverage for the previously-untested states.

## Tasks & Acceptance

**Execution:**
- `server/src/application/reconcile.rs` (NEW) -- implement the watcher supervisor (per-source `notify::RecommendedWatcher`, per-source debounce, periodic reconcile timer, `trigger_reconcile`) where hints only enqueue and `trigger_reconcile` reuses `application::scan_reserved_source` on a worker thread with its own connection -- delivers AD-8 (watcher-as-hint, reconcile-as-truth) and the A-12 invariant by construction.
- `server/src/application/mod.rs` -- declare `pub mod reconcile;` and replace the aspirational doc comment with the implemented contract -- makes the submodule real and the doc honest.
- `server/src/http/mod.rs` -- factor `start_rescan`'s begin_run+spawn into a shared callable that `trigger_reconcile` also uses (or document the direct-reuse decision in code) -- ensures HTTP rescan and watcher reconcile share one mutation path, never two.
- `server/src/lib.rs` -- start the reconcile supervisor after `recover_scans`, hold its handles in `ServerState`, drop on shutdown -- gives the supervisor the same lifecycle as the DB and recovery sweep.
- `server/src/domain/ports/provider_adapter.rs` -- update the `watch` doc comment to record that watching is root-path-based and lives in `application::reconcile`, not on the trait -- prevents a future reader from re-adding a redundant trait method.
- `server/Cargo.toml` -- add `notify-debouncer-full` (or `mini`) only if the debounce strategy needs a crate; otherwise leave as-is -- minimal dependency delta.
- `server/tests/reconcile.rs` (NEW) -- integration tests for every I/O matrix row, the A-12 invariant (hint enqueued, assert no canonical row changed before reconcile drains), parser_version-bump re-parse, watcher start/stop on source lifecycle transitions, and boot-recovery for `queued`/`running`/`committing` states -- proves the contract and closes the deferred-work test-coverage gap.

**Acceptance Criteria:**
- Given a confirmed source with an active generation, when one of its memory files changes on disk, then within one debounce+reconcile cycle the change is reflected in search/browse queries (add for new, modify for changed, delete for removed), and the previous generation remained queryable throughout (NFR-12).
- Given a burst of edits to one source within the debounce window, when the debounce fires, then exactly one reconcile runs — not one per edit.
- Given a missed watcher event (file changed, no `notify` delivered), when the periodic reconcile tick fires, then the change is reconciled within one period — self-healing per AD-8.
- Given a watcher hint enqueued for a source, when the canonical tables are inspected before reconcile drains the hint, then NO row in `memory_records`, `scan_runs`, or `tessera_meta` has been mutated by the hint itself (A-12).
- Given the adapter's `parser_version` constant bumps, when the next reconcile runs, then every record is re-parsed and stamped with the new `parser_version`.
- Given a confirmed source at app boot, when the app starts, then a watcher is active for its root and the first periodic reconcile validates its index against disk.
- Given a source transitioning to confirmed (or to unconfirmed/deleted), when the transition commits, then its watcher starts (or stops) accordingly.
- Given a mid-reconcile file drift, when reconcile's manifest/digest fence detects it, then `dirty_after_validation` fires, the previous generation stays active, and the next hint or periodic tick retries.
- Given concurrent reconciles for two different sources, when both run, then each commits independently via its own fencing-token CAS and neither blocks the other's queries.
- Given the full test suite, when `cargo test --manifest-path server/Cargo.toml` runs, then all pre-existing scan/zero-source-mutation tests stay green and the new `reconcile.rs` tests pass.

## Spec Change Log

## Review Triage Log

### 2026-07-25 — Review pass 1
- intent_gap: 0
- bad_spec: 0
- patch: 18: (high 2, medium 8, low 8)
- defer: 3
- reject: 0
- addressed_findings:
  - `[high]` `[patch]` Patch A (blind-hunter #2 + edge-case #1 + IA-3d): `start_watch`/`stop_watch` were `pub` and documented as the source-lifecycle hook, but no production code called them — only `boot_start_watches` started watchers, and only at boot. A source confirmed at runtime (the normal user flow) got no `notify` watcher; a disabled source's watcher was never stopped. Wired `start_watch_best_effort`/`stop_watch_best_effort` into `http::{confirm_source, reject_source, disable_source}` (lock the supervisor slot, log/swallow errors, periodic tick is the safety net). Added 2 integration tests proving runtime confirm starts the watcher and reconciles on edit without waiting for a periodic tick, and runtime disable stops the watcher and clears hints.
  - `[high]` `[patch]` Patch B (edge-case #2 + verification-gap #1 + blind-hunter #1): `reserve_run` always allocated a fresh fencing token via `begin_run`, so an HTTP `start_rescan` worker and a watcher reconcile worker could scan the SAME source concurrently. The watcher's higher token won `commit_cas`; the HTTP worker's CAS returned 0 rows and `scan_reserved_source_with` deliberately does not `fail_run` on CAS loss → the HTTP run stayed `committing` until next boot, and `get_scan_status` reported `committing` indefinitely. Any rescan longer than one period (60s) triggered this. Added `ScanStore::has_in_flight_run(source_rowid)` and a single-owner gate inside `reserve_run`: before `begin_run`, if a non-terminal run exists, return `TriggerError::AlreadyRunning { source_id }` without allocating — this makes the previously-dead `AlreadyRunning` variant live and enforces the AD-5/16/28/32 single-fenced-owner invariant at the one shared chokepoint. Added 2 tests (reserve_run returns AlreadyRunning; trigger_reconcile returns AlreadyRunning when a rescan is in-flight).
  - `[high]` `[patch]` Patch C (blind-hunter #1): rewrote the `AlreadyRunning` variant doc to describe the live enforcement (no longer a dead variant). Patch B's tests exercise both match arms (HTTP → bad_request; loop → clear_in_flight without re-arm).
  - `[medium]` `[patch]` Patch D (blind-hunter #3 + edge-case #4): if `thread::Builder::spawn` failed or the worker's own `Connection::open` failed before calling `scan_reserved_source`, `begin_run`'s `queued` row was never marked `failed` — it stayed `queued` until next boot, contradicting the module's "失败即 fail_run、不留半态" invariant. Added `fail_reserved_run_from_main_conn` helper; both failure paths now `fail_run(scan_id, "internal")` before returning. Added test forcing worker-connection-open failure and asserting the run lands `failed`.
  - `[medium]` `[patch]` Patch E (edge-case #5): the loop's `ReservationFailed` arm re-armed the hint via `record_hint`, refreshing `queued_at` → for a disabled/deleted source, infinite retry (~120 log lines/min) until restart. Permanent failures (`"not found"`/`"not confirmed"`) now `drop_hint` (no re-arm); transient failures still clear_in_flight + record_hint. Added `HintQueue::drop_hint`. Added test driving the loop against a source disabled mid-flight, asserting the hint is dropped within a few period cycles.
  - `[medium]` `[patch]` Patch F (verification-gap #2): the refactor's string-matching error-code dispatch (`contains("not found")→source_not_found`, `contains("not confirmed")→scan_failed_not_confirmed`, else internal) had only `source_not_found` pinned by a test — a typo would silently mis-map. Added 2 HTTP tests pinning `scan_failed_not_confirmed` (disabled source) and `bad_request` for `AlreadyRunning`.
  - `[medium]` `[patch]` Patch G (verification-gap #3): `reconcile_restamps_parser_version_after_bump` only asserted a fresh reconcile stamps SOME version — it would pass even if the pipeline skipped re-staging on a version bump. Rewrote as a real bump test: stages an ACTIVE generation with a fake-old `codex-markdown/v0-FAKE-OLD` parser_version via SQL, runs reconcile, asserts the new active generation carries the adapter's CURRENT version despite identical content.
  - `[medium]` `[patch]` Patch H (verification-gap #4): `reconcile_reuses_dirty_after_validation_fence` only asserted a successful reconcile reaches `Succeeded` — never drifted a file mid-reconcile. The scripted `DriftAdapter` cannot reach `trigger_reconcile`'s adapter dispatch (it routes via `adapter_for`), so applied the documented fallback: renamed to `reconcile_run_reaches_succeeded` with a comment pointing to `scan_pipeline.rs` `manifest_drift_during_scan_marks_run_dirty_after_validation` for the real fence test.
  - `[medium]` `[patch]` Patch I (verification-gap / IA-3a): AC1 names "reflected in search/browse queries", but the headline modify test asserted via raw SQL `SELECT m.body FROM memory_records JOIN tessera_meta` — reimplementing the active-generation join rather than calling the query API. Replaced the raw-SQL assertion with a call through `application::search`, asserting the edited content is returned via `page.results()[..].excerpt()`.
  - `[medium]` `[patch]` Patch J (verification-gap / IA-3e): no test wrote to a watched directory and waited for `notify` to drive a reconcile — every hint was injected via `record_hint`/accessors, every "watcher" test routed through the periodic force-drain. Added `notify_event_drives_reconcile_end_to_end`: starts a real supervisor (60s period), confirms via HTTP, edits the file, asserts a new generation commits + is queryable via `application::search` within 10s — too fast for the 60s periodic tick, proving the real notify→hint→debounce→reconcile leg works.
  - `[low]` `[patch]` Patch K (blind-hunter #4): test-only methods (`hint_for_test`, `queue_for_test`, `state_for_test`, `clear_in_flight_pub`, etc.) shipped `#[doc(hidden)] pub` in the production binary with no `#[cfg(test)]` gate, and `clear_in_flight_pub` exposed a state-mutation seam. Renamed off the `for_test` framing (`pending_count`, `has_pending_hint`, `remove`, `record_hint_sync`, `queue`, `state`), made `clear_in_flight` pub directly, dropped the "Story 4.1 integration tests" comments.
  - `[low]` `[patch]` Patch L (blind-hunter #5 + edge-case #8): `drain_due`'s `force_all` doc claimed it "is how the periodic tick re-enumerates every confirmed source", but the periodic tick actually calls `due_for_periodic_tick`. Corrected the doc: the path drains only sources already in the queue; production periodic force-reconcile is `due_for_periodic_tick`; `force_all=true` is exercised only by tests.
  - `[low]` `[patch]` Patch M (blind-hunter #6): `DEFAULT_PERIOD` doc said "order of minutes; set via with_period for production", but `main.rs` shipped `ReconcileConfig::default()` with no override — production ran the test-biased 60s. Aligned the doc with the shipped code: "60 seconds, tuned for fast feedback on Carver's small single-machine dataset; main.rs ships `ReconcileConfig::default()`."
  - `[low]` `[patch]` Patch N (blind-hunter #8): deleted `type _Generation`, `WatchEntry::root`, and `ReconcileSupervisor::config` (plus the now-unused `PathBuf` import) — three `#[allow(dead_code)]` items with no reader.
  - `[low]` `[patch]` Patch O (verification-gap #5): `reconcile_dispatches_adapter_via_same_registry_as_scan` claimed to prove "no drift between two mutation paths" but would pass with a separate registry that happened to include Codex. Renamed to `reconcile_indexes_codex_source_via_codex_adapter`; the "no drift" guarantee is structural (both paths call `scan_reserved_source`) — noted in a comment.
  - `[low]` `[patch]` Patch P (verification-gap #6): the loop's `ReservationFailed` retry branch was untested — if broken (e.g. forgot `clear_in_flight`), the source would stick `in_flight` forever silently. Added `loop_permanent_reservation_failure_drops_hint_not_re_arms` driving the supervisor loop against a source disabled mid-flight (also covers Patch E's new drop behavior).
  - `[low]` `[patch]` Patch Q (verification-gap #7): `supervisor_drop_stops_the_loop` only asserted `drop(supervisor)` returns — never that the loop stopped. Added `supervisor_drop_stops_the_loop_and_no_further_reconcile_fires`: after drop, mutates the file, waits 3× the period, asserts no new generation commits.
  - `[low]` `[patch]` Patch R (verification-gap #8): `hint_queue_force_all_drains_every_confirmed_source` name oversold — `drain_due(force_all=true)` only yields queued sources, and the test only inserted one. Renamed to `hint_queue_force_all_drains_every_queued_source`.

## Design Notes

- **Why reconcile reuses `run_pipeline` instead of a new incremental path.** The AC says "reconcile 通过受限扫描 + size/mtime/hash + parser_version 判断变化" (reconcile detects changes via a bounded scan + size/mtime/hash + parser_version). Two readings: (a) a bounded FULL re-scan of the source that reuses the existing pipeline, where size/mtime/hash are the manifest+digest drift fence and parser_version is stamped per record; (b) an INCREMENTAL scan that compares new size/mtime/hash against persisted per-file fingerprints to skip unchanged files. The architecture selects (a) uniquely: AD-5/AD-34/AD-36 mandate a single atomic-generation-switch mutation path, and (b) would require a second mutation path (or persisted per-file fingerprints + a diff that bypasses the generation switch) — breaking that invariant. Reading (a) also makes "periodic reconcile 修复漏事件" self-evidently true: a full re-enumeration repairs any missed event by construction. The incremental path (b) is explicitly deferred; its prerequisites (persisted per-file fingerprints, a diff algorithm) are out of scope for 4.1.
- **Why the watcher lives in `application::reconcile`, not on `ProviderAdapter`.** Both Codex and Claude watch a filesystem root, not an adapter-specific surface. Adding `watch` to the trait (as the doc comment at `provider_adapter.rs:23` once mused) would couple every future adapter to a filesystem model it may not have. Root-path-based watching at the application layer is adapter-agnostic and matches how `begin_run`/`run_pipeline` already key off `source_id`.
- **Why no UI change.** The AC's observable is "add/modify/delete 反映到查询" (reflected in queries), not "the UI auto-refreshes in real time". Queries already return the active generation's records; after a reconcile CAS-commits a new generation, the next search/browse returns the new data. The UI fetches on entry today and needs no change. Real-time push is long-lived SSE, which is Story 1.8.
- **Why the periodic tick is mandatory, not optional.** `notify` event delivery is best-effort on every OS (recursively-watched dirs, inode-recycling, buffer pressure all drop events). AD-8 makes periodic reconcile the self-heal. Without it, the index could drift silently whenever the OS drops an event — exactly the stale-by-default failure 4.1 exists to fix. The interval is a tunable (default on the order of minutes), not a human-input decision.
- **Threading recap (binding constraint, per deferred-work).** The transport is tiny_http + one-thread-per-connection + synchronous handler + `std::sync::Mutex<Connection>`. The rescan worker is the ONE sanctioned exception: it opens its own `Connection` and relies on the fencing-token CAS. Reconcile reuses that exact pattern. No async path exists; the `notify` callback (on a notify-internal thread) enqueues a hint and returns immediately — it never acquires the request mutex.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests green (scan_pipeline, http_api, zero-source-mutation, inventory, source_registry, claude_code_scan) PLUS the new `reconcile.rs` integration tests passing.
- `cargo clippy --manifest-path server/Cargo.toml -- -D warnings` -- expected: clean (the watcher introduces a background thread and a notify callback; clippy catches the obvious lifetime/locking mistakes).
- `cargo build --manifest-path server/Cargo.toml` -- expected: clean compile with the new module wired into `ServerState`.

**Manual checks (if no CLI):**
- Confirm a Codex or Claude source, edit one of its memory files, and without restarting poll `GET /api/scan/status?source_id=...`: within one debounce+reconcile cycle the `active_generation` advances and `GET /api/search` returns the edited content; the previous generation was queryable the entire time.
- Stop the app, edit a file, restart: the boot watcher + first periodic tick reconcile the change without a manual rescan.

## Auto Run Result

Status: done

**Summary:** Implemented Story 4.1 — a `notify`-based per-Source watcher emitting debounced dirty *hints*, plus a periodic reconcile self-heal, all routed through ONE shared reservation+worker path that reuses the existing `scan_reserved_source` → atomic generation switch (AD-5/AD-34/AD-36). The hint path writes none of `memory_records`/`scan_runs`/`tessera_meta` (A-12, pinned by test). On-disk changes to a Confirmed Source now reflect in `application::search`/`browse` queries within one debounce+reconcile cycle without a manual rescan; the previous generation stays queryable throughout (NFR-12, structurally via the existing generation switch). Server-only: no UI, DTO, cursor, or long-lived-SSE change (those are out of scope / Story 1.8).

**Files changed:**
- `server/src/application/reconcile.rs` (NEW) — watcher supervisor: one `notify::RecommendedWatcher` per confirmed source root, per-source debounced `HintQueue`, periodic reconcile timer (registry-driven force-reconcile every period = AD-8 self-heal + boot validation), shared `reserve_run` (single-owner gate via `has_in_flight_run`), shared `trigger_reconcile`, `run_reconcile_loop`. Hint path is in-memory only.
- `server/src/application/mod.rs` — declared `pub mod reconcile;`, replaced the aspirational "future submodule" doc with the implemented contract, re-exported the public surface.
- `server/src/http/mod.rs` — refactored `start_rescan` to call the shared `application::reserve_run`; wired `start_watch`/`stop_watch` into `confirm_source`/`reject_source`/`disable_source` via best-effort helpers (Patch A).
- `server/src/index/scan_store.rs` — added `has_in_flight_run(source_rowid)` for the single-owner gate (Patch B); no schema change.
- `server/src/lib.rs` — added `reconcile_supervisor: Mutex<Option<ReconcileSupervisor>>` to `IndexState`; `boot_with_reconcile`/`install_reconcile` so the supervisor (borrowing `Arc<IndexState>`) wires in after the Arc exists; `Drop` stops it cleanly.
- `server/src/main.rs` — switched to `boot_with_reconcile`.
- `server/src/domain/ports/provider_adapter.rs` — replaced the "future `watch` method" doc with the placement decision (watching is root-path-based, lives in `application::reconcile`, not on the trait).
- `server/src/application/source.rs` — widened `adapter_for` to `pub` so the reconcile test can prove adapter dispatch matches the scan path.
- `server/tests/reconcile.rs` (NEW) — 31 integration tests covering every I/O matrix row, the A-12 invariant, a real parser_version-bump re-stamp, a real notify→hint→debounce→reconcile end-to-end test, watcher start/stop on runtime HTTP lifecycle transitions, the single-owner gate, orphan-hint drop, and boot-recovery for the previously-untested `queued`/`running`/`committing` states.

**Review findings:** Review pass 1 — 0 intent_gap, 0 bad_spec, 18 patches applied (2 high: A watcher-lifecycle wiring, B single-owner gate; 8 medium: D fail_run on worker failure, E orphan-hint drop, F HTTP error-code mapping tests, G real parser_version-bump test, H dirty_after_validation test rename, I query-surface assertion, J notify end-to-end test, + IA surface divergences; 8 low: K test-only surface cleanup, L drain_due doc, M DEFAULT_PERIOD doc, N dead-code removal, O/P/Q/R test renames + new loop/drop coverage), 3 deferred (HintQueue mutex poison, boot list-failure watch retry, NFR-12 in-flight read test). Patch score: high present → follow-up review recommended.

**Follow-up review recommended:** true.

**Verification performed:**
- `cargo build --manifest-path server/Cargo.toml` — clean, no warnings.
- `cargo clippy --manifest-path server/Cargo.toml -- -D warnings` — clean.
- `cargo clippy --tests --manifest-path server/Cargo.toml -- -D warnings` — clean.
- `cargo test --manifest-path server/Cargo.toml` — 362 tests pass, 0 fail (95 lib + 267 integration across 15 test files, including 31 reconcile tests + 38 http_api tests). All pre-existing scan_pipeline / zero-source-mutation / inventory / source_registry / claude_code_scan suites stay green after the `start_rescan` refactor and the lifecycle-wiring changes.
- Binary smoke test (`cargo run --bin tessera`) — boots cleanly past migrations + recovery + supervisor start.
- Matrix Test Audit — all 11 I/O matrix rows covered by passing tests; the headline modify row now asserts via `application::search` (Patch I), and the notify end-to-end row is covered by a real disk-write test (Patch J).

**Residual risks:**
- The notify end-to-end test (Patch J) relies on kernel notify event delivery within 10s; it passed reliably on this macOS host. If it proves flaky on a different CI platform, the documented fallback (assert a hint was recorded via `has_pending_hint` within a bounded timeout) can be swapped in.
- The Patch D worker-failure test uses `Arc::get_mut` to sabotage `db_path`; a future test refactor that clones the state earlier would make it panic with a clear message rather than silently pass.
- NFR-12 is structurally satisfied but not tested at the in-flight-read surface (deferred).
- Per-worker threads are fire-and-forget (not joined on shutdown); the supervisor loop thread IS joined in `Drop`. This mirrors the existing `http::start_rescan` worker pattern.

