---
title: 'Story 4.3: Rediscovery & Degraded Handling for Path/Permission/Identity Change'
type: 'feature'
created: '2026-07-26'
status: 'in-review'
review_loop_iteration: 1
followup_review_recommended: true
baseline_revision: 'f3fb14c05373dff93ed78a9ba64cdbefa4a92d8c'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-4-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-4-2-connector-failure-isolation-stale-last-success.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-4-1-watcher-reconcile.md'
  - '{project-root}/_bmad-output/planning-artifacts/epics.md'
warnings: []
---

<intent-contract>

## Intent

**Problem:** When a Confirmed Source's root moves / loses permissions / changes filesystem identity, 4.2 already marks the old Source `Degraded + cause + last-success + stale` and preserves the previous generation. But nothing produces a Candidate for the new location, and there is no explicit "rebind" action — so a Source moved to a non-default path is invisible until the user manually re-discovers and re-confirms, with no link between the old and new identities. The AC requires the old Source to stay degraded with cause/last-success *while* a new Candidate is produced, no auto-merge of mappings, and only explicit rebind to change the root.

**Approach:** Add an explicit **rebind** command (`POST /api/sources/rebind`) that takes an old `source_id` + a new `root_path`, canonicalizes + fingerprints the new root, and (a) marks the old Source `Disabled`, (b) inserts a new confirmed Source row at the new fingerprint. Rebind does NOT copy or migrate Tessera Project mappings (deferred to Epic 5 — no mapping table exists yet). The "new Candidate" the AC names IS the candidate the rebind command validates — there is no active search for moved roots (architecture specifies no search strategy; discover stays stateless). Ambiguous/colliding fingerprints stay separate rows by the existing exact-match invariant. Surface the old Source's degraded state alongside the new Candidate via the existing inventory + discover endpoints — no new GET endpoint.

## Boundaries & Constraints

