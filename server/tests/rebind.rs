//! Story 4.3 — Rediscovery & degraded handling for path/permission/identity
//! change (spec-4-3-path-change-degraded.md).
//!
//! These tests pin every I/O matrix row of the `application::rebind_source`
//! recovery path: an old Confirmed Source (4.2 already marks it
//! `Degraded + PathMissing + last-success + stale`) is rebound to a new root,
//! producing a `Disabled` old row (retaining cause + last-success) and a new
//! `Confirmed` row at the new fingerprint. Rebind is one logical action: the
//! disable-old + insert-or-wake-new pair is wrapped in ONE SQLite transaction
//! so a mid-way failure rolls the disable back.
//!
//! Coverage:
//! - Happy rebind (root moved): old → Disabled + retained cause/last-success;
//!   new → Confirmed + Unknown + fresh source_id at the new fingerprint.
//! - Rebind to a fingerprint that already exists as a Source (wake-up): the
//!   existing row is woken to Confirmed AND its health/cause reset to
//!   Unknown/None (no stale degraded state resurrected).
//! - Rebind unknown old `source_id` → `SourceNotFound` (404 envelope).
//! - Rebind old Source in Rejected/Disabled state → `ConfirmFailed` (409).
//! - Rebind new root missing / not-a-dir → `ConfirmFailed` (409), no state
//!   change (fail-closed BEFORE touching old row).
//! - No-op rebind (same fingerprint): old row stays Confirmed, no new row.
//! - Provider-unknown defensive path → `ConfirmFailed`.
//! - AC real Given (F3): a 4.2-Degraded old row with cause + ACTIVE
//!   GENERATION (run a scan before degrading), then rebind, then assert the
//!   disabled old row RETAINS `health_cause=PathMissing` and its last-success
//!   generation pointer survives.
//! - Transaction rollback (F1): inject a failure between disable-old and
//!   insert-new; assert the old row is restored to Confirmed (no window where
//!   it is Disabled with no new Confirmed Source).
//! - `native_project` re-derivation (F7): rebind of a Claude Code source to a
//!   different project root produces a row whose `native_project` matches the
//!   NEW root's derivation, NOT the old row's value.

use std::fs;

use rusqlite::Connection;
use tempfile::tempdir;

use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    CandidateSource, CoverageLevel, DiscoveryBasis,
};
use tessera_lib::domain::source::{
    HealthCause, HealthState, SourceId, SourceLifecycle,
};
use tessera_lib::index::migrations;
use tessera_lib::index::SourceRegistry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a fresh in-memory DB and apply all migrations. Returns a connection
/// with foreign-key enforcement ON (matching boot).
fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign_keys pragma must apply");
    migrations::apply(&mut conn).expect("migrations apply on fresh db");
    conn
}

/// Build a real Codex-shaped candidate for a root path. Codex's
/// `native_project` is `None` (global store).
fn codex_candidate(root: &std::path::Path) -> CandidateSource {
    CandidateSource {
        provider: "codex".to_string(),
        root_path: root.to_string_lossy().into_owned(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    }
}

/// Build a Claude Code-shaped candidate at a `<…>/projects/<project>/memory`
/// root. The encoded `<project>` key is the parent dir name of the memory
/// root (mirroring `ClaudeCodeAdapter::discover`).
fn claude_candidate(root: &std::path::Path, project_key: &str) -> CandidateSource {
    CandidateSource {
        provider: "claude_code".to_string(),
        root_path: root.to_string_lossy().into_owned(),
        basis: DiscoveryBasis::ClaudeDefaultHome,
        coverage_level: CoverageLevel::Full,
        native_project: Some(project_key.to_string()),
    }
}

fn opencode_candidate(root: &std::path::Path, project_id: &str) -> CandidateSource {
    CandidateSource {
        provider: "opencode".to_string(),
        root_path: root.to_string_lossy().into_owned(),
        basis: DiscoveryBasis::OpencodeProjectDatabase,
        coverage_level: CoverageLevel::Full,
        native_project: Some(project_id.to_string()),
    }
}

/// Create a memories-shaped directory and return its path.
fn make_memories(parent: &std::path::Path) -> std::path::PathBuf {
    let memories = parent.join("memories");
    fs::create_dir_all(&memories).expect("create memories dir");
    memories
}

/// Confirm a Codex source at `root` and return the materialized `Source`.
fn confirm_codex(conn: &Connection, root: &std::path::Path) -> tessera_lib::domain::source::Source {
    let registry = SourceRegistry::new(conn);
    application::confirm_source(&registry, &codex_candidate(root)).expect("confirm codex")
}

// ===========================================================================
// I/O matrix — happy rebind (root moved)
// ===========================================================================

/// I/O matrix row 1 (happy rebind): an old Confirmed+Degraded+PathMissing
/// Source rebound to a new root produces a `Disabled` old row and a new
/// `Confirmed` Source at a fresh `source_id` with `Unknown` health.
#[test]
fn rebind_to_new_root_disables_old_and_inserts_new_confirmed() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    // 4.2 mark: Degraded + PathMissing on the old row (the precondition the
    // AC names — a real degraded source, not a freshly-confirmed one).
    registry
        .set_health_and_cause(
            &old.source_id,
            HealthState::Degraded,
            HealthCause::PathMissing,
        )
        .expect("mark degraded");

    let new = application::rebind_source(&registry, &old.source_id, &new_root.to_string_lossy())
        .expect("rebind");

    // New row: fresh source_id at the new fingerprint, Confirmed, Unknown
    // health, fresh cause=none.
    assert_ne!(new.source_id, old.source_id, "new row gets a fresh id");
    assert_eq!(new.lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(new.health_state, HealthState::Unknown);
    assert_eq!(new.health_cause, HealthCause::None);
    assert_eq!(
        new.normalized_root_path,
        std::fs::canonicalize(&new_root)
            .expect("canonicalize new")
            .to_string_lossy(),
        "new row points at the new canonical root"
    );
    assert_ne!(new.fingerprint, old.fingerprint);

    // Old row: Disabled, retained Degraded + PathMissing (per the spec's
    // Always rule "Old Source row is retained ... Rebind sets it to
    // Disabled"). The cause survives rebind — this is the load-bearing AC.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Disabled);
    assert_eq!(old_after.health_state, HealthState::Degraded);
    assert_eq!(old_after.health_cause, HealthCause::PathMissing);

    // Both rows preserved (no remove command exists).
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 2);
}

