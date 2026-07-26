---
title: 'Story 4.4: Full Derived Index Rebuild'
type: 'feature'
created: '2026-07-26'
status: 'done'
review_loop_iteration: 0
followup_review_recommended: true
baseline_revision: 'bcd590f92643e7c9e259a61536206d5f64235edb'
final_revision: '0f1549883f736307fbbc1e4b966201ffc9139cbe'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-4-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-4-3-path-change-degraded.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-4-2-connector-failure-isolation-stale-last-success.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-4-1-watcher-reconcile.md'
  - '{project-root}/_bmad-output/planning-artifacts/epics.md'
warnings: [oversized]
---

<intent-contract>

## Intent

**Problem:** There is no way to delete and fully re-derive the Derived Index. The only existing wholesale clear is the one-shot v3 schema migration (`server/src/index/migrations.rs:270-272`); reject/disable never delete a source's derived records (`server/src/application/source.rs:183,234`), so stale records from disabled/rejected sources leak until re-confirm. When the index is corrupt or the user wants a clean re-derivation, nothing resets the derived data while keeping Confirmed Sources and (future) Tessera Project mappings intact (FR-15, A-29/AD-29, NFR-10).

**Approach:** Add one operation — `POST /api/index/rebuild` — that (1) atomically wipes exactly the Tessera-derived tables (`memory_records`, `scan_runs`, `scan_diagnostics`, and the `active_generation:*` rows of `tessera_meta`) in a single transaction while preserving `source_registry`, `tessera_meta.schema_version`, and `tessera_migrations_applied`; then (2) re-scans every Confirmed Source by reusing the existing read-only scan pipeline (4.1's reserve→worker→`scan_reserved_source`→`commit_cas` path, the same one `start_rescan` uses). Because `record_id = rec_<fnv1a(source_id|provider|native_locator|unit_kind)>` (`server/src/domain/scan.rs:316`) is a pure function of source data + the preserved `src_<rowid>`, re-scanning reproduces identical record IDs and Provenance for unchanged sources. No new mutation path, CAS, or state machine — the rebuild is the existing pipeline applied to every Confirmed Source after a wipe.

## Boundaries & Constraints

**Always:**
- The wipe deletes EXACTLY four targets in ONE SQLite transaction: `memory_records`, `scan_runs`, `scan_diagnostics`, and `tessera_meta` rows `WHERE key LIKE 'active_generation:%'`. It MUST NOT touch `source_registry`, `tessera_meta.schema_version`, `tessera_migrations_applied`, or any other `tessera_meta` key. (`tessera_meta` is MIXED — a blanket `DELETE FROM tessera_meta` would destroy the schema version. Mirror the v3 precedent at `migrations.rs:272`, but ADD `scan_diagnostics` which v3 predates.)
- Only Confirmed Sources are re-scanned, via the existing read-only pipeline. The zero-source-mutation gate holds: rebuild only reads source files (the scan path's only FS calls are `File::open`/`read_to_end`/`canonicalize`/`metadata`, `application/scan.rs:700-737`); a test must assert source file set/content/size/mtime are unchanged across rebuild (NFR-1/NFR-10).
- Reject the rebuild with `409 rebuild_failed` if any scan run is currently in-flight (`queued/running/staging/committing`) across ANY source — checked via a new `ScanStore::any_in_flight_run()`. This is the primary race guard: it prevents a scan that has already staged data from being wiped mid-pipeline. Existing 4.2 source-scoped error isolation applies to the post-wipe re-scans: a source that fails to re-scan (e.g. unreadable) is marked degraded/error with cause + last-success + stale per 4.2; other sources still rebuild.
- Reserve the rebuild's per-source runs UNDER the IndexState mutex (one `reserve_run`/`begin_run` per Confirmed source in the same critical section that wipes), so reconcile cannot grab any Confirmed source between the wipe and the rebuild's dispatch. Workers then run on their own connections exactly like `start_rescan` (`server/src/http/mod.rs:412-415`), so queries stay available during the rebuild (NFR-12).
- Per-source progress MUST flow through the existing `rescan_jobs` map + `GET /api/sources/rescan/events` SSE by dispatching each rebuild scan through the same path `start_rescan` uses — no new SSE channel. A rebuild is "settled" when every Confirmed source's latest run reaches a terminal state (`succeeded/failed/cancelled`).
- Errors use the AD-13 envelope. Add a `rebuild_failed` constructor (→ 409) for the in-flight-rejection; wipe/DB failures map to `internal` (→500). No memory body, query text, or credentials in any message (NFR-3).
- `record_id` reproduction is guaranteed by construction (identity depends only on source data + preserved `source_id`); the rebuild MUST NOT introduce any scan-time random/autoincrement component into `record_id`. The `generation` half of the `memory_records` PK (`gen_<scan_runs.id>`) WILL differ after rebuild (scan_runs is wiped and AUTOINCREMENT continues) — this is expected and harmless, since generation is not part of `record_id` or Provenance.

**Block If:** (none — all readings resolved; rationale under Design Notes).

**Never:**
- Do NOT add a schema migration for the rebuild. It is a repeatable RUNTIME operation, not a migration. (v3's wipe was a one-time migration; this is the user-triggered equivalent.) Do NOT touch `CURRENT_SCHEMA_VERSION` in `server/src/index/mod.rs:29` — its pre-existing staleness (declares 5, real is 6) is out of scope for this story.
- Do NOT delete or alter `source_registry` rows, `schema_version`, or `tessera_migrations_applied`.
- Do NOT blanket-`DELETE FROM tessera_meta` (MIXED table). Only `active_generation:*` rows.
- Do NOT mutate source files (zero-source-mutation gate).
- Do NOT hold the IndexState request mutex for the duration of the per-source rescans — wipe + reserve-all under the mutex, then scans run on worker connections (NFR-12).
- Do NOT add a Tessera Project mapping table or migration (Epic 5). "Preserve project mappings" is satisfied trivially today (none exist); the wipe simply must not target any future mapping data, and none exists to protect.
- Do NOT pause/stop the reconcile supervisor as a correctness requirement — SQLite write serialization + the rebuild's full per-source re-dispatch make it unnecessary (see Design Notes). (An implementation MAY pause it as an optimization, but correctness must not depend on it.)
- Do NOT relax any 4.1/4.2/4.3-pinned test (`replaced_root_requires_reconfirmation_and_preserves_active_generation`, `same_path_different_inode_yields_different_source_no_merge`, `inventory_one_source_down_does_not_affect_others`, reconcile-recovery tests).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| Happy rebuild (idle, ≥1 Confirmed source) | All Confirmed sources idle; user confirms the warning and POSTs `/api/index/rebuild` | One tx wipes `memory_records`+`scan_runs`+`scan_diagnostics`+`active_generation:*`; `source_registry`/`schema_version`/migrations audit unchanged; each Confirmed source is re-scanned to a fresh active generation; `record_id`s + Provenance identical to pre-rebuild for unchanged source data; response `200 {sources_rescanning: N}`; per-source progress visible via existing rescan SSE/inventory | No error |
| Rebuild restores stable identity + Provenance | Same source data before and after | Post-rebuild active records have the SAME `record_id` and Provenance fields (`native_locator`, `display_locator`, `native_unit_id`, `provider`, `unit_kind`, `provider_memory_type`, `native_project`) as pre-rebuild (only `generation`/`observed_at` may differ) | No error |
| Rebuild with one unreadable source | One Confirmed source's root unreadable (4.2 PathMissing/PermissionDenied), others healthy | Wipe proceeds; unreadable source's re-scan fails → marked degraded/error + cause + last-success(None post-wipe)+stale per 4.2; healthy sources rebuild fully; `source_registry` unchanged; source FILES untouched | Per-source `scan_failed` envelope surfaced via that source's inventory/SSE; rebuild itself still returns 200 |
| Rebuild while a scan is in-flight | Any source has a `queued/running/staging/committing` run | No state change (no wipe, no reservation) | `409` envelope `rebuild_failed` with a redacted message (e.g. "a scan is in progress"); UI tells the user to wait or cancel |
| Rebuild with zero Confirmed sources | No Confirmed sources (all rejected/disabled/candidates) | Wipe proceeds (clears any leaked disabled/rejected records — first path that ever does); no rescans dispatched; response `200 {sources_rescanning: 0}`; index empty | No error |
| Disabled/rejected sources are NOT re-scanned, but their stale records ARE cleared | A Disabled/Rejected source carries leaked `memory_records` from a prior confirm/scan | Wipe deletes those leaked records; that source's `source_registry` row + lifecycle + health unchanged; it is NOT in the rescan set (not Confirmed) | No error |
| Rebuild crash mid-rescan | Wipe committed, some rescans not yet terminal, process dies | On reboot `recover_stale_runs` (`server/src/index/scan_store.rs:517`, called `lib.rs:143`) cleans partial runs; the next periodic reconcile tick / a rebuild retry re-scans the unfinished Confirmed sources; `source_registry` intact → recoverability holds (NFR-10) | Self-heal; no corruption |

</intent-contract>

## Code Map

- `server/src/index/scan_store.rs` — add `reset_derived_data(&self) -> rusqlite::Result<()>`: one `unchecked_transaction()` running the four DELETEs (mirror the structure of `recover_stale_runs` at `:517-555` and the v3 wipe at `migrations.rs:270-272`, but ADD `scan_diagnostics`). Add `any_in_flight_run(&self) -> bool` (`SELECT EXISTS(... state IN ('queued','running','staging','committing'))`), sibling to `has_in_flight_run` at `:465-479`.
- `server/src/application/rebuild.rs` (NEW) — add `rebuild_index(conn: &Connection) -> Result<Vec<SourceId>, RebuildError>`: the synchronous core — `any_in_flight_run`?→`Err(RebuildInFlight)`; `ScanStore::reset_derived_data()`; `SourceRegistry::list()` filtered to `SourceLifecycle::Confirmed` (`source_registry.rs:173-185`); return the Confirmed `SourceId` list. (Re-export from `server/src/application/mod.rs`.) `RebuildError` → envelope via the HTTP layer.
- `server/src/http/mod.rs` — add `start_rebuild(state) -> Result<Envelope<RebuildOutcome>, ErrorEnvelope>`: `lock_conn`; call `application::rebuild_index(&conn)` (wipe + Confirmed list) UNDER the mutex; then for each `SourceId` reuse the `start_rescan` dispatch path (`:366-438`) — `application::reserve_run` + `thread::spawn` worker opening its own `Connection` (`:415`) running `application::scan_reserved_source`, emitting progress into `rescan_jobs` so the existing SSE works. Release the mutex before spawning workers (reserve-all under mutex, spawn after). Return `RebuildOutcome { sources_rescanning }`.
- `server/src/http/envelope.rs` — add `rebuild_failed(message)` constructor (code `"rebuild_failed"`, safe message); it maps to 409 via the existing `respond_result` branch at `server.rs:641` (add `rebuild_failed` to the 409 arm, alongside `scan_failed`).
- `server/src/http/server.rs` — register `POST /api/index/rebuild` → `start_rebuild` in the route table at `:143-275` (next to `rescan` at `:181`).
- `src/api/index.ts` (NEW) — typed `rebuildIndex(): Promise<RebuildOutcome>` mirroring `src/api/scan.ts` (envelope/api_version validation, `asRebuildOutcome` runtime shape guard, contract-error thrower).
- `src/api/errors.ts` — add `"rebuild_failed"` to `TESSERA_STABLE_ERROR_CODES` (`:17-37`).
- `src/features/sources/Sources.tsx` — add a keyboard-reachable "Rebuild index" button in the inventory header (`:117` area); on activation reveal an inline confirm region (NOT a modal — no dialog infra exists) with a `role="alert"` warning ("Deletes only Tessera-derived index data. Confirmed sources and project mappings are kept. Source files are never modified.") + "Rebuild now"/"Cancel" buttons; on confirm call `rebuildIndex()`, show a `data-testid="rebuild-status"` polite `aria-live` region ("Rebuilding…") and poll inventory until settled; disable the button while rebuilding. Focus the confirm region on open; Esc/Cancel closes it (a11y contract `tests/ui/accessibility.spec.ts`).
- `server/tests/rebuild.rs` (NEW) — cover every I/O Matrix row + the stable-identity AC + the zero-source-mutation gate.
- `server/tests/http_api.rs` — add `POST /api/index/rebuild` HTTP test (200 outcome; 409 `rebuild_failed` when in-flight).
- `tests/ui/accessibility.spec.ts` — extend with the rebuild confirmation flow (keyboard-reachable, warning announced, focus managed, post-rebuild inventory renders).

## Tasks & Acceptance

**Execution:**
- `server/src/index/scan_store.rs` -- add `reset_derived_data` (one-tx four-DELETE wipe incl. `scan_diagnostics` + `active_generation:%`) and `any_in_flight_run` -- the AD-29 reset boundary + the in-flight race guard.
- `server/src/application/rebuild.rs` -- add `rebuild_index` (reject-if-in-flight → wipe → collect Confirmed ids) + `RebuildError`; re-export from `application/mod.rs` -- the synchronous rebuild core, separable from HTTP.
- `server/src/http/mod.rs` -- add `start_rebuild` (wipe+reserve-all under mutex, then fan out per-source rescans reusing the `start_rescan` worker path) + `RebuildOutcome` -- exposes rebuild over the loopback API with progress via existing SSE.
- `server/src/http/envelope.rs` + `server/src/http/server.rs` -- add `rebuild_failed` constructor (409) + register it in `respond_result`, and register `POST /api/index/rebuild` -- makes the action reachable and its conflict status correct.
- `src/api/index.ts` + `src/api/errors.ts` -- typed `rebuildIndex` client + register `rebuild_failed` stable code -- frontend transport mirroring the scan client.
- `src/features/sources/Sources.tsx` -- keyboard-reachable rebuild button + inline confirm region (warning + confirm/cancel) + `aria-live` rebuild status + settle polling -- the warned, a11y-compliant destructive surface the AC requires.
- `server/tests/rebuild.rs` -- new module covering the I/O Matrix, the stable-identity reproduction, the zero-source-mutation gate, atomic wipe, and the unreadable-source isolation case -- pins every rebuild behavior.
- `server/tests/http_api.rs` -- HTTP-level rebuild test (200 + 409) -- verifies the wire response and conflict envelope.
- `tests/ui/accessibility.spec.ts` -- extend with the rebuild confirm flow -- the pinned a11y artifact (AD-21) must cover the new destructive action.

**Acceptance Criteria:**
- Given at least one Confirmed Source with a successfully indexed active generation, when the user confirms the warning and triggers `POST /api/index/rebuild`, then the response is `200 {sources_rescanning: N}` AND a subsequent search/inventory for an unchanged source returns the SAME records (same `record_id` and Provenance fields) as before the rebuild — verified by a test that snapshots `record_id` + Provenance pre-rebuild and asserts equality post-rebuild (only `generation`/`observed_at` may differ).
- Given the rebuild runs, then EXACTLY `memory_records`, `scan_runs`, `scan_diagnostics`, and `tessera_meta` rows matching `key LIKE 'active_generation:%'` are empty after the wipe, while `source_registry` row count/contents, `tessera_meta.schema_version`, and `tessera_migrations_applied` are unchanged — verified by a test asserting the table boundaries directly.
- Given a rebuild is requested while any source has an in-flight scan run, when invoked, then no wipe occurs, no reservations are made, and the response is a `409` envelope with code `rebuild_failed` and a redacted message.
- Given one Confirmed Source whose root is unreadable (4.2 cause) and one healthy Confirmed Source, when the rebuild runs, then the healthy source is fully rebuilt AND the unreadable source is marked degraded/error + cause + stale per 4.2 (source-scoped isolation), AND the rebuild response is still `200` with both sources in the rescan set, AND no source FILE is modified (zero-source-mutation test passes).
- Given a Disabled or Rejected Source that leaked derived records from a prior confirm/scan, when the rebuild runs, then those leaked records are deleted by the wipe AND that source's `source_registry` row + lifecycle + health are unchanged AND it is NOT re-scanned (not Confirmed).
- Given the user opens the rebuild action in the UI, then a warning stating that only Tessera-derived data is deleted (sources and project mappings kept, source files untouched) is announced (keyboard-reachable, `role="alert"`/`aria-live`) BEFORE any destructive call, and the rebuild only proceeds on an explicit keyboard-activatable confirm — verified by extending `tests/ui/accessibility.spec.ts`.
- Given the existing 4.1/4.2/4.3 tests, when the 4.4 changes land, then all of them still pass unchanged.

## Design Notes

- **Why wipe + rescan (not a migration, not an in-place rebuild).** The v3 migration (`migrations.rs:270-272`) already proved the exact "clear derived, preserve registry" SQL; 4.4 promotes it to a repeatable runtime operation. Re-scanning via the existing pipeline (reserve→`scan_reserved_source`→`commit_cas`) reuses 4.1's fencing/CAS/snapshot-at-validation for free — no second state machine. Because `record_id` is a pure function of source data + the preserved `src_<rowid>` (`domain/scan.rs:316`), re-scan reproduces identical identity + Provenance; this is the AC's "restore stable identity and Provenance" satisfied by construction, not by a copy step.
- **Why reject-if-in-flight, not supervisor pause.** The wipe is one fast transaction holding SQLite's write lock. A scan that started BEFORE the wipe and commits AFTER it finds its `scan_runs` row deleted → `commit_cas` returns `Ok(false)` (`scan_store.rs:329-336`), its generation never activates, and its staged rows are reclaimed by the next `recover_stale_runs`/wipe. The rebuild's own full per-source re-dispatch re-scans that source anyway, so correctness holds without pausing reconcile. Rejecting when a run is already in-flight avoids the messiest case (a scan mid-pipeline with staged data) and gives the user a clear "wait/cancel and retry". SQLite serializes writes, so there is no true concurrent-corruption window; sources are never mutated regardless.
- **Why "sources unchanged" (NFR-1/10) is about Agent Memory, not the index.** The AC's "重建失败时原始 Agent Memory 保持不变" targets the source FILES (zero-source-mutation gate), NOT index transactionality. The wipe is intentionally destructive and pre-warned; a partial rebuild failure leaves a partially-rebuilt index that is fully recoverable (sources intact → rebuild again, or let reconcile self-heal). This matches AD-29's intent (the migration-atomicity clause is about schema migrations, not this runtime reset) and NFR-10 (rebuildable from Confirmed Sources alone).
- **Why reserve-all under the mutex, then spawn workers.** Reserving every Confirmed source's run inside the same critical section that wipes closes the TOCTOU window: once reserved, `has_in_flight_run` is true for those sources, so reconcile cannot start a competing scan for them. Workers then scan on their own connections (NFR-12: the request mutex is released; queries keep working).
- **Why reuse `rescan_jobs` + existing SSE, not a new channel.** The current SSE is scoped per `(source_id, job_id)` and `rescan_jobs` is keyed by source (`lib.rs:64`). Dispatching rebuild scans through the same path `start_rescan` uses makes each source's progress visible with zero new transport. The UI tracks "rebuild settled" by polling inventory until every Confirmed source's latest run is terminal.
- **Why an inline confirm region, not a modal.** `Grep` for `dialog|modal|role="dialog"` in `src/` returns zero matches — no dialog infrastructure exists. An inline confirm region (`role="alert"` warning + confirm/cancel buttons, keyboard-reachable, focus managed) satisfies the AC's "clearly inform before rebuild" and the AD-21 a11y contract without inventing modal machinery. The warning copy is inline English (no i18n regression — the app has no i18n today).
- **Known pre-existing issue (NOT in scope):** `CURRENT_SCHEMA_VERSION` in `server/src/index/mod.rs:29` declares `5` while the real applied version is `6` (`migrations.rs:424`). Do not "fix" it in this story — it is unrelated and touching it risks churn.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml --test rebuild` -- expected: all new rebuild unit tests pass.
- `cargo test --manifest-path server/Cargo.toml --test http_api rebuild` -- expected: HTTP-level rebuild test passes (200 + 409 `rebuild_failed`).
- `cargo test --manifest-path server/Cargo.toml --test scan_pipeline replaced_root_requires_reconfirmation` -- expected: 4.2 fail-closed behavior unchanged.
- `cargo test --manifest-path server/Cargo.toml --test source_registry same_path_different_inode` -- expected: 4.2 invariant green (Unix only).
- `cargo test --manifest-path server/Cargo.toml --test inventory` -- expected: all 4.2 inventory tests green.
- `cargo test --manifest-path server/Cargo.toml --no-fail-fast` -- expected: full suite green (modulo known pre-existing flaky `phase_zero_baseline_gate` / reconcile supervisor timing tests documented in 4.1/4.3 specs).
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- expected: no new warnings.
- `cargo build --manifest-path server/Cargo.toml` -- expected: clean build.
- `npm run build` -- expected: frontend type-check + Vite build clean.
- `npm run test:e2e` -- expected: Playwright suite (incl. extended rebuild a11y flow) passes.

**Manual checks (if no CLI):**
- None beyond the test suite — the rebuild surface is covered by unit + HTTP + Playwright tests.

## Review Triage Log

### 2026-07-26 — Review pass 1 (4 reviewers: adversarial, edge-case-hunter, verification-gap, intent-alignment)
- intent_gap: 0
- bad_spec: 0
- patch: 10: (high 1, medium 4, low 5)
- defer: 3 entries (worker-panic `catch_unwind`; crash-mid-rescan recovery smoke; low hardening cluster) — see `deferred-work.md`
- reject: 0
- addressed_findings:
  - `[high]` `[patch]` A — post-wipe `begin_run` failure orphaned `queued` runs and wedged all later rebuilds (409 until reboot; `recover_stale_runs` is boot-only). Fixed: `start_rebuild` collects reservations and on `begin_run` Err `fail_run`s every previously-reserved row in the same mutex hold before returning 500. Pinned by new test `start_rebuild_cleans_up_orphan_reservations_on_begin_run_failure`.
  - `[medium]` `[patch]` C — rebuild worker's failure SSE message falsely claimed "the previous index is unchanged" (the wipe cleared it). Fixed: rebuild-specific message ("…no indexed records until a later scan succeeds"); `start_rescan`'s copy untouched.
  - `[medium]` `[patch]` G — no end-to-end rebuild test through the live server (happy-path bypassed `start_rebuild`'s dispatch). Fixed: `rebuild_restores_records_via_live_inventory_poll` (POST rebuild → poll inventory → assert record count restored). Pins AC#1 at the consumption surface.
  - `[medium]` `[patch]` E — HTTP 409 test was non-deterministic (accepted 200 OR 409). Fixed: deterministic seed of a `queued` `scan_runs` row; asserts EXACTLY 409 + `rebuild_failed`.
  - `[medium]` `[patch]` F — rebuild SSE progress wiring unverified. Fixed: `rebuild_per_source_progress_surfaces_via_rescan_events_sse` (queued→running→terminal).
  - `[low]` `[patch]` D — `reset_derived_data` used unescaped `LIKE 'active_generation:%'` (`_` wildcard; spec says EXACTLY). Fixed: precise `substr(key,1,?)=?` literal-prefix match via `ACTIVE_GENERATION_KEY_PREFIX`.
  - `[low]` `[patch]` H — `RebuildError::Internal` / wipe-failure → 500 path untested. Fixed: `rebuild_returns_internal_when_wipe_fails` (+ pairs with A).
  - `[low]` `[patch]` I — zero-source-mutation gate asserted size+mtime only. Fixed: `snapshot_files` now hashes content (FNV-1a); gate asserts all four; run across the unreadable-source test (AC#4).
  - `[low]` `[patch]` J — registry-preservation tests omitted `health_cause` (4.2 health = state + cause). Fixed: full-row snapshot incl. `health_cause` across three preservation tests.
  - `[low]` `[patch]` UI — rebuild poll loop had no unmount cleanup. Fixed: `rebuildPollTokenRef` + `useEffect` cleanup + `pollToken.stop` checks in every async callback.
- deferred: worker-panic `catch_unwind` (systemic across `start_rescan` + `start_rebuild`); crash-mid-rescan recovery smoke (relies on pre-existing `recover_stale_runs`); low hardening cluster (`any_in_flight_run` NOT-IN formulation, `source_rowid` None continue, Esc window-scope, unbounded DELETE batching).

## Auto Run Result

Status: done

### Summary

Implemented Story 4.4 — the `POST /api/index/rebuild` operation that atomically wipes the Tessera-derived tables (`memory_records`, `scan_runs`, `scan_diagnostics`, `tessera_meta WHERE key = 'active_generation:*'`) in one transaction while preserving `source_registry` + `schema_version` + `tessera_migrations_applied`, then re-scans every Confirmed Source via the existing read-only scan pipeline. Stable `record_id` + Provenance are restored by construction (identity is a pure function of source data + the preserved `source_id`). Zero-source-mutation holds. Review pass 1 found 10 patch findings (1 high, 4 medium, 5 low); all applied and re-verified.

(Note: this run initially HALTed at step-03 because the host lacked an MSVC toolchain — `VS 2022` was an empty stub and GNU `/usr/bin/link.exe` shadowed the absent MSVC linker. Resolved mid-run by installing VS 2022 Build Tools + Windows SDK via choco; all verification below ran after that.)

### Files changed (13)

- `server/src/index/scan_store.rs` — `reset_derived_data` (one-tx four-DELETE wipe; precise prefix match per patch D) + `any_in_flight_run` (race guard).
- `server/src/application/rebuild.rs` (NEW) — `rebuild_index` (reject-if-in-flight → wipe → Confirmed ids) + `RebuildError`.
- `server/src/application/mod.rs` — `pub mod rebuild` + re-exports.
- `server/src/http/mod.rs` — `start_rebuild` (wipe + reserve-all under mutex → fan out per-source rescans reusing the `start_rescan` worker path; patch A orphan-cleanup on `begin_run` Err; patch C rebuild-specific failure message) + `RebuildOutcome`.
- `server/src/http/envelope.rs` — `rebuild_failed()` constructor.
- `server/src/http/server.rs` — `POST /api/index/rebuild` route + `rebuild_failed` → 409 arm.
- `server/src/lib.rs` — re-export `start_rebuild`.
- `src/api/index.ts` (NEW) — typed `rebuildIndex()` client.
- `src/api/errors.ts` — `rebuild_failed` in `TESSERA_STABLE_ERROR_CODES`.
- `src/features/sources/Sources.tsx` — keyboard-reachable rebuild button + inline `role="alert"` confirm region + `aria-live` rebuild status + settle polling (patch UI unmount cleanup).
- `server/tests/rebuild.rs` (NEW) — 13 unit tests (every I/O matrix row + stable-identity + atomic-wipe + in-flight race + zero-confirmed + leaked-disabled + unreadable-source isolation + zero-source-mutation incl. content hash + internal-wipe-fail + orphan-cleanup).
- `server/tests/http_api.rs` — 4 HTTP rebuild tests (200 outcome; deterministic 409; SSE progress; live-inventory restore).
- `tests/ui/accessibility.spec.ts` — 3 Playwright rebuild a11y tests (keyboard-reachable warning before the call; Esc closes without firing; 409 safe-alert).

### Review findings breakdown

Pass 1 (4 reviewers): 0 intent_gap, 0 bad_spec, 10 patch (high 1, medium 4, low 5) — all applied & re-verified; 3 defer entries written to `deferred-work.md`; 0 reject. See `## Review Triage Log` above.

### Follow-up review recommendation

true — pass 1 patched 1 high-severity finding (A, the post-wipe orphan wedge) and the patch score is 3×medium(4) + 1×low(5) = 17 ≥ 5. A focused re-review of the patched `start_rebuild` region is worthwhile before merge (a second automated pass was not run; the patches are pinned by targeted new tests instead).

### Verification performed (Windows host, after MSVC install)

- `cargo build --manifest-path server/Cargo.toml` — clean.
- `cargo test --manifest-path server/Cargo.toml --test rebuild` — 13/13 passed.
- `cargo test --manifest-path server/Cargo.toml --test http_api rebuild` — 4/4 passed.
- `cargo test --manifest-path server/Cargo.toml --test inventory` — 14/14 passed (4.2 inventory green).
- `cargo clippy --manifest-path server/Cargo.toml --lib --test rebuild --test http_api -- -D warnings` — clean (all 4.4 code).
- `npm run build` — clean.
- `npm run test:e2e` — 16/16 passed (incl. 3 new rebuild a11y tests; existing 13 not regressed).
- Full-suite regression vs pristine baseline `bcd590f` (clean git worktree): ZERO new failures from 4.4. Pre-existing native-Windows failures unchanged (Unix-only filesystem-identity: `replaced_root_requires_reconfirmation…`, `same_size_restored_mtime…`, `scan_indexes_only_supported_artifact_matrix`; CRLF: `fallback_line_endings…`; Unix path/env-resolver: `adapters::codex`/`claude_code` resolver+enumeration, `policy::resolver_paths_are_exactly_documented`; `source_registry::confirm_*_fingerprint`; timing-flaky `phase_zero_baseline_gate`). The 4.3 spec's "406 passed" was on a Unix toolchain.

### Residual risks

1. **Pre-existing Windows-only test/clippy failures** (not caused by 4.4; documented above). `cargo clippy --all-targets -- -D warnings` is NOT clean on Windows due to one pre-existing cfg(unix)-gated unused import in `tests/codex_canonicalization.rs:11` (clean on Unix); the 4.4 code itself is clippy-clean.
2. **Worker-panic wedge (deferred).** A panic inside a rebuild/rescan worker (no `catch_unwind`) leaves the run non-terminal → wedges later rebuilds 409 until reboot. Patch A closed the `begin_run`-failure path; the panic path is deferred (systemic across `start_rescan` too). See `deferred-work.md`.
3. **`record_id` reproduction is by construction.** Asserted in `happy_path_rebuild_restores_stable_identity_and_provenance`; survives as long as the `record_id` derivation (`domain/scan.rs`) stays a pure function of `(source_id, provider, native_locator, unit_kind)`.
4. **`followup_review_recommended: true`** — a focused re-review of the patched `start_rebuild` region is advisable before merge.

### Residual artifacts (not part of the change)

None. `git status --porcelain` after the scoped commit will show only the pre-existing uncommitted `_bmad/` framework churn (a BMAD self-update unrelated to this story) and the scratch `.claude/tmp/*` files (review diff + install log); neither is committed.