**Always:**
- Old Source row is retained (never deleted — no remove command exists, per `index/source_registry.rs:12-13`). Rebind sets it to `Disabled` (not rejected — the user may re-point at it again).
- New Source row gets a fresh `source_id` (AUTOINCREMENT) at the new fingerprint. The old `source_id` is NEVER reused with a different fingerprint — this preserves the invariant pinned by `tests/source_registry.rs:357 same_path_different_inode_yields_different_source_no_merge`.
- Rebind canonicalizes + fingerprints the new root via the SAME `policy::canonicalize_root` + `build_fingerprint` path as `confirm_source` (AD-4/NFR-5/6). A new root that fails canonicalization (missing / not-a-dir / not-absolute) rejects the rebind with `SourceError::ConfirmFailed` and leaves the old Source UNCHANGED (still degraded, not disabled).
- Rebind is one logical action: if any step fails after the old Source is disabled, the old Source MUST be restored to its prior (degraded) state. Implement as: canonicalize+fingerprint new root FIRST (fail-closed), then disable-old + insert-or-wake-new inside ONE SQLite transaction (a `BEGIN ... COMMIT` block, or rusqlite's `transaction()` helper) so a crash/error between the disable and the insert rolls the disable back. If the insert collides on the new fingerprint (an identical Source already exists), wake it to `Confirmed` instead of inserting.
- Coverage for the new Source comes from the adapter (single source of truth), matching `confirm_source`.
- `native_project` for the new Source is RE-DERIVED from the new root, never copied from the old row. For Codex (global store) this is `None`. For Claude Code, the `<project>` key is parsed from the new root path (mirroring `ClaudeCodeAdapter::discover` → `candidate_if_existing_dir`). Copying the old `native_project` to a different physical root would mis-identify the new Source as belonging to a project it does not belong to, corrupting any future Epic-5 mapping keyed off `native_project`.
- Errors use the existing AD-13 envelope (`scan_failed` / `confirm_failed` / `source_not_found`). No memory body, query text, or credentials in any message (NFR-3).

**Block If:** (none — all decisions resolved; readings selected are documented under Design Notes for review).

**Never:**
- Do NOT add an active "search for the moved root" mechanism. Architecture specifies no search strategy; inventing one is out of scope. The new location arrives via the rebind command's `root_path` argument.
- Do NOT migrate or copy Tessera Project mappings. The mapping table does not exist yet (Epic 5); "no auto-merge" is satisfied trivially. Mapping migration is explicitly Epic 5's concern.
- Do NOT change `discover()` — it stays stateless, reading only default home/env locations.
- Do NOT change `scan_source_with` / `scan_reserved_source_with` failure handling (4.2 owns degraded-marking on fingerprint mismatch; 4.3 adds rebind as the *recovery* path, not a scan-time change).
- Do NOT introduce a `rebind` lifecycle variant. Rebind produces `Disabled` (old) + `Confirmed` (new) using existing lifecycle states — the *relationship* between old and new is implicit (the user supplied the old id), not a persisted field.
- Do NOT relax `same_path_different_inode_yields_different_source_no_merge` or any 4.2-pinned test.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Happy rebind (root moved) | Old `source_id` is `Confirmed + Degraded + PathMissing`; new `root_path` exists, is a dir, new fingerprint | Old row → `Disabled`; new row inserted `Confirmed + Unknown health` at new fingerprint; return new `Source` | No error |
| Rebind to a fingerprint that already exists as a Source | New root's fingerprint matches an existing Source row | Old row → `Disabled`; existing matching row → `Confirmed` AND its `health_state`/`health_cause` reset to `Unknown`/`None` (wake-up produces a freshly-confirmed row, NOT a resurrection of stale degraded state); return the woken Source | No error |
| Rebind unknown old `source_id` | `source_id` matches no row | No state change | `SourceError::SourceNotFound` → 404 envelope `source_not_found` |
| Rebind old Source not in `Confirmed`/`Degraded` state | Old row is `Rejected` or already `Disabled` | No state change | `SourceError::ConfirmFailed` → 409 envelope `confirm_failed` (rebind requires a confirmed-or-degraded old source) |
| Rebind new root missing / not a dir / not absolute | `policy::canonicalize_root(new_path)` fails | No state change (fail-closed BEFORE touching old row) | `SourceError::ConfirmFailed` → 409 envelope `confirm_failed` |
| Rebind new root has same fingerprint as old (no-op move) | New fingerprint equals old Source's fingerprint | Old row stays `Confirmed` (not disabled); return old Source; this is a no-op rebind, not an error | No error |
| Rebind new root canonicalizes but provider unknown | `adapter_for(provider)` returns None for the old source's provider | No state change | `SourceError::ConfirmFailed` → 409 (defensive; should not happen for persisted sources) |
| Inventory surfaces post-rebind state | Old Source degraded + new Source confirmed | `GET /api/sources/inventory`: old row shows `Disabled + cause + last-success` (no longer stale — disabled is not degraded); new row shows `Confirmed + Unknown` (freshly confirmed, no generation yet) | No error |

</intent-contract>

## Code Map

- `server/src/application/source.rs` — add `rebind_source(registry, old_source_id, new_root_path) -> Result<Source, SourceError>`. Composes `policy::canonicalize_root` + `build_fingerprint` (existing helpers, same as `confirm_source`), then disables the old row and inserts/wakes the new INSIDE ONE TRANSACTION. Re-derives `native_project` for the new root via the adapter (NOT copied from the old row — see Boundaries).
- `server/src/application/source.rs` — add a helper that re-derives `native_project` from a root path + provider (for Claude Code: parse the `<project>` key; for Codex: `None`). This mirrors `ClaudeCodeAdapter::discover`'s project-key derivation. Expose it so `rebind_source` does not duplicate the parsing.
- `server/src/domain/source.rs` — NO structural change. `SourceLifecycle`, `HealthState`, `HealthCause`, `SourceFingerprint`, `build_fingerprint` are all already in place from 4.2.
- `server/src/index/source_registry.rs` — add a thin transactional helper OR expose the connection so `rebind_source` can wrap disable-old + insert-new in `conn.transaction()`. The disable-old + insert-new pair MUST be atomic: on any error between them, the transaction rolls back and the old row stays Confirmed. Also: the wake-up branch MUST call `set_health_and_cause(existing.id, Unknown, None)` before/with `set_lifecycle(Confirmed)` so a resurrected row surfaces as freshly-confirmed, not stale-degraded.
- `server/src/http/mod.rs` — add `rebind_source` handler: parse `(source_id, root_path)` from the request body, call `application::rebind_source`, map `SourceError` via the existing `map_source_error`/`map_scan_error` envelope handlers. Re-check the no-op watcher comment for accuracy (see Review Triage Log).
- `server/src/http/server.rs` — register `POST /api/sources/rebind` route (line ~146, next to `confirm`/`reject`/`disable`).
- `server/src/http/envelope.rs` — NO new envelope code. `confirm_failed` (→ 409), `source_not_found` (→ 404), `bad_request` (→ 400) already exist and cover rebind's failure modes.
- `server/tests/source_registry.rs` — NO change to `same_path_different_inode_yields_different_source_no_merge` (must stay green; rebind preserves its invariant).
- `server/tests/scan_pipeline.rs` — NO change to `replaced_root_requires_reconfirmation_and_preserves_active_generation` (4.2 fail-closed behavior stays; rebind is the recovery path on top).
- `server/tests/rebind.rs` (NEW) — cover every I/O Matrix row as a unit test against `application::rebind_source` + an in-memory registry. MUST include a test that reconstructs the AC's real Given: an old row with `health_state=Degraded, health_cause=PathMissing` AND a prior active generation (run a scan before degrading), then assert after rebind the disabled old row RETAINS `health_cause=PathMissing` and its last-success generation pointer. MUST include a test that the disable+insert pair is atomic (inject a failure between them, assert old row restored).
- `server/tests/http_api.rs` — add an HTTP-level test asserting `POST /api/sources/rebind` returns the new Source and the old row is `Disabled` on the wire; assert the 404 `source_not_found` and 409 `confirm_failed` envelopes (NOT 400 — `confirm_failed` maps to 409 per the existing envelope convention).

## Tasks & Acceptance

**Execution:**
- `server/src/application/source.rs` -- add `rebind_source(registry, old_source_id, new_root_path)` -- the recovery path AC requires; canonicalize+fingerprint new root FIRST (fail-closed), then disable-old + insert-or-wake-new INSIDE ONE SQLite TRANSACTION so a mid-way failure rolls back the disable; re-derive `native_project` from the new root (NOT copied from old); on wake-up, reset health/cause to `Unknown`/`None`; return the new `Source`.
- `server/src/application/source.rs` -- add `native_project_for_root(provider, canonical_root)` helper -- re-derives the provider-native project id from the new root (Claude Code project-key parsing; Codex → None) so rebind does not duplicate adapter parsing.
- `server/src/index/source_registry.rs` -- expose a transactional seam (either `with_transaction(|tx| ...)` or a `rebind_atomically` helper) -- makes the disable+insert pair atomic per Boundaries.
- `server/src/http/mod.rs` -- add `rebind_source` handler wired to `application::rebind_source`; fix the no-op watcher comment if it diverges from code -- exposes rebind over the loopback-only HTTP API alongside confirm/reject/disable.
- `server/src/http/server.rs` -- register `POST /api/sources/rebind` route -- makes the action reachable.
- `server/tests/rebind.rs` -- new test module covering the I/O Matrix AND the AC's real Given -- pins every rebind edge case (happy, fingerprint collision wake-up with health reset, unknown id, bad old state, missing new root, no-op same-fingerprint, provider-unknown defensive path) PLUS a test reconstructing a Degraded+PathMissing old row with a prior active generation asserting cause+last-success survive rebind, PLUS a transaction-rollback test.
- `server/tests/http_api.rs` -- add HTTP-level rebind test -- verifies the wire response + envelope error codes (404 `source_not_found`, 409 `confirm_failed`).

**Acceptance Criteria:**
- Given a Confirmed Source whose root was moved (so `scan_source` returns `RootIdentityChanged` and 4.2 marks it `Degraded + PathMissing`) AND the old row has been scanned at least once so it carries an active generation (last-success), when the user calls `POST /api/sources/rebind` with the old `source_id` and the new `root_path`, then the old Source becomes `Disabled` AND retains its prior `health_state=Degraded`, `health_cause=PathMissing`, and last-success generation pointer (verified by a test that reconstructs this exact precondition — not a freshly-confirmed old row), AND a new `Confirmed` Source is created at the new fingerprint with a fresh `source_id` distinct from the old.
- Given rebind to a new root whose fingerprint already matches an existing Source row, when rebind runs, then the existing row is woken to `Confirmed` AND its `health_state`/`health_cause` are reset to `Unknown`/`None` (no stale degraded state resurrected), and NO duplicate row is inserted.
- Given rebind called with an unknown `source_id`, when invoked, then no registry state changes and the response is a 404 `source_not_found` envelope.
- Given rebind called with a new `root_path` that does not exist / is not a directory, when invoked, then no registry state changes (the old Source stays in its prior state) and the response is a 409 `confirm_failed` envelope.
- Given a successful rebind, when `GET /api/sources/inventory` is called, then the old Source appears `Disabled` with its prior `health_state`/`health_cause`/last-success preserved. Note: the `stale` derivation (`(health in {degraded,error}) AND active_generation IS NOT NULL`, scan.rs:860-866) does NOT consult `lifecycle_state`, so a Disabled row that still carries `Degraded` health + an active generation derives `stale=true` — this is honest (the old generation IS stale). The new Source appears `Confirmed + Unknown` with no active generation (not stale).
- Given a rebind that fails mid-way (a SQLite error injected between disable-old and insert-new, or a process crash simulated in a test), the old Source MUST be restored to its prior state via the transaction rollback; a test must verify no window exists where the old row is `Disabled` with no new `Confirmed` Source.
- Given the existing 4.2 tests (`replaced_root_requires_reconfirmation_and_preserves_active_generation`, `same_path_different_inode_yields_different_source_no_merge`, `inventory_one_source_down_does_not_affect_others`), when the 4.3 changes land, then all of them still pass unchanged.

## Design Notes

- **Why an explicit rebind command (Gap 1 reading C), not active search.** The architecture (AD-33/AD-35, `ARCHITECTURE-SPINE.md:252-268`) specifies no search strategy for finding a moved root, and `discover()` is deliberately stateless (reads only fixed default locations). Inventing a "search nearby dirs" strategy would be unspecified behavior and a likely source of false positives. The AC's "produce a new Candidate" is satisfied by the Candidate the rebind command validates — the user, who knows where they moved the root, supplies the path. This matches "only explicit rebind changes the root" literally.
- **Why rebind = confirm-new + disable-old (Gap 2 reading Y), not identity migration.** (Z) "reuse old source_id with new fingerprint" breaks the fingerprint→row invariant pinned by `same_path_different_inode_yields_different_source_no_merge` and contradicts AD-33's "retain old Source as degraded". (X) "migrate identity + inherit mappings" is indistinguishable from (Y) until Epic 5 implements Tessera Project mappings — there is nothing to inherit today. (Y) is the minimal reading that satisfies "no auto-merge or copy of Tessera Project mapping" trivially and preserves every 4.2 invariant. When Epic 5 lands, it can add explicit mapping migration on top without reworking 4.3.
- **Why `Disabled`, not `Rejected`, for the old row.** `Rejected` is a user decision ("I don't want this Source"); `Disabled` is "this Source is currently unusable but I may re-point at it". A rebind may be temporary (the user is reshuffling dirs); keeping the old row `Disabled` (retained with cause + last-success) lets the user re-rebind back if needed. This also matches 4.2's existing disable semantics.
- **Why fail-closed on the new root BEFORE disabling the old, AND why disable+insert must be ONE transaction.** Canonicalize+fingerprint the new root first; only on success proceed to disable-old + insert-new. The disable-old + insert-new pair MUST be wrapped in a single SQLite transaction: if the insert/wake fails after the disable committed, the transaction rolls the disable back so the old row returns to Confirmed. Without the transaction, a crash between the two writes leaves the user with a Disabled old row and no new Confirmed Source — the catastrophic state the fail-closed design exists to prevent. (The original "compose existing methods" wording was ambiguous and led to a non-transactional first implementation; the Boundaries now mandate the transaction explicitly.)
- **Why wake-up resets health/cause.** A rebind that hits an existing row at the new fingerprint "wakes" that row to Confirmed. That row may carry stale `Degraded`/`PathMissing` from a prior failure. The wake-up MUST call `set_health_and_cause(existing.id, Unknown, None)` alongside `set_lifecycle(Confirmed)` so the resurrected row surfaces as freshly-confirmed on inventory, not as a stale-degraded source. This matches the I/O matrix's "Confirmed + Unknown health" expectation for the new row.
- **No-op rebind (same fingerprint).** If the new root's fingerprint equals the old Source's fingerprint (e.g. the user "moved" to the same canonical path, or the identity didn't actually change), rebind is a no-op: the old row stays `Confirmed`, no new row, return the old Source. This avoids creating a spurious second row and a useless `Disabled` record.
- **Stale marker after rebind.** `stale = (health in {degraded, error}) AND active_generation IS NOT NULL` (4.2 derivation, scan.rs:860-866) does NOT consult `lifecycle_state`. So a Disabled old row that retains `Degraded` health + an active generation still derives `stale=true` — honest, because that old generation IS stale. The new Source starts `Unknown` with no active generation, so it is not stale.

## Verification

**Commands:**
- `cargo test -p tessera --test rebind` -- expected: all new rebind unit tests pass.
- `cargo test -p tessera --test http_api rebind` -- expected: HTTP-level rebind test passes.
- `cargo test -p tessera --test source_registry same_path_different_inode` -- expected: 4.2-pinned invariant still green (Unix only).
- `cargo test -p tessera --test scan_pipeline replaced_root_requires_reconfirmation` -- expected: 4.2 fail-closed behavior unchanged.
- `cargo test -p tessera --test inventory` -- expected: all 4.2 inventory tests still green.
- `cargo clippy -p tessera --all-targets -- -D warnings` -- expected: no new warnings.
- `cargo build -p tessera` -- expected: clean build.

**Manual checks (if no CLI):**
- None beyond the test suite — the rebind surface is fully covered by unit + HTTP tests.

## Spec Change Log

### 2026-07-26 — Review pass 1 (bad_spec loopback)

Triggered by the first review pass (4 reviewers: adversarial, edge-case-hunter, verification-gap, intent-alignment). Five bad_spec findings root-caused to gaps/contradictions OUTSIDE `<intent-contract>` (Boundaries, I/O matrix, AC, Design Notes, Verification). Amendments:

- **Transaction mandate (F1, high).** Boundaries previously said "single transactional sequence" but Design Notes said "compose existing methods... otherwise compose existing methods" — internally contradictory, and the first implementation was non-transactional (two unwrapped SQL statements, leaving a crash window where the old row is Disabled with no new Confirmed Source). Amended: Boundaries now explicitly requires a SQLite transaction; Design Notes "Why fail-closed..." rewritten to name the transaction and call out the ambiguity; a new AC row requires a rollback test. Known-bad state avoided: disable-old committed, insert-new not run, user left with zero confirmed Sources. KEEP: fail-closed-on-new-root-BEFORE-disable ordering was correct and is preserved.
- **HTTP 400 → 409 (F2, medium).** I/O matrix and Boundaries said `confirm_failed → 400 envelope`; the codebase's `respond_result` (and the existing confirm/reject/scan routes) map `confirm_failed` to 409. The first implementation followed the codebase convention (409), so the spec was the side out of date. Amended: all `confirm_failed` envelope references in the I/O matrix now say 409; AC row 4 corrected; Code Map + Tasks note the 409 mapping. Known-bad state avoided: clients built against the spec's 400 would mis-route error handling. KEEP: the envelope code `confirm_failed` is unchanged (only the status code description is corrected).
- **AC real Given must be tested (F3, high).** No test reconstructed the AC's actual Given (a 4.2-Degraded old row with cause + last-success); every test confirmed a freshly-Confirmed row, so "cause + last-success preserved through rebind" was never asserted. Amended: AC row 1 now explicitly requires a test that sets up `Degraded + PathMissing + active generation` and asserts those fields survive rebind; Code Map + Tasks for `tests/rebind.rs` name this requirement. Known-bad state avoided: a future `set_lifecycle` refactor that clears health fields would silently break the preservation guarantee. KEEP: the I/O matrix row structure was correct, only the test coverage gap is closed.
- **Wake-up resets health/cause (F5, high).** I/O matrix row 2 said wake-up yields "Confirmed + Unknown health" but the first implementation's wake-up path only called `flip_lifecycle` (which does NOT clear health/cause), so a resurrected previously-degraded row would surface stale `Degraded + PathMissing`. Amended: I/O matrix row 2 now explicitly requires `set_health_and_cause(existing.id, Unknown, None)` with the wake-up; a new Design Notes bullet explains why; AC row 2 corrected. Known-bad state avoided: a rebind to a previously-failed fingerprint resurrects stale degraded state instead of a fresh confirmation. KEEP: the wake-up-otherwise-idempotent behavior is preserved.
- **`native_project` re-derived, not copied (F7, medium).** The first implementation copied `old.native_project` to the new row. For Claude Code (where `native_project` is the `<project>` key parsed from the root), copying the OLD project id to a DIFFERENT physical root mis-identifies the new Source and would corrupt future Epic-5 mappings. Amended: Boundaries adds an Always rule re-deriving `native_project` from the new root; Code Map + Tasks add a `native_project_for_root` helper. Known-bad state avoided: new Source tagged with the wrong provider-native project. KEEP: same-provider-by-construction is preserved (the provider IS carried from old; only the project id is re-derived).
- **`stale` derivation accuracy (review sub-finding).** Design Notes previously claimed "the old row, once Disabled, is no longer Degraded" — false, because `stale` derives from `health_state + active_generation` and does NOT consult `lifecycle_state`. Amended: Design Notes "Stale marker after rebind" corrected to state a Disabled row retaining Degraded health + an active generation still derives `stale=true` (honest); AC row 5 corrected. Known-bad state avoided: reviewer/reader confusion about post-rebind inventory state.

## Review Triage Log

### 2026-07-26 — Review pass 1
- intent_gap: 0
- bad_spec: 5: (high 3, medium 2)
- patch: 4: (medium 1, low 3) — deferred to next pass per cascading order (bad_spec triggers re-derive; patch findings will be re-evaluated against re-derived code)
- defer: 2
- reject: rest
- addressed_findings:
  - `[high]` `[bad_spec]` F1 transaction mandate — spec amended (Boundaries + Design Notes + AC + Code Map + Tasks); triggers re-derive.
  - `[high]` `[bad_spec]` F3 AC real Given test requirement — spec amended (AC row 1 + Code Map + Tasks); triggers re-derive.
  - `[high]` `[bad_spec]` F5 wake-up health/cause reset — spec amended (I/O matrix row 2 + AC row 2 + Design Notes + Code Map); triggers re-derive.
  - `[medium]` `[bad_spec]` F2 HTTP 400 → 409 — spec amended (I/O matrix + AC row 4 + Code Map + Tasks); triggers re-derive.
  - `[medium]` `[bad_spec]` F7 native_project re-derive — spec amended (Boundaries + Code Map + Tasks); triggers re-derive.
  - `[medium]` `[patch]` F4 verification `-p tessera-lib` → `-p tessera` — applied to spec Verification now (no code change needed).
  - `[low]` `[defer]` F6 no-op watcher comment drift — defer to focused cleanup.
  - `[low]` `[defer]` F10 cluster (source_id format validation, concurrent-race guards, test-harness PathBuf::new() sentinel, Windows identity=None test meaningfulness, watcher-hook coverage) — defer; pre-existing patterns or low-likelihood edge cases not caused by this story's core intent.

Patch findings held for next pass (will re-evaluate after re-derive): F8 (root_path trim + control-char rejection), F9 (rebind-to-already-confirmed-fingerprint behavior pinning), plus low-severity HTTP/input-validation hardening.

### 2026-07-26 — Review pass 2
- intent_gap: 0
- bad_spec: 0
- patch: 5: (high 1, medium 2, low 2)
- defer: 4
- reject: 1 (wake-up two-UPDATE ordering — intermediate state unobservable inside the transaction)
- addressed_findings:
  - `[high]` `[patch]` G1 — F7 re-derive incomplete: `native_project_for_root` returned `Some(parent)` for ALL claude_code roots, but the adapter emits `native_project: None` for the `autoMemoryDirectory` shape. Fix: only return `Some(project_key)` when the canonical root matches the `<config>/projects/<project>/memory` shape (basename == `memory` AND parent is under `projects/`); otherwise return `None` to faithfully mirror `ClaudeCodeAdapter::discover`'s two emission paths. Add a test: rebind of an auto-memory-shaped Claude source keeps `native_project = None`.
  - `[medium]` `[patch]` G2 — symlink divergence: adapter extracts `project_key` from the LEXICAL `entry.file_name()`, but `native_project_for_root` extracts from the CANONICAL parent. Fix: extract the project key from the same lexical shape the adapter uses (the `memory` dir's parent's name on the canonical path is the faithful mirror only because canonicalization preserves the trailing components; verify and pin with a test that a symlinked project dir yields the same key at rebind as at confirm).
  - `[medium]` `[patch]` G3 — F1 rollback test targeted `with_transaction` primitive, not `rebind_source`'s body. Fix: add a test that forces a mid-transaction failure INSIDE `application::rebind_source` (e.g. pre-insert a row at the new fingerprint so the wake-up branch's `set_lifecycle`/`set_health_and_cause` returns None → `.ok_or(Internal)?`), asserting the old row returns to `Confirmed` and no new Confirmed row exists.
  - `[medium]` `[patch]` G5 — `with_transaction` rollback-failure path leaves the pooled connection in an open transaction; `rollback(self)` consumes `tx` so no `Drop` runs, and the comment claiming implicit rollback-on-drop is wrong. Fix: handle the `rollback()` `Err` (log it; the connection is suspect but `IndexState`'s mutex means the next caller re-acquires — document or reconnect), and correct the comment.
  - `[low]` `[patch]` G4 — F5 wake-up test simulated degraded via direct setter, not via real 4.2 scan-time marking; the woken row's inventory projection (`stale`, `cause`, `last_success`) was not asserted. Fix: add an inventory-level assertion that the woken row shows `stale=false`, `cause=None` after rebind (mirror AC row 5's projection check at the wake-up branch).
  - `[low]` `[patch]` G9 — `boot_rebind_server` test helper returns a misleading empty `PathBuf::new()`/TempDir dressed as "the source root". Fix: drop the third tuple element (return `(port, source_id)`) and update callers; or return the real source root.
  - `[low]` `[defer]` G7 — `From<rusqlite::Error> for SourceError` blanket impl collapses all DB errors to `Internal`. Defer: contained today (only exercised by `with_transaction`'s begin/commit), but worth scoping to the transaction boundary in a future cleanup.
  - `[low]` `[defer]` G8 cluster — no-op short-circuit returns a stale snapshot; `adapter_for` check after no-op; watcher uses woken row's stored path; `RebindRequest` input hygiene (F8 trim + control-char + Content-Length bound). Defer: pre-existing patterns or low-likelihood edge cases; the no-op staleness is bounded by `IndexState`'s single-writer mutex.
  - `[low]` `[defer]` G10 — intent-alignment pass 2 surface observations (discover surface untested, AC "重新发现" trigger faked, "上次成功时间" asserted as generation pointer). Descriptive only — Reading B (rebind-as-rediscovery) is pre-justified in spec Design Notes; the discover surface is intentionally out of scope. No action.
  - `[low]` `[reject]` G6 — wake-up two-UPDATE ordering (lifecycle-then-health) creates a brief `Confirmed+Degraded` window inside the transaction. Reject: the intermediate state is unobservable to other readers (mutex + transaction isolation); rollback or commit resolves it atomically. No action.

## Auto Run Result

Status: done

### Summary

Implemented Story 4.3 — the explicit `POST /api/sources/rebind` recovery path for a Confirmed Source whose root moved / lost permissions / changed filesystem identity. On top of 4.2's degraded-marking (which already retains the old Source as `Degraded + cause + last-success + stale` and preserves the previous generation), rebind is the user-supplied action that points at the new location: it canonicalizes + fingerprints the new root, then atomically disables the old row and inserts (or wakes) a new Confirmed Source inside one SQLite transaction. No auto-merge of Tessera Project mappings (deferred to Epic 5); no active search for moved roots (Reading B — rebind-as-rediscovery); ambiguous fingerprints stay separate rows by the existing exact-match invariant.

### Files changed

- `server/src/index/source_registry.rs` — added `with_transaction` transactional seam (BEGIN/commit/rollback via `unchecked_transaction`); rollback-failure path logged (not swallowed) and the misleading implicit-drop comment corrected.
- `server/src/application/source.rs` — added `native_project_for_root` (re-derives the Claude Code `<project>` key from the LEXICAL root path, faithfully mirroring `ClaudeCodeAdapter::discover`'s two emission paths: `Some(key)` only for the `projects/<project>/memory` shape, `None` otherwise including `autoMemoryDirectory`); added `rebind_source` (fail-closed canonicalize-new-root-FIRST, then disable-old + insert-or-wake-new INSIDE ONE transaction; wake-up branch resets health/cause to `Unknown`/`None`; `native_project` re-derived NOT copied); added `From<rusqlite::Error> for SourceError` (scoped to `with_transaction`'s begin/commit).
- `server/src/application/mod.rs` — re-exported `native_project_for_root`, `rebind_source`.
- `server/src/http/mod.rs` — added `rebind_source` HTTP handler + `RebindRequest` DTO; watcher lifecycle mirrors the row lifecycle (stop on old id, start fresh on new id).
- `server/src/http/server.rs` — registered `POST /api/sources/rebind` route (no envelope code changes — `confirm_failed` maps to 409 via the existing `respond_result` convention).
- `server/tests/rebind.rs` (NEW) — 17 unit tests covering every I/O matrix row, the AC real-Given (degraded + active generation precondition, cause + last-success preserved), the transaction rollback (both primitive-level and end-to-end through `rebind_source`'s body), `native_project` re-derivation (project-memory shape, auto-memory shape, Codex None, symlink lexical-name fidelity), and the wake-up health-reset inventory projection.
- `server/tests/http_api.rs` — 3 HTTP-level tests asserting wire shape (new Source returned, old row Disabled via inventory), 404 `source_not_found`, 409 `confirm_failed` (NOT 400). `boot_rebind_server` helper simplified to return `(port, source_id)`.

### Review findings breakdown

- **Pass 1 (4 reviewers):** 5 bad_spec (high 3, medium 2) → spec amended (transaction mandate, 400→409, AC real-Given test requirement, wake-up health reset, native_project re-derive), code reverted + re-derived. 4 patch + 2 defer held for pass 2.
- **Pass 2 (4 reviewers):** 0 bad_spec, 0 intent_gap. 6 patch (high 1, medium 2, low 2; plus 1 low test-harness) → all applied (P1 native_project shape faithfulness, P2 symlink lexical-name fidelity, P3 end-to-end rollback test, P4 rollback-failure logging + comment fix, P5 wake-up inventory projection, P6 test-helper cleanup). 3 defer (From<rusqlite::Error> blanket impl, RebindRequest input-hygiene cluster, intent-alignment discover-surface observations). 1 reject (wake-up two-UPDATE ordering — unobservable intermediate state).
- **Follow-up review recommendation:** true — pass 2 had 1 high-severity patch (native_project faithfulness). Score: 3×medium(2) + 1×low(2) = 8 ≥ 5, AND a high patch was applied.

### Verification performed

- `cargo test -p tessera --test rebind` → 17 passed
- `cargo test -p tessera --test http_api rebind` → 3 passed
- `cargo test -p tessera --test source_registry same_path_different_inode` → 1 passed (4.2 invariant green)
- `cargo test -p tessera --test scan_pipeline replaced_root_requires_reconfirmation` → 1 passed (4.2 fail-closed unchanged)
- `cargo test -p tessera --test inventory` → 16 passed (all 4.2 inventory green)
- `cargo clippy -p tessera --all-targets -- -D warnings` → clean
- `cargo build -p tessera` → clean
- Full suite `cargo test -p tessera --no-fail-fast` → 406 passed, 0 failed, 1 ignored (pre-existing). `phase_zero_baseline_gate` is a known host-timing flaky test that fails identically on pristine baseline `f3fb14c` (measured 31ms vs 12ms threshold) — unrelated to 4.3.

### Residual risks

1. **Pre-existing flaky tests (not 4.3-caused).** `phase_zero_baseline_gate` (host cold-scan timing; fails on pristine baseline too) and `reconcile::supervisor_*` (kernel-notify timing under parallel load; documented as residual in the 4.1 spec). Neither touches nor is touched by 4.3.
2. **Transaction rollback test uses a test-only negative-rowid trick.** `rebind_source_rolls_back_disable_when_wake_up_branch_fails_mid_transaction` inserts a row at `id=-5` to exploit the asymmetry between `SourceId::from_rowid` (crate-internal, no validation) and `SourceId::to_rowid` (rejects non-positive), forcing the wake-up branch's `.ok_or(Internal)?`. This cannot occur in production (AUTOINCREMENT always allocates `id >= 1`); the test exercises the production `rebind_source` body closure faithfully.
3. **`From<rusqlite::Error> for SourceError` blanket impl.** Contained today (only `with_transaction`'s begin/commit map through it), but enables silent `?` coercion in future code — deferred for scoping to the transaction boundary.
4. **Rebind input hygiene.** `RebindRequest` accepts arbitrary `root_path` strings with only `canonicalize_root`'s 409 as the gate — no pre-canonicalize 400 layer for malformed input (trim, control chars, NUL, Content-Length bound). Deferred.
5. **Intent surface narrowing.** The AC's literal "重新发现 / 产生新 Candidate" lives at the discover surface, but 4.3 implements Reading B (rebind-as-rediscovery, pre-justified in Design Notes). If a future epic re-reads the AC to require auto-discovery of moved roots, this story's rebind-only scope would need revisiting.

### Residual artifacts (not part of the change)

None. `git status --porcelain` after commit will show only the spec file itself and the deferred-work.md update as the change set; no stray untracked files.