// ===========================================================================
// I/O matrix — wake-up (rebind to an existing fingerprint)
// ===========================================================================

/// I/O matrix row 2 (wake-up): rebind to a new root whose fingerprint matches
/// an existing Source row wakes that row to Confirmed AND resets its
/// health/cause to Unknown/None. A resurrected previously-degraded row must
/// surface as freshly-confirmed, NOT stale-degraded (spec Design Notes —
/// "Why wake-up resets health/cause").
#[test]
fn rebind_to_existing_fingerprint_wakes_row_and_resets_health_and_cause() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let shared_root = tmp.path().join("shared-memories");
    fs::create_dir_all(&shared_root).expect("shared root");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);

    let registry = SourceRegistry::new(&conn);
    // Pre-confirm the shared root — this is the row rebind will wake. Then
    // degrade it so we can prove the wake-up clears the stale state.
    let shared = application::confirm_source(&registry, &codex_candidate(&shared_root))
        .expect("confirm shared");
    registry
        .set_health_and_cause(
            &shared.source_id,
            HealthState::Degraded,
            HealthCause::PathMissing,
        )
        .expect("degrade shared");

    // Rebind old → shared_root. The shared row's fingerprint matches the new
    // root, so the wake-up branch fires.
    let woken = application::rebind_source(&registry, &old.source_id, &shared_root.to_string_lossy())
        .expect("rebind");

    // The woken row is the SHARED row (same source_id), now Confirmed + health
    // reset to Unknown + cause reset to None — NOT a resurrection of the
    // stale Degraded+PathMissing state.
    assert_eq!(woken.source_id, shared.source_id, "wake-up returns the existing row");
    assert_eq!(woken.lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(woken.health_state, HealthState::Unknown);
    assert_eq!(
        woken.health_cause,
        HealthCause::None,
        "wake-up resets health_cause so stale-degraded does not resurrect"
    );

    // Old row was disabled; shared row was woken. Exactly two rows.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 2, "no duplicate row inserted on wake-up");
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Disabled);

    // P5: assert the woken row's INVENTORY PROJECTION (AC row 5's
    // `stale`/`cause`/`last_success` check, applied to the wake-up branch).
    // The shared row was never scanned (no active generation), so after the
    // wake-up resets health→Unknown + cause→None:
    //   - `stale = (health in {degraded,error}) AND active_generation IS NOT NULL`
    //     = false (health is Unknown, AND no active generation — both clauses
    //     false). Honest: this row has no older results to be stale.
    //   - `cause = None` on the wire (health_cause=None → not surfaced).
    //   - `last_success_at = None` (no succeeded scan_runs row).
    let woken_inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == woken.source_id)
        .expect("woken row in inventory");
    assert_eq!(
        woken_inv.health_state,
        HealthState::Unknown,
        "P5: woken row surfaces Unknown health on inventory"
    );
    assert_eq!(
        woken_inv.cause,
        None,
        "P5: woken row surfaces cause=None on inventory (health was reset)"
    );
    assert!(
        !woken_inv.stale,
        "P5: woken row is NOT stale (health=Unknown, no active generation to be stale against)"
    );
    assert_eq!(
        woken_inv.last_successful_scan,
        None,
        "P5: woken row has no last_success (never scanned)"
    );
}

// ===========================================================================
// I/O matrix — unknown old source_id → SourceNotFound (404)
// ===========================================================================

/// I/O matrix row 3: rebind with an unknown `source_id` → no state change,
/// `SourceError::SourceNotFound` (maps to the 404 `source_not_found`
/// envelope).
#[test]
fn rebind_unknown_source_id_returns_source_not_found() {
    let tmp = tempdir().expect("tempdir");
    let new_root = make_memories(tmp.path());

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let bogus = SourceId("src_99999".to_string());
    let err = application::rebind_source(&registry, &bogus, &new_root.to_string_lossy())
        .expect_err("unknown id");
    assert!(matches!(err, application::SourceError::SourceNotFound));

    // No row was inserted.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 0, "no state change for an unknown id");
}

// ===========================================================================
// I/O matrix — old source not Confirmed → ConfirmFailed (409)
// ===========================================================================

/// I/O matrix row 4: rebind an old row in Rejected or already-Disabled state
/// → `SourceError::ConfirmFailed` (409 `confirm_failed`). Rebind requires a
/// confirmed-or-degraded old source — Degraded IS still Confirmed (degraded
/// is a HealthState, not a Lifecycle), so a 4.2-marked degraded row passes.
#[test]
fn rebind_old_source_not_confirmed_returns_confirm_failed() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    // Reject the old row, then attempt rebind.
    let rejected =
        application::reject_source(&registry, &codex_candidate(&old_root)).expect("reject");
    let err = application::rebind_source(
        &registry,
        &rejected.source_id,
        &new_root.to_string_lossy(),
    )
    .expect_err("rejected old source");
    assert!(matches!(err, application::SourceError::ConfirmFailed));

    // Disable a confirmed old row, then attempt rebind.
    let second_root = tmp.path().join("second-root");
    fs::create_dir_all(&second_root).expect("second root");
    let disabled_old = confirm_codex(&conn, &second_root);
    let _ = application::disable_source(&registry, &disabled_old.source_id).expect("disable");
    let err = application::rebind_source(
        &registry,
        &disabled_old.source_id,
        &new_root.to_string_lossy(),
    )
    .expect_err("disabled old source");
    assert!(matches!(err, application::SourceError::ConfirmFailed));
}

// ===========================================================================
// I/O matrix — new root missing / not-a-dir → ConfirmFailed (409), no change
// ===========================================================================

/// I/O matrix row 5: rebind to a new root that does not exist (or is not a
/// directory) fails with `SourceError::ConfirmFailed` (409), and the old row
/// is UNCHANGED (fail-closed BEFORE touching the old row).
#[test]
fn rebind_to_missing_new_root_returns_confirm_failed_and_leaves_old_unchanged() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let bogus_new = tmp.path().join("does-not-exist");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    let err = application::rebind_source(&registry, &old.source_id, &bogus_new.to_string_lossy())
        .expect_err("missing new root");
    assert!(matches!(err, application::SourceError::ConfirmFailed));

    // Old row UNCHANGED — still Confirmed (not disabled), still at its
    // original fingerprint.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(
        old_after.lifecycle_state,
        SourceLifecycle::Confirmed,
        "fail-closed: old row not touched"
    );
    assert_eq!(old_after.fingerprint, old.fingerprint);

    // No new row was inserted.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1);
}

/// A new root that is a regular file (not a directory) also fails
/// `policy::canonicalize_root` → ConfirmFailed, no state change.
#[test]
fn rebind_to_non_directory_new_root_returns_confirm_failed() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let not_a_dir = tmp.path().join("not-a-dir");
    fs::write(&not_a_dir, "i am a file").expect("write file");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    let err = application::rebind_source(&registry, &old.source_id, &not_a_dir.to_string_lossy())
        .expect_err("non-dir new root");
    assert!(matches!(err, application::SourceError::ConfirmFailed));

    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Confirmed);
}

// ===========================================================================
// I/O matrix — no-op rebind (same fingerprint)
// ===========================================================================

/// I/O matrix row 6: rebind to a new root whose fingerprint equals the old
/// Source's fingerprint is a no-op. The old row stays Confirmed, no new row
/// is inserted, and the old Source is returned.
#[test]
fn rebind_to_same_fingerprint_is_a_noop_returning_the_old_row() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    // Rebind "to the same root" — the new fingerprint equals the old one.
    let result = application::rebind_source(
        &registry,
        &old.source_id,
        &old_root.to_string_lossy(),
    )
    .expect("no-op rebind");

    assert_eq!(
        result.source_id, old.source_id,
        "no-op rebind returns the old row"
    );
    assert_eq!(result.lifecycle_state, SourceLifecycle::Confirmed);

    // Exactly one row (no spurious Disabled/Confirmed pair).
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1);
}

// ===========================================================================
// I/O matrix — provider unknown (defensive)
// ===========================================================================

/// I/O matrix row 7: if `adapter_for(old.provider)` returns None (defensive;
/// should not happen for persisted sources), rebind fails with
/// `SourceError::ConfirmFailed` AFTER canonicalizing the new root, and the
/// old row is UNCHANGED. Driven by hand-editing the persisted provider to an
/// unknown value, then attempting rebind.
#[test]
fn rebind_with_unknown_old_provider_returns_confirm_failed_defensively() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);

    // Hand-edit the persisted provider to an unknown value (simulating a
    // provider that was removed from the registry in a future migration).
    conn.execute(
        "UPDATE source_registry SET provider = 'future-provider' WHERE id = ?1",
        rusqlite::params![old.source_id.to_rowid().expect("rowid")],
    )
    .expect("hand-edit provider");

    let registry = SourceRegistry::new(&conn);
    let err = application::rebind_source(&registry, &old.source_id, &new_root.to_string_lossy())
        .expect_err("unknown provider");
    assert!(matches!(err, application::SourceError::ConfirmFailed));

    // The old row's lifecycle is preserved (Confirmed — not disabled).
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Confirmed);
}

// ===========================================================================
// AC real Given (F3) — Degraded + PathMissing old row WITH an active
// generation; cause + last-success survive rebind.
// ===========================================================================

/// AC row 1 (real Given): reconstruct the AC's actual precondition — confirm
/// a source, run a scan to populate an ACTIVE GENERATION, force 4.2's
/// degraded-marking (`set_health_and_cause(old_id, Degraded, PathMissing)`)
/// on it, THEN call rebind. Assert that the disabled old row RETAINS
/// `health_cause=PathMissing` and its last-success generation pointer
/// survives. The first implementation only tested freshly-confirmed old
/// rows — this gap is what amendment F3 closes.
#[test]
fn rebind_preserves_degraded_cause_and_last_success_generation_on_old_row() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    fs::write(old_root.join("MEMORY.md"), "# memory\nbody").expect("fixture");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    // Run a real scan to populate an ACTIVE GENERATION on the old source.
    let outcome = application::scan_source(&registry, &conn, &old.source_id)
        .expect("initial scan succeeds");
    let active_generation = outcome.generation.0.clone();
    let source_rowid = old.source_id.to_rowid().expect("rowid");

    // Sanity: the active generation pointer exists for the old source.
    let active_before: Option<String> = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = ?1",
            rusqlite::params![format!("active_generation:{source_rowid}")],
            |row| row.get(0),
        )
        .ok();
    assert_eq!(
        active_before.as_deref(),
        Some(active_generation.as_str()),
        "active generation pointer persisted before rebind"
    );

    // Force 4.2's degraded-marking (RootIdentityChanged → Degraded +
    // PathMissing). This is the AC's Given: a 4.2-Degraded row WITH an active
    // generation (the precondition the first implementation never tested).
    registry
        .set_health_and_cause(
            &old.source_id,
            HealthState::Degraded,
            HealthCause::PathMissing,
        )
        .expect("mark degraded");

    // Rebind to a new root.
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");
    let _ = application::rebind_source(&registry, &old.source_id, &new_root.to_string_lossy())
        .expect("rebind");

    // AC: the disabled old row RETAINS health_cause=PathMissing AND its
    // last-success generation pointer survives.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Disabled);
    assert_eq!(old_after.health_state, HealthState::Degraded);
    assert_eq!(
        old_after.health_cause,
        HealthCause::PathMissing,
        "AC: degraded cause survives rebind on the disabled old row"
    );

    // The active generation pointer for the old source survives (rebind
    // touches only the source row's lifecycle/health, NOT the scan_runs /
    // tessera_meta active-generation state — that's Story 4.4's rebuild
    // boundary, not 4.3's). This is the load-bearing NFR-9 invariant: the
    // old generation stays queryable.
    let active_after: Option<String> = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = ?1",
            rusqlite::params![format!("active_generation:{source_rowid}")],
            |row| row.get(0),
        )
        .ok();
    assert_eq!(
        active_after.as_deref(),
        Some(active_generation.as_str()),
        "AC: last-success generation pointer survives rebind (NFR-9)"
    );

    // And the inventory projection of the old row carries the retained cause
    // (this is the user-visible surface the AC describes).
    let inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == old.source_id)
        .expect("old row in inventory");
    assert_eq!(inv.health_state, HealthState::Degraded);
    assert_eq!(
        inv.cause,
        Some(HealthCause::PathMissing),
        "inventory surfaces the retained cause"
    );
    assert!(inv.stale, "Disabled row retaining Degraded + active gen IS stale (honest)");
}

// ===========================================================================
// Transaction rollback (F1) — inject a failure between disable and insert;
// the old row is restored to Confirmed (no catastrophic window).
// ===========================================================================

/// AC row 6 (transaction rollback): the disable-old + insert-new pair MUST be
/// wrapped in ONE SQLite transaction. A failure injected between the two
/// writes MUST roll the disable back so the old row returns to Confirmed.
///
/// This test exercises the `with_transaction` primitive directly: it does a
/// real write (disable the old row) inside the transaction, then returns
/// `Err` to simulate a failure between disable and insert. The transaction
/// must roll back the disable, so the old row stays Confirmed — no window
/// exists where the old row is Disabled with no new Confirmed Source.
#[test]
fn rebind_transaction_rolls_back_disable_when_body_fails() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    // Drive the same transaction primitive rebind uses, but inject a failure
    // AFTER the disable commit. This is exactly the catastrophic state the
    // spec's F1 amendment names: disable committed, insert never ran.
    let result: Result<(), application::SourceError> = registry.with_transaction(|tx| {
        // Disable the old row (the first write of the pair).
        tx.set_lifecycle(&old.source_id, SourceLifecycle::Disabled)
            .map_err(|_| application::SourceError::Internal)?
            .ok_or(application::SourceError::SourceNotFound)?;
        // Inject a failure between the two writes — simulates a crash, a
        // constraint violation on the upcoming INSERT, or any other mid-way
        // error. The transaction MUST roll the disable back.
        Err(application::SourceError::Internal)
    });
    assert!(matches!(result, Err(application::SourceError::Internal)));

    // AC: the old row is restored to Confirmed via the rollback. No window
    // exists where it is Disabled with no new Confirmed Source.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(
        old_after.lifecycle_state,
        SourceLifecycle::Confirmed,
        "F1: disable rolled back — old row is Confirmed, not Disabled"
    );

    // And no second row was inserted.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "no new row inserted when the body failed");
}

/// P3 fix: this is the END-TO-END rollback test through
/// `application::rebind_source` itself (the primitive-level test above pins
/// `with_transaction` in isolation; this one pins `rebind_source`'s body
/// closure — the actual code that runs in production).
///
/// To force a mid-transaction failure inside `rebind_source`'s body WITHOUT
/// a `#[cfg(test)]` injection hook, we exploit an asymmetry between the
/// registry's row-read path and its row-write path:
/// - `row_to_source` builds a `SourceId` directly from the row's rowid via
///   `SourceId::from_rowid(rowid)` (crate-internal; bypasses validation),
///   so a row inserted at a NEGATIVE rowid reads back as e.g. `"src_-5"`.
/// - `set_lifecycle` and `set_health_and_cause` translate the SourceId back
///   to a rowid via `SourceId::to_rowid()`, which rejects non-positive
///   values (`(n > 0).then_some(n)` at `domain/source.rs:73`). So
///   `"src_-5"` → `None` → the UPDATE returns 0 rows → `Ok(None)`.
/// - `rebind_source`'s wake-up branch maps `Ok(None)` to
///   `Err(SourceError::Internal)` via `.ok_or(Internal)?`, which is the
///   mid-transaction failure we want.
///
/// Setup: pre-insert a row at `id=-5` carrying the SAME fingerprint the new
/// root will canonicalize to. `rebind_source` finds it via
/// `find_by_fingerprint`, takes the wake-up branch, attempts
/// `set_lifecycle` on `"src_-5"` → `Ok(None)` → `Err(Internal)`. The
/// transaction MUST roll back the disable-old write so the old row returns
/// to Confirmed.
///
/// This faithfully exercises the production `rebind_source` body closure,
/// not just the transaction primitive.
#[test]
fn rebind_source_rolls_back_disable_when_wake_up_branch_fails_mid_transaction() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    // Compute the fingerprint the new root WILL canonicalize to, so we can
    // pre-insert a row that collides (driving the wake-up branch).
    let new_canonical = std::fs::canonicalize(&new_root).expect("canonicalize new root");
    let new_fingerprint = tessera_lib::domain::source::build_fingerprint(
        "codex",
        tessera_lib::domain::source::ROOT_KIND_DIR,
        &new_canonical,
        tessera_lib::policy::canonicalize_root(&new_root).expect("canonicalize").identity,
    );

    // Hand-insert a row at id=-5 with the new fingerprint. The negative id
    // bypasses AUTOINCREMENT (we specify the id explicitly), and
    // `row_to_source` will read it back as SourceId("src_-5") which
    // `to_rowid()` rejects — the trigger for the wake-up branch's
    // `.ok_or(Internal)?` mid-transaction failure.
    conn.execute(
        "INSERT INTO source_registry
            (id, provider, source_kind, lifecycle_state, health_state, coverage_level,
             normalized_root_path, fingerprint, native_project, health_cause)
         VALUES (-5, 'codex', 'agent_memory', 'confirmed', 'unknown', 'full',
                 ?1, ?2, NULL, 'none')",
        rusqlite::params![new_canonical.to_string_lossy(), new_fingerprint.0],
    )
    .expect("hand-insert negative-id row");

    // rebind: old is Confirmed, new root canonicalizes, fingerprint collides
    // with the pre-inserted row → wake-up branch → set_lifecycle on
    // "src_-5" returns Ok(None) → Err(Internal) → transaction must roll back
    // the disable-old write.
    let err = application::rebind_source(&registry, &old.source_id, &new_root.to_string_lossy())
        .expect_err("wake-up branch fails mid-transaction");
    assert!(
        matches!(err, application::SourceError::Internal),
        "expected Internal from the failed wake-up; got {err:?}"
    );

    // P3 / F1: the disable-old write rolled back. The old row is Confirmed
    // (NOT Disabled) — no window exists where it is Disabled with no new
    // Confirmed Source.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(
        old_after.lifecycle_state,
        SourceLifecycle::Confirmed,
        "P3: end-to-end rollback through rebind_source — old row restored to Confirmed"
    );

    // The pre-inserted row at id=-5 is unchanged (its lifecycle was never
    // flipped because set_lifecycle's UPDATE matched 0 rows).
    let preinserted: String = conn
        .query_row(
            "SELECT lifecycle_state FROM source_registry WHERE id = -5",
            [],
            |row| row.get(0),
        )
        .expect("pre-inserted row present");
    assert_eq!(preinserted, "confirmed", "pre-inserted row's lifecycle unchanged");
}

/// Direct happy-path check: the disable-old + insert-new pair runs in ONE
/// transaction. After a successful rebind, the old row's lifecycle change and
/// the new row's insertion are BOTH durable — there is no intermediate state
/// where one committed and the other did not.
#[test]
fn rebind_disable_and_insert_commit_atomically_on_success() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    let registry = SourceRegistry::new(&conn);

    let new = application::rebind_source(&registry, &old.source_id, &new_root.to_string_lossy())
        .expect("rebind");

    // Both writes durable: old is Disabled, new is Confirmed at a fresh id.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Disabled);
    let new_after = registry.get(&new.source_id).expect("db ok").expect("new row");
    assert_eq!(new_after.lifecycle_state, SourceLifecycle::Confirmed);
    assert_ne!(new.source_id, old.source_id);
}

// ===========================================================================
// native_project re-derivation (F7)
// ===========================================================================

/// AC amendment F7: rebind of a Claude Code source to a different project
/// root produces a row whose `native_project` matches the NEW root's
/// derivation, NOT the old row's value. Copying `old.native_project` to a
/// different physical root would mis-identify the new Source and corrupt any
/// future Epic-5 mapping keyed off `native_project`.
///
/// Claude Code shape: `<config>/projects/<project>/memory`. The project key
/// is the parent dir name of the memory root (mirroring
/// `ClaudeCodeAdapter::discover`).
#[test]
fn rebind_re_derives_native_project_from_new_root_for_claude_code() {
    let tmp = tempdir().expect("tempdir");
    // Old root: a Claude memory dir under `<tmp>/old-config/projects/old-project/memory`.
    // The `<config>/projects/<project>/memory` shape is what the adapter's
    // `project_memory_dirs` emits, and what `native_project_for_root`
    // recognizes (P1: only this exact trailing shape returns Some).
    let old_projects = tmp.path().join("old-config").join("projects");
    let old_project_dir = old_projects.join("old-project").join("memory");
    fs::create_dir_all(&old_project_dir).expect("old memory dir");
    // New root: a Claude memory dir under a DIFFERENT config's
    // `projects/<new-project>/memory`. The grandparent MUST be named exactly
    // `projects` (P1: the shape check requires this exact name).
    let new_projects = tmp.path().join("new-config").join("projects");
    let new_project_dir = new_projects.join("new-project").join("memory");
    fs::create_dir_all(&new_project_dir).expect("new memory dir");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let old = application::confirm_source(
        &registry,
        &claude_candidate(&old_project_dir, "old-project"),
    )
    .expect("confirm old claude");
    assert_eq!(old.native_project.as_deref(), Some("old-project"));

    let new = application::rebind_source(
        &registry,
        &old.source_id,
        &new_project_dir.to_string_lossy(),
    )
    .expect("rebind");

    // F7: native_project matches the NEW root's derivation, NOT the old row's
    // "old-project" value. The new root's project key is "new-project"
    // (extracted from the lexical path `<…>/projects/<new-project>/memory`).
    assert_eq!(
        new.native_project.as_deref(),
        Some("new-project"),
        "F7: native_project re-derived from the new root, not copied from old"
    );
    assert_ne!(
        new.native_project,
        old.native_project,
        "F7: new native_project differs from the old row's value"
    );
}

/// P1 fix end-to-end: a Claude source whose root is NOT under
/// `projects/<project>/memory` (the `autoMemoryDirectory` candidate shape,
/// which `ClaudeCodeAdapter::discover`'s `auto_memory_candidate` emits with
/// `native_project: None`) MUST keep `native_project=None` after rebind to
/// another non-project root. The previous implementation returned
/// `Some(parent_dir_name)` for every Claude root, silently mutating
/// `native_project` from `None` to `Some(<garbage parent>)` — exactly the
/// corruption F7 was filed to prevent.
#[test]
fn rebind_of_auto_memory_claude_source_keeps_native_project_none() {
    let tmp = tempdir().expect("tempdir");
    // Old root: an autoMemoryDirectory-shaped Claude source (NOT under
    // `projects/<project>/memory`). At confirm, the candidate carries
    // native_project=None (matching `auto_memory_candidate`'s emission).
    let old_root = tmp.path().join("custom-memory");
    fs::create_dir_all(&old_root).expect("old auto-mem root");
    // New root: also an autoMemoryDirectory-shaped root (a different
    // physical path, NOT under `projects/<project>/memory`).
    let new_root = tmp.path().join("moved-memory");
    fs::create_dir_all(&new_root).expect("new auto-mem root");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    // Confirm with native_project=None (the autoMemoryDirectory shape).
    let old = application::confirm_source(
        &registry,
        &claude_candidate(&old_root, "ignored-project-key-that-should-not-stick"),
    )
    .expect("confirm old auto-mem claude");
    // At confirm, native_project is taken from the candidate payload. The
    // adapter's auto_memory_candidate emits None for this shape, but
    // confirm_source trusts the candidate's native_project. We model the
    // adapter's emission faithfully by passing None via a hand-set candidate.
    // To prove the P1 fix is on the RE-DERIVATION (not the confirm path),
    // we hand-set old.native_project=None post-confirm via SQL.
    conn.execute(
        "UPDATE source_registry SET native_project = NULL WHERE id = ?1",
        rusqlite::params![old.source_id.to_rowid().expect("rowid")],
    )
    .expect("set old.native_project=None (auto-mem shape)");
    let old_refreshed = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert!(
        old_refreshed.native_project.is_none(),
        "precondition: old auto-mem row has native_project=None"
    );

    // Rebind to a new autoMemoryDirectory-shaped root.
    let new = application::rebind_source(
        &registry,
        &old.source_id,
        &new_root.to_string_lossy(),
    )
    .expect("rebind");

    // P1: the new row's native_project is None (NOT Some(<parent-dir-name>)).
    // The new root is `/.../moved-memory`, whose parent dir name is the
    // tempdir — garbage that must NOT leak into the new row.
    assert_eq!(
        new.native_project,
        None,
        "P1: autoMemoryDirectory-shaped rebind keeps native_project=None (no garbage parent)"
    );

    // Old row: Disabled, also retains native_project=None.
    let old_after = registry.get(&old.source_id).expect("db ok").expect("old row");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Disabled);
    assert!(
        old_after.native_project.is_none(),
        "old row's native_project stays None post-rebind"
    );
}

/// Codex's `native_project` is always `None` (global store). Rebind preserves
/// this: the new row's `native_project` is `None`, NOT some copied value.
#[test]
fn rebind_re_derives_native_project_as_none_for_codex() {
    let tmp = tempdir().expect("tempdir");
    let old_root = make_memories(tmp.path());
    let new_root = tmp.path().join("new-memories");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let old = confirm_codex(&conn, &old_root);
    assert!(old.native_project.is_none());

    let registry = SourceRegistry::new(&conn);
    let new = application::rebind_source(&registry, &old.source_id, &new_root.to_string_lossy())
        .expect("rebind");
    assert!(
        new.native_project.is_none(),
        "Codex native_project stays None (global store)"
    );
}

/// Direct unit test for the `native_project_for_root` helper (P1+P2 fix):
/// the helper takes the USER-SUPPLIED (lexical) path STRING (not a
/// canonicalized Path), and only returns `Some(project_key)` when the path
/// matches the project-keyed shape `<…>/projects/<project>/memory`. An
/// autoMemoryDirectory root (any other absolute path) returns `None`,
/// matching `auto_memory_candidate`'s emission. Codex returns `None`
/// regardless of path.
#[test]
fn native_project_for_root_parses_only_project_keyed_claude_shape() {
    // Claude project-keyed shape: `<config>/projects/<project>/memory`.
    let claude_root = "/home/c/.claude/projects/my-project/memory";
    assert_eq!(
        application::native_project_for_root("claude_code", claude_root),
        Some("my-project".to_string()),
        "Claude project key is the parent dir name of the memory root"
    );
    // Different project key re-derives to a different value.
    let claude_root_2 = "/home/c/.claude/projects/other-project/memory";
    assert_eq!(
        application::native_project_for_root("claude_code", claude_root_2),
        Some("other-project".to_string())
    );

    // P1 fix: an autoMemoryDirectory-shaped root (NOT under
    // `projects/<project>/memory`) returns None, NOT a garbage parent dir.
    // The adapter's `auto_memory_candidate` emits `native_project: None`
    // for this shape; re-derivation must match.
    let auto_mem_root = "/Users/c/custom-memory-dir";
    assert_eq!(
        application::native_project_for_root("claude_code", auto_mem_root),
        None,
        "P1: autoMemoryDirectory-shaped root → None (not garbage parent)"
    );
    // Even an autoMemoryDirectory path whose parent happens to be named
    // "memory" must NOT be misread: the file_name() must be `memory` AND the
    // grandparent must be `projects`.
    let auto_mem_root_2 = "/Users/c/memory/something";
    assert_eq!(
        application::native_project_for_root("claude_code", auto_mem_root_2),
        None,
        "P1: only the exact `projects/<project>/memory` shape returns Some"
    );
    // A `memory` dir not under a `projects` parent → None.
    let orphan_memory = "/Users/c/memory";
    assert_eq!(
        application::native_project_for_root("claude_code", orphan_memory),
        None,
        "P1: `memory` without a `projects` grandparent → None"
    );

    // Codex (global store) → None regardless of path.
    assert_eq!(
        application::native_project_for_root("codex", "/home/c/.codex/memories"),
        None
    );
    // An unknown provider → None (defensive; same as Codex).
    assert_eq!(
        application::native_project_for_root("future-provider", "/x/memories"),
        None
    );
}

/// P2 fix: `native_project_for_root` operates on the user-supplied LEXICAL
/// path string (the path the adapter would have seen at discover time), NOT
/// the canonicalized path. A symlinked project dir (the symlink's name is
/// "link-project", the target's name is "real-project") yields the SYMLINK's
/// name as the project key — matching `ClaudeCodeAdapter::discover`'s
/// `entry.file_name()` (which reads the lexical entry, not its resolved
/// target). Before P2, deriving from the canonical path would have produced
/// "real-project", diverging from what the adapter emitted at confirm.
#[cfg(unix)]
#[test]
fn native_project_for_root_uses_lexical_path_so_symlinked_project_keeps_symlink_name() {
    use std::path::PathBuf;
    let tmp = tempfile::tempdir().expect("tempdir");
    // Build `<tmp>/projects/real-project/memory` as the canonical target,
    // then symlink it as `<tmp>/projects/link-project` so the lexical entry
    // name is "link-project" but the canonicalized target name is
    // "real-project".
    let projects = tmp.path().join("projects");
    let real_project = projects.join("real-project");
    let real_memory = real_project.join("memory");
    std::fs::create_dir_all(&real_memory).expect("mkdir real memory");
    let link_project = projects.join("link-project");
    std::os::unix::fs::symlink(&real_project, &link_project).expect("symlink project");

    // The lexical path the user supplies (and that the adapter would see):
    // `<tmp>/projects/link-project/memory`.
    let lexical: PathBuf = link_project.join("memory");
    let lexical_str = lexical.to_string_lossy();
    let derived = application::native_project_for_root("claude_code", &lexical_str);
    assert_eq!(
        derived,
        Some("link-project".to_string()),
        "P2: project key from LEXICAL path is the symlink name, matching the adapter's entry.file_name()"
    );
    assert_ne!(
        derived.as_deref(),
        Some("real-project"),
        "P2: rebind does NOT use the canonicalized target name"
    );
}

#[test]
fn rebind_re_derives_exact_current_opencode_metadata() {
    let tmp = tempdir().expect("tempdir");
    let old_root = tmp.path().join("old-opencode");
    let new_root = tmp.path().join("new-opencode");
    fs::create_dir_all(&old_root).expect("old root");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let old = application::confirm_source(&registry, &opencode_candidate(&old_root, "old-project"))
        .expect("confirm old opencode");

    let new = application::rebind_source_with_opencode_identity_resolver(
        &registry,
        &old.source_id,
        &new_root.to_string_lossy(),
        |root| {
            assert_eq!(root, new_root);
            Some(Some("current-project".to_string()))
        },
    )
    .expect("rebind");

    assert_eq!(new.native_project.as_deref(), Some("current-project"));
    assert_ne!(new.source_id, old.source_id);
    let old_after = registry.get(&old.source_id).expect("db").expect("old");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Disabled);
}

#[test]
fn rebind_missing_opencode_identity_fails_before_mutation() {
    let tmp = tempdir().expect("tempdir");
    let old_root = tmp.path().join("old-opencode");
    let new_root = tmp.path().join("new-opencode");
    fs::create_dir_all(&old_root).expect("old root");
    fs::create_dir_all(&new_root).expect("new root");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let old = application::confirm_source(&registry, &opencode_candidate(&old_root, "old-project"))
        .expect("confirm old opencode");

    let result = application::rebind_source_with_opencode_identity_resolver(
        &registry,
        &old.source_id,
        &new_root.to_string_lossy(),
        |_| None,
    );

    assert!(matches!(
        result,
        Err(application::SourceError::ConfirmFailed)
    ));
    let old_after = registry.get(&old.source_id).expect("db").expect("old");
    assert_eq!(old_after.lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(registry.list().expect("list").len(), 1);
}
