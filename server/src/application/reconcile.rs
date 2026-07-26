//! `application::reconcile` — file-change watcher hint ingestion and bounded
//! reconcile auto-refresh (Story 4.1).
//!
//! ## Intent (binding contract — see spec-4-1-watcher-reconcile.md)
//!
//! Watcher events are HINTS ONLY. The hint path writes NONE of
//! `memory_records`, `scan_runs`, or `tessera_meta.active_generation` (A-12).
//! Reconcile IS the existing scan pipeline: `trigger_reconcile` reserves a run
//! (the synchronous request mutex is held only for the `begin_run` reservation)
//! and then spawns a worker thread that opens its OWN `rusqlite::Connection`
//! and reuses [`application::scan_reserved_source`] — exactly the same pattern
//! as [`http::start_rescan`](crate::http::start_rescan). There is NO second
//! canonical mutation path; add/modify/delete fall out of the existing
//! atomic generation switch (AD-5/AD-34/AD-36).
//!
//! ## Layout
//!
//! - [`ReconcileSupervisor`] owns one [`notify::RecommendedWatcher`] per
//!   confirmed source root, plus a per-source debounce/coalesce state, plus the
//!   periodic reconcile timer. Held by `IndexState` and dropped on shutdown.
//! - [`trigger_reconcile`] is the shared reservation+spawn callable used by BOTH
//!   the HTTP rescan path and the watcher/periodic reconcile path, so the two
//!   surfaces can never diverge into two mutation paths (spec task — "factor
//!   `start_rescan`'s begin_run+spawn into a shared callable").
//! - The [`HintQueue`] is the in-memory hint accumulator. The `notify` callback
//!   (which runs on a notify-internal thread) ONLY touches this queue — it never
//!   acquires the synchronous request mutex and never touches canonical tables
//!   (A-12 by construction).
//!
//! ## Threading recap (binding constraint, per deferred-work)
//!
//! The transport is tiny_http with one thread per connection, a synchronous
//! handler, and a `std::sync::Mutex<Connection>`. The rescan worker is the ONE
//! sanctioned exception: it opens its own `Connection` and relies on the
//! fencing-token CAS. Reconcile reuses that exact pattern. No async path
//! exists.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;

use crate::application;
use crate::domain::scan::{Generation, ScanError};
use crate::domain::source::{SourceId, SourceLifecycle};
use crate::index::scan_store::ScanStore;
use crate::index::SourceRegistry;
use crate::IndexState;

/// Default debounce window for watcher hints. A burst of edits within this
/// window collapses to ONE reconcile. Tunable via [`ReconcileConfig`].
///
/// Kept small so a single edit reflects in queries quickly; the periodic tick
/// is the safety net for anything missed. This is a tunable with a documented
/// sane default, NOT a human-input decision (spec Block If).
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// Default periodic reconcile interval. Each tick reconciles any confirmed
/// source not currently reconciling. This is the self-heal for dropped/missed
/// `notify` events (AD-8) AND acts as the initial reconcile at boot.
///
/// 60 seconds. This is the value `main.rs` ships (it constructs
/// `ReconcileConfig::default()` with no override), tuned for fast feedback on
/// Carver's small single-machine dataset: a missed `notify` event is repaired
/// within one minute, and the boot-time first tick (which fires immediately,
/// not after a full period) validates every confirmed source's index on
/// startup. Tunable via [`ReconcileConfig::with_period`] for deployments that
/// want a different cadence.
pub const DEFAULT_PERIOD: Duration = Duration::from_secs(60);

/// Per-source debounce + coalesce state.
///
/// `pending` is set true by the notify callback; the debounce thread (or the
/// periodic tick) drains it. `in_flight` is set true while a reconcile worker
/// is running for this source, so a hint that arrives mid-reconcile is
/// remembered and retried on the next periodic tick (spec I/O matrix — "Same
/// source, hint while reconcile in-flight").
#[derive(Debug, Default, Clone, Copy)]
struct SourceHintState {
    /// A hint has been observed since the last reconcile started for this
    /// source. The debounce window is elapsed via `queued_at`.
    pending: bool,
    /// When the most recent pending hint was observed (for the debounce
    /// window). `None` while no hint is pending.
    queued_at: Option<Instant>,
    /// A reconcile worker is currently running for this source.
    in_flight: bool,
}

/// The in-memory hint queue. The notify callback ONLY touches this; it never
/// acquires the synchronous request mutex (A-12 by construction). Reconcile
/// drains it under its own lock and then enters the existing mutation path.
///
/// Wrapped in a `Mutex` and shared between the notify callback thread(s), the
/// periodic tick thread, and the supervisor's public API (start/stop watch on
/// source lifecycle transitions).
#[derive(Debug, Default)]
pub struct HintQueue {
    sources: Mutex<HashMap<SourceId, SourceHintState>>,
}

impl HintQueue {
    /// Construct an empty hint queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a hint for `source_id` (called from the notify callback on a
    /// notify-internal thread). Sets `pending = true` and stamps `queued_at`.
    /// Idempotent: a burst within the debounce window overwrites `queued_at`
    /// but leaves `pending = true` (one hint per source until drained). NEVER
    /// touches canonical tables — A-12 by construction.
    ///
    /// Exposed as `pub` (with `#[doc(hidden)]`) so the Story 4.1 integration
    /// tests can drive hint ingestion without a live notify backend.
    #[doc(hidden)]
    pub fn record_hint(&self, source_id: &SourceId) {
        let mut sources = self.sources.lock().expect("hint queue mutex poisoned");
        let state = sources.entry(source_id.clone()).or_default();
        state.pending = true;
        state.queued_at = Some(Instant::now());
    }

    /// Mark `source_id` as having a reconcile in-flight. Called by the
    /// supervisor right before spawning the worker so a concurrent hint can be
    /// remembered for the next tick.
    fn mark_in_flight(&self, source_id: &SourceId) {
        let mut sources = self.sources.lock().expect("hint queue mutex poisoned");
        let state = sources.entry(source_id.clone()).or_default();
        state.in_flight = true;
    }

    /// Clear `in_flight` (worker finished) and re-arm `pending` if a hint
    /// arrived during the run (so the next periodic tick retries it). Public
    /// because the reconcile loop (this module) and integration tests both
    /// call it; there is no internal-only visibility tier that fits both.
    pub fn clear_in_flight(&self, source_id: &SourceId) {
        let mut sources = self.sources.lock().expect("hint queue mutex poisoned");
        if let Some(state) = sources.get_mut(source_id) {
            state.in_flight = false;
        }
    }

    /// Drop ALL hint state for `source_id` (pending, queued_at, in_flight).
    /// Called by the loop when a reservation fails permanently (source is no
    /// longer confirmed / not found) so the hint is not re-armed into an
    /// orphan-hint retry storm. Mirrors what [`ReconcileSupervisor::stop_watch`]
    /// does to the queue on a source-lifecycle transition away from confirmed.
    pub fn drop_hint(&self, source_id: &SourceId) {
        let mut sources = self.sources.lock().expect("hint queue mutex poisoned");
        sources.remove(source_id);
    }

    /// Drain the set of sources whose debounce window has elapsed and that are
    /// not currently in-flight. Returns the source ids to reconcile and marks
    /// them in-flight. Sources with a pending hint whose window has NOT yet
    /// elapsed stay queued (they will be drained on a later tick or by the
    /// debounce-pass in [`run_reconcile_loop`]).
    ///
    /// `force_all` skips the debounce-window check, yielding every queued
    /// source that is not in-flight. NOTE: this only drains sources ALREADY
    /// in the queue — it does NOT enumerate the registry. The production
    /// periodic tick uses [`due_for_periodic_tick`] (which reads the registry
    /// directly) for its AD-8 self-heal force-reconcile, NOT this method with
    /// `force_all=true`. This method's `force_all` path is exercised by tests
    /// that pre-seed the queue; production never passes `force_all=true`
    /// here.
    ///
    /// Exposed as `pub` (with `#[doc(hidden)]`) so integration tests can
    /// exercise debounce/coalesce behavior without a live notify backend.
    #[doc(hidden)]
    pub fn drain_due(&self, debounce: Duration, force_all: bool) -> Vec<SourceId> {
        let mut sources = self.sources.lock().expect("hint queue mutex poisoned");
        let now = Instant::now();
        let mut due = Vec::new();
        for (source_id, state) in sources.iter_mut() {
            if state.in_flight {
                continue;
            }
            let due_by_time = state
                .queued_at
                .is_some_and(|queued| now.duration_since(queued) >= debounce);
            if force_all || (state.pending && due_by_time) {
                state.pending = false;
                state.queued_at = None;
                state.in_flight = true;
                due.push(source_id.clone());
            }
        }
        due
    }

    /// Snapshot the (source_id, queued_at) pairs so the debounce-pass can sleep
    /// until the next due hint. Used only by [`ReconcileSupervisor::run`].
    fn next_due_in(&self, debounce: Duration) -> Duration {
        let sources = self.sources.lock().expect("hint queue mutex poisoned");
        let now = Instant::now();
        let mut earliest: Option<Duration> = None;
        for state in sources.values() {
            if !state.pending || state.in_flight {
                continue;
            }
            if let Some(queued) = state.queued_at {
                let elapsed = now.duration_since(queued);
                let remaining = debounce.saturating_sub(elapsed);
                earliest = Some(earliest.map_or(remaining, |e| e.min(remaining)));
            }
        }
        earliest.unwrap_or(debounce)
    }

    /// How many sources currently carry a pending hint. Diagnostic / test
    /// accessor — production code does not read this (the loop drains
    /// opportunistically and the periodic tick force-reconciles regardless).
    #[doc(hidden)]
    pub fn pending_count(&self) -> usize {
        let sources = self.sources.lock().expect("hint queue mutex poisoned");
        sources.values().filter(|state| state.pending).count()
    }

    /// Is a hint currently pending for `source_id`? Diagnostic / test accessor.
    #[doc(hidden)]
    pub fn has_pending_hint(&self, source_id: &SourceId) -> bool {
        let sources = self.sources.lock().expect("hint queue mutex poisoned");
        sources
            .get(source_id)
            .is_some_and(|state| state.pending)
    }

    /// Remove a source's entry from the queue. Equivalent to [`Self::drop_hint`];
    /// kept under this name where the test is asserting on the queue's
    /// membership rather than the production lifecycle path. Diagnostic / test
    /// accessor.
    #[doc(hidden)]
    pub fn remove(&self, source_id: &SourceId) {
        let mut sources = self.sources.lock().expect("hint queue mutex poisoned");
        sources.remove(source_id);
    }
}

/// Tunables for the reconcile supervisor. Both fields are documented sane
/// defaults; they are NOT human-input decisions (spec Block If).
#[derive(Debug, Clone, Copy)]
pub struct ReconcileConfig {
    /// Per-source debounce window. A burst of edits within this window
    /// collapses to one reconcile.
    pub debounce: Duration,
    /// Periodic reconcile interval. Each tick reconciles every confirmed
    /// source not currently reconciling (self-heal for missed events — AD-8).
    pub period: Duration,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
            period: DEFAULT_PERIOD,
        }
    }
}

impl ReconcileConfig {
    /// Override the debounce window.
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// Override the periodic interval.
    pub fn with_period(mut self, period: Duration) -> Self {
        self.period = period;
        self
    }
}

/// One per-source watcher entry. Dropping the [`RecommendedWatcher`] stops
/// delivery of further hints for its root (the notify backend unregisters its
/// kernel watch on Drop).
#[derive(Debug)]
struct WatchEntry {
    /// The notify watcher. Held so its lifetime is bound to this entry; the
    /// kernel watch is unregistered when the entry (and thus the watcher) is
    /// dropped.
    _watcher: RecommendedWatcher,
}

/// The supervisor owns:
/// - one `RecommendedWatcher` per confirmed source root (watcher lifetime
///   mirrors source lifecycle: confirm → start; unconfirm/delete → stop),
/// - the shared [`HintQueue`] drained by the reconcile loop,
/// - the periodic reconcile timer (the self-heal for dropped/missed notify
///   events — AD-8),
/// - a stop flag that the debounce/periodic loop checks between iterations.
///
/// Held by `IndexState`; dropped on shutdown. The notify backends unregister
/// their kernel watches on Drop, and the worker threads observe the stop flag
/// on their next iteration.
#[derive(Debug)]
pub struct ReconcileSupervisor {
    /// Per-source watcher entries. Mutated under `watches` lock.
    watches: Mutex<HashMap<SourceId, WatchEntry>>,
    /// Shared hint queue (also referenced by the worker threads via Arc).
    queue: Arc<HintQueue>,
    /// Shared state handle. The workers open their own connection per run; the
    /// supervisor only needs the path and the synchronous-reservation lock.
    state: Arc<IndexState>,
    /// Stop flag for the debounce/periodic loop.
    stop: Arc<AtomicBool>,
    /// Handles for the supervisor threads so Drop can join them cleanly.
    threads: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl ReconcileSupervisor {
    /// Construct and START the supervisor. After this returns:
    /// - Watchers are started for every currently-confirmed source (boot
    ///   recovery — spec I/O matrix "Boot with confirmed sources").
    /// - The periodic reconcile loop is running (its first tick fires almost
    ///   immediately, validating each confirmed source's index against disk).
    ///
    /// The supervisor borrows the shared `IndexState` for its whole lifetime;
    /// the caller (lib.rs `boot`) keeps the `Arc<IndexState>` and drops the
    /// supervisor on shutdown.
    pub fn start(state: Arc<IndexState>, config: ReconcileConfig) -> std::io::Result<Self> {
        let queue = Arc::new(HintQueue::new());
        let watches = Mutex::new(HashMap::new());
        let stop = Arc::new(AtomicBool::new(false));

        // Capture the loop tunables before moving `config` into the struct.
        let debounce = config.debounce;
        let period = config.period;

        // Boot: start watches for every currently-confirmed source.
        let supervisor = Self {
            watches,
            queue: Arc::clone(&queue),
            state: Arc::clone(&state),
            stop: Arc::clone(&stop),
            threads: Mutex::new(Vec::new()),
        };
        supervisor.boot_start_watches()?;

        // Periodic + debounce loop. Splits each iteration into:
        // 1. Force-drain at boot (one-shot) and on every period boundary: every
        //    confirmed source not currently reconciling is reconciled (AD-8
        //    self-heal — full re-enumeration repairs any missed event).
        // 2. Otherwise, drain only hints whose debounce window has elapsed.
        let supervisor_state = Arc::clone(&state);
        let supervisor_queue = Arc::clone(&queue);
        let supervisor_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("tessera-reconcile".to_string())
            .spawn(move || {
                run_reconcile_loop(
                    &supervisor_state,
                    &supervisor_queue,
                    &supervisor_stop,
                    debounce,
                    period,
                );
            })?;
        supervisor.threads.lock().expect("threads lock").push(handle);

        Ok(supervisor)
    }

    /// Boot-time: start a watcher for every confirmed source. Errors are
    /// log-and-continue per the spec I/O matrix ("Boot with confirmed sources"
    /// → "Log-and-continue if a watcher fails to start; periodic reconcile
    /// still covers it"). A failed watcher does not block boot.
    fn boot_start_watches(&self) -> std::io::Result<()> {
        let sources = {
            let conn = self
                .state
                .conn
                .lock()
                .map_err(|_| std::io::Error::other("synchronous request mutex poisoned"))?;
            let registry = SourceRegistry::new(&conn);
            match registry.list() {
                Ok(sources) => sources,
                Err(e) => {
                    eprintln!("tessera: reconcile supervisor: list_sources failed at boot: {e:?}");
                    return Ok(());
                }
            }
        };
        for source in sources {
            if source.lifecycle_state != SourceLifecycle::Confirmed {
                continue;
            }
            // The root is the persisted normalized (canonical) root path.
            let root_path = Path::new(&source.normalized_root_path);
            if let Err(e) =
                self.start_watch_internal(&source.source_id, &source.provider, root_path)
            {
                eprintln!(
                    "tessera: reconcile supervisor: watcher start failed for {} ({}): {e:?}; periodic reconcile still covers it",
                    source.source_id, source.normalized_root_path
                );
            }
        }
        Ok(())
    }

    /// Start a watcher for `source_id` rooted at `canonical_root`. Idempotent:
    /// re-starting for an already-watched source replaces the prior watcher.
    /// Public surface used by the source-lifecycle hook (confirm → start).
    pub fn start_watch(
        &self,
        source_id: &SourceId,
        provider: &str,
        canonical_root: &Path,
    ) -> std::io::Result<()> {
        self.start_watch_internal(source_id, provider, canonical_root)
    }

    fn start_watch_internal(
        &self,
        source_id: &SourceId,
        provider: &str,
        canonical_root: &Path,
    ) -> std::io::Result<()> {
        let queue = Arc::clone(&self.queue);
        let captured_source = source_id.clone();
        let captured_provider = provider.to_string();
        let captured_root = canonical_root.to_path_buf();
        // The notify callback runs on a notify-internal thread. It ONLY records
        // a hint — it never acquires the synchronous request mutex, never
        // touches canonical tables (A-12 by construction).
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<notify::Event>| {
                if let Ok(event) = result {
                    record_event_hint_if_relevant(
                        &queue,
                        &captured_source,
                        &captured_provider,
                        &captured_root,
                        &event,
                    );
                }
            },
            notify::Config::default(),
        )
        .map_err(std::io::Error::other)?;
        let recursive_mode = recursive_mode_for_provider(provider);
        watcher
            .watch(canonical_root, recursive_mode)
            .map_err(std::io::Error::other)?;
        let entry = WatchEntry { _watcher: watcher };
        let mut watches = self.watches.lock().expect("watches lock");
        // Replacing an existing entry drops the prior watcher, which
        // unregisters its kernel watch.
        watches.insert(source_id.clone(), entry);
        Ok(())
    }

    /// Stop the watcher for `source_id` (drops the kernel watch). Public
    /// surface used by the source-lifecycle hook (unconfirm/delete → stop).
    pub fn stop_watch(&self, source_id: &SourceId) {
        let removed = {
            let mut watches = self.watches.lock().expect("watches lock");
            watches.remove(source_id)
        };
        // `removed` drops here → kernel watch unregistered.
        drop(removed);
        // Also clear any pending hint for this source so a stale hint cannot
        // trigger a reconcile after the source is no longer watched.
        self.queue.drop_hint(source_id);
    }

    /// Synchronously record a hint for `source_id` without going through
    /// `notify`. Useful for tests that need to drive the debounce/reconcile
    /// path deterministically without waiting for kernel event delivery.
    /// Does NOT touch canonical tables (A-12).
    #[doc(hidden)]
    pub fn record_hint_sync(&self, source_id: &SourceId) {
        self.queue.record_hint(source_id);
    }

    /// Reference to the shared hint queue. Diagnostic / test accessor.
    #[doc(hidden)]
    pub fn queue(&self) -> &Arc<HintQueue> {
        &self.queue
    }

    /// Reference to the shared IndexState. Diagnostic / test accessor.
    #[doc(hidden)]
    pub fn state(&self) -> &Arc<IndexState> {
        &self.state
    }
}

fn recursive_mode_for_provider(provider: &str) -> RecursiveMode {
    if provider == crate::adapters::opencode::OpenCodeAdapter::PROVIDER_ID {
        RecursiveMode::NonRecursive
    } else {
        RecursiveMode::Recursive
    }
}

fn record_event_hint_if_relevant(
    queue: &HintQueue,
    source_id: &SourceId,
    provider: &str,
    canonical_root: &Path,
    event: &notify::Event,
) {
    let relevant = provider != crate::adapters::opencode::OpenCodeAdapter::PROVIDER_ID
        || event
            .paths
            .iter()
            .any(|path| path == &canonical_root.join("AGENTS.md"));
    if relevant {
        queue.record_hint(source_id);
    }
}

impl Drop for ReconcileSupervisor {
    fn drop(&mut self) {
        // Signal the loop thread to stop on its next iteration, then join so a
        // clean shutdown waits for any in-flight iteration to finish. The
        // worker threads spawned per-reconcile are NOT joined here: they open
        // their own connection and finish on their own; on process exit the OS
        // reaps them. Joining every worker would risk blocking shutdown on a
        // slow FS scan.
        self.stop.store(true, Ordering::SeqCst);
        let mut threads = self.threads.lock().expect("threads lock");
        for handle in threads.drain(..) {
            let _ = handle.join();
        }
    }
}

/// The periodic + debounce reconcile loop. Splits each iteration:
/// - On the period boundary (and at boot), force-reconcile: enumerate every
///   confirmed source from the registry and reconcile any that is not
///   currently in-flight (full re-enumeration — AD-8 self-heal). This is the
///   self-heal for dropped/missed `notify` events AND the boot validation.
/// - Between period boundaries, drain only hints whose debounce window has
///   elapsed (burst collapses to one reconcile).
fn run_reconcile_loop(
    state: &Arc<IndexState>,
    queue: &Arc<HintQueue>,
    stop: &AtomicBool,
    debounce: Duration,
    period: Duration,
) {
    // The first tick fires almost immediately so boot validates each
    // confirmed source's index against disk (spec I/O matrix — "first periodic
    // reconcile validates its index").
    let mut next_periodic = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let now = Instant::now();
        let on_period_boundary = now >= next_periodic;
        if on_period_boundary {
            // Reset the period boundary AFTER this iteration's force-reconcile.
            next_periodic = now + period;
        }
        // Decide which sources to reconcile this iteration.
        let due = if on_period_boundary {
            // Force-reconcile: enumerate every confirmed source from the
            // registry and reconcile any not currently in-flight. The hint
            // queue's in-flight flags gate which ones are skipped.
            due_for_periodic_tick(state, queue)
        } else {
            // Debounce path: drain only hints whose window has elapsed.
            queue.drain_due(debounce, false)
        };
        for source_id in due {
            // Reservation holds the synchronous request mutex only for the
            // begin_run; the worker opens its own connection for the FS work.
            // Errors here are log-and-continue; the dispatch below decides
            // whether to retry next tick or drop the hint permanently. Pass the
            // queue so the worker clears `in_flight` when it finishes —
            // otherwise the next tick would skip this source forever.
            match trigger_reconcile_with_hint_queue(
                source_id.clone(),
                state,
                Some(Arc::clone(queue)),
            ) {
                Ok(()) => {}
                Err(TriggerError::AlreadyRunning { .. }) => {
                    // Another owner (HTTP rescan or a prior reconcile) is
                    // already in-flight. The worker never started, so it would
                    // never clear `in_flight` — clear it here so the next tick
                    // can retry once the current owner finishes. Do NOT log:
                    // this is the common case whenever a rescan outlasts one
                    // period.
                    queue.clear_in_flight(&source_id);
                }
                Err(TriggerError::ReservationFailed(reason)) => {
                    // Decide whether this is a permanent or transient failure.
                    // Permanent ("source not found" / "source is not
                    // confirmed"): drop the hint entirely — re-arming would
                    // retry every debounce window forever (an orphan-hint
                    // retry storm, ~120 log lines/min per source until
                    // restart). Transient (mutex poisoned / DB / spawn):
                    // clear in_flight and re-arm so the next tick retries.
                    let permanent =
                        reason.contains("not found") || reason.contains("not confirmed");
                    if permanent {
                        eprintln!(
                            "tessera: reconcile: dropping hint for {source_id} ({reason}); source is no longer reconcile-eligible"
                        );
                        queue.drop_hint(&source_id);
                    } else {
                        eprintln!(
                            "tessera: reconcile: reservation failed for {source_id} ({reason}); periodic tick will retry"
                        );
                        queue.clear_in_flight(&source_id);
                        queue.record_hint(&source_id);
                    }
                }
            }
        }
        // Sleep until either the next hint's debounce elapses or the next
        // period boundary, whichever is sooner. Stop flag is checked at the
        // top of the next iteration.
        let sleep_for = if on_period_boundary {
            // Just force-reconciled everything; sleep toward the next period.
            period.min(debounce)
        } else {
            queue.next_due_in(debounce).min(period)
        };
        // Bound the sleep so the stop flag latency stays small.
        let sleep_for = sleep_for.min(Duration::from_secs(1));
        thread::sleep(sleep_for);
    }
}

/// Enumerate every confirmed source from the registry and return the subset
/// that is not currently in-flight. This is the force-reconcile path used on
/// every period boundary (AD-8 self-heal) and at boot (first-tick validation).
///
/// The registry read happens under the synchronous request mutex; the returned
/// source ids are then reconciled outside the mutex via the shared
/// [`trigger_reconcile`] callable. Sources already in-flight (a hint-driven
/// reconcile is running) are skipped this tick — they'll be picked up next
/// period.
fn due_for_periodic_tick(state: &IndexState, queue: &HintQueue) -> Vec<SourceId> {
    let sources = {
        let conn = match state.conn.lock() {
            Ok(conn) => conn,
            Err(_) => return Vec::new(),
        };
        let registry = SourceRegistry::new(&conn);
        match registry.list() {
            Ok(sources) => sources,
            Err(e) => {
                eprintln!("tessera: reconcile: registry list failed: {e:?}");
                return Vec::new();
            }
        }
    };
    // Mark each confirmed source as in-flight (so two ticks in a row don't
    // double-reserve), and yield those not already in-flight.
    let mut due = Vec::new();
    let mut sources_map = queue.sources.lock().expect("hint queue mutex poisoned");
    for source in sources {
        if source.lifecycle_state != SourceLifecycle::Confirmed {
            continue;
        }
        let state = sources_map.entry(source.source_id.clone()).or_default();
        if state.in_flight {
            continue;
        }
        state.pending = false;
        state.queued_at = None;
        state.in_flight = true;
        due.push(source.source_id.clone());
    }
    due
}

/// The failure modes of [`trigger_reconcile`] and [`reserve_run`].
/// Reservation-time only; the worker's own failure path runs asynchronously and
/// is not surfaced here.
#[derive(Debug)]
pub enum TriggerError {
    /// Another reconcile/rescan is already in-flight for this source. The
    /// AD-5/16/28/32 "single fenced owner per source" invariant is enforced at
    /// the [`reserve_run`] chokepoint: when a non-terminal `scan_runs` row
    /// already exists for this source, a new reservation returns this variant
    /// WITHOUT allocating a new run. Both the HTTP `start_rescan` path and the
    /// watcher reconcile path pass through the same chokepoint, so neither can
    /// start a second concurrent owner for the same source. The caller decides
    /// how to react: the HTTP path maps it to `bad_request`; the supervisor
    /// loop re-arms the hint so the next periodic tick retries once the
    /// in-flight run finishes.
    AlreadyRunning { source_id: SourceId },
    /// The `begin_run` reservation itself failed (DB error, or the source
    /// lookup rejected it — non-confirmed / unknown id). Carries a short
    /// reason string for log-and-continue.
    ReservationFailed(&'static str),
}

/// The synchronous-mutex reservation block, factored out so BOTH the HTTP
/// rescan path ([`crate::http::start_rescan`]) and the watcher/periodic
/// reconcile path ([`run_reconcile_loop`] via [`trigger_reconcile`]) share ONE
/// reservation code path. This is the spec task "factor `start_rescan`'s
/// begin_run+spawn into a shared callable" — there is ONE mutation path; HTTP
/// rescan and watcher reconcile cannot diverge.
///
/// Holds the synchronous request mutex ONLY for the validation +
/// single-owner check + `begin_run` call. Returns the reserved
/// `(scan_id, fencing_token, generation)` ownership triple on success.
///
/// **Single-owner gate (AD-5/16/28/32):** BEFORE `begin_run`, this checks
/// [`ScanStore::has_in_flight_run`] for the source. If a non-terminal run
/// already exists, the reservation returns [`TriggerError::AlreadyRunning`]
/// WITHOUT allocating a new fencing token. This is the one shared chokepoint
/// both HTTP rescan and watcher reconcile pass through, so two owners can never
/// run the same source concurrently. Without this gate, a watcher reconcile
/// starting while an HTTP rescan is in-flight would allocate a higher token,
/// win the CAS, and leave the HTTP-triggered run stuck in `committing` until
/// the next boot — surfacing as `get_scan_status` reporting `committing`
/// indefinitely for any rescan that takes longer than one period.
pub fn reserve_run(
    source_id: &SourceId,
    state: &Arc<IndexState>,
) -> Result<(i64, i64, Generation), TriggerError> {
    let conn = state
        .conn
        .lock()
        .map_err(|_| TriggerError::ReservationFailed("synchronous request mutex poisoned"))?;
    let registry = SourceRegistry::new(&conn);
    let source = registry
        .get(source_id)
        .map_err(|_| TriggerError::ReservationFailed("registry lookup failed"))?;
    match source {
        None => {
            return Err(TriggerError::ReservationFailed("source not found"))
        }
        Some(source)
            if source.lifecycle_state != SourceLifecycle::Confirmed =>
        {
            return Err(TriggerError::ReservationFailed("source is not confirmed"))
        }
        Some(_) => {}
    }
    let rowid = ScanStore::source_rowid(source_id).ok_or(
        TriggerError::ReservationFailed("source_id handle did not resolve to a rowid"),
    )?;
    let store = ScanStore::new(&conn);
    // Single-owner gate: refuse to allocate a new run while a non-terminal run
    // exists for this source. See the method doc for why this must run before
    // `begin_run`.
    if store
        .has_in_flight_run(rowid)
        .map_err(|_| TriggerError::ReservationFailed("in-flight check failed"))?
    {
        return Err(TriggerError::AlreadyRunning {
            source_id: source_id.clone(),
        });
    }
    store
        .begin_run(rowid, "pending")
        .map_err(|_| TriggerError::ReservationFailed("begin_run failed"))
}

/// The shared reservation + worker-spawn callable. Used by BOTH:
/// - the HTTP rescan path ([`crate::http::start_rescan`] wraps this with
///   transport job tracking), and
/// - the watcher/periodic reconcile path ([`run_reconcile_loop`]).
///
/// This is the spec task "factor `start_rescan`'s begin_run+spawn into a shared
/// callable" — there is ONE mutation path; HTTP rescan and watcher reconcile
/// cannot diverge.
///
/// Steps (binding constraint — never hold the synchronous request mutex on a
/// reconcile):
/// 1. Acquire the synchronous request mutex only long enough to validate the
///    source is confirmed and call `begin_run` (via [`reserve_run`]). The
///    reserved `(scan_id, token, generation)` is the ownership claim.
/// 2. Spawn a worker thread that opens its OWN `rusqlite::Connection` and runs
///    [`application::scan_reserved_source`]. The mutex is released before the
///    FS work starts.
///
/// Returns `Ok(())` once the worker has been spawned; the worker's own
/// outcome is observed via the existing `GET /api/scan/status` surface (spec
/// Never: "Never push long-lived SSE/streaming notifications for
/// watcher/reconcile").
pub fn trigger_reconcile(source_id: SourceId, state: &Arc<IndexState>) -> Result<(), TriggerError> {
    trigger_reconcile_with_hint_queue(source_id, state, None)
}

/// Same as [`trigger_reconcile`] but also clears the per-source `in_flight`
/// flag on the supplied hint queue when the worker finishes. Used internally by
/// [`run_reconcile_loop`]; the HTTP path passes `None` (it tracks jobs in
/// `rescan_jobs` instead).
pub(crate) fn trigger_reconcile_with_hint_queue(
    source_id: SourceId,
    state: &Arc<IndexState>,
    queue: Option<Arc<HintQueue>>,
) -> Result<(), TriggerError> {
    // Shared reservation with the HTTP rescan path: validates source is
    // confirmed + calls begin_run, holding the synchronous request mutex ONLY
    // for the reservation. The FS work happens on the worker thread with its
    // own connection.
    let (scan_id, fencing_token, generation) = reserve_run(&source_id, state)?;

    let worker_state = Arc::clone(state);
    let worker_source = source_id.clone();
    let worker_queue = queue.clone();
    if let Some(queue) = &queue {
        queue.mark_in_flight(&source_id);
    }
    let spawn_result = thread::Builder::new()
        .name(format!("tessera-reconcile-{}", source_id.0))
        .spawn(move || {
            let result = (|| -> Result<(), ScanError> {
                let conn = Connection::open(&worker_state.db_path).map_err(|_| {
                    // The worker could not open its own connection. The run
                    // row is still `queued` (scan_reserved_source never ran),
                    // so we must fail_run HERE — under the synchronous request
                    // mutex, since this is the only connection we can write
                    // through — to honor the "失败即 fail_run、不留半态"
                    // invariant. Without this the row would sit `queued` until
                    // the next boot recovery.
                    fail_reserved_run_from_main_conn(&worker_state, scan_id, "internal");
                    ScanError::Internal
                })?;
                conn.execute_batch("PRAGMA foreign_keys = ON;")
                    .map_err(|_| ScanError::Internal)?;
                let registry = SourceRegistry::new(&conn);
                application::scan_reserved_source(
                    &registry,
                    &conn,
                    &worker_source,
                    scan_id,
                    fencing_token,
                    generation,
                )
                .map(|_| ())
            })();
            if let Err(e) = result {
                eprintln!(
                    "tessera: reconcile worker for {} failed: {e:?}; previous generation preserved",
                    worker_source
                );
            }
            if let Some(queue) = worker_queue {
                queue.clear_in_flight(&worker_source);
            }
        });
    if let Err(_spawn_error) = spawn_result {
        // The worker thread never started, so scan_reserved_source never ran
        // and the run row is still `queued`. Fail it now under the
        // synchronous request mutex so it does not sit non-terminal until the
        // next boot (spec Design Notes — "失败即 fail_run、不留半态").
        fail_reserved_run_from_main_conn(state, scan_id, "internal");
        // Clear in_flight on the queue if the loop armed it (the worker never
        // will). drop_hint is NOT appropriate here: this is a transient
        // failure (thread spawn), not a permanent source-eligibility change.
        if let Some(queue) = &queue {
            queue.clear_in_flight(&source_id);
        }
        return Err(TriggerError::ReservationFailed("worker spawn failed"));
    }
    // Drop the JoinHandle deliberately: the worker is fire-and-forget (on
    // process exit the OS reaps it; the supervisor does not join per-worker).
    Ok(())
}

/// Best-effort `fail_run` for a reserved run when the worker cannot do it
/// itself (spawn failed, or the worker's own `Connection::open` failed before
/// it could call `scan_reserved_source`). Acquires the synchronous request
/// mutex briefly, opens a [`ScanStore`] over the shared connection, and marks
/// the run `failed` with the supplied error code. Errors are logged and
/// swallowed: this is a cleanup path, and a write failure here just leaves the
/// row for the next boot recovery (which is the same outcome as if this
/// function did not exist).
fn fail_reserved_run_from_main_conn(state: &IndexState, scan_id: i64, error_code: &str) {
    let Ok(conn) = state.conn.lock() else {
        eprintln!(
            "tessera: reconcile: could not acquire main mutex to fail run {scan_id}; leaving for boot recovery"
        );
        return;
    };
    if let Err(e) = ScanStore::new(&conn).fail_run(scan_id, error_code) {
        eprintln!(
            "tessera: reconcile: fail_run({scan_id}, {error_code}) failed: {e:?}; leaving for boot recovery"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::provider_adapter::{CandidateSource, CoverageLevel, DiscoveryBasis};
    use crate::index::migrations;

    fn fresh_state(tmp: &Path) -> Arc<IndexState> {
        let mut conn = Connection::open(tmp.join("tessera-index.db")).expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON;").expect("pragma");
        migrations::apply(&mut conn).expect("migrations");
        Arc::new(IndexState {
            conn: Mutex::new(conn),
            rescan_jobs: Mutex::new(HashMap::new()),
            db_path: tmp.join("tessera-index.db"),
            reconcile_supervisor: Mutex::new(None),
        })
    }

    fn candidate_for(root: &Path) -> CandidateSource {
        CandidateSource {
            provider: "codex".to_string(),
            root_path: root.to_string_lossy().into_owned(),
            basis: DiscoveryBasis::CodexHomeEnv,
            coverage_level: CoverageLevel::Full,
            native_project: None,
        }
    }

    #[test]
    fn hint_queue_records_and_drains_pending_hints() {
        let queue = HintQueue::new();
        let src = SourceId("src_1".to_string());
        queue.record_hint(&src);
        assert!(queue.has_pending_hint(&src));
        assert_eq!(queue.pending_count(), 1);

        // Drain with elapsed debounce: yields the source and clears pending.
        std::thread::sleep(Duration::from_millis(5));
        let due = queue.drain_due(Duration::from_millis(1), false);
        assert_eq!(due, vec![src.clone()]);
        assert!(!queue.has_pending_hint(&src));

        // Drain again: empty.
        let due = queue.drain_due(Duration::from_millis(1), false);
        assert!(due.is_empty());
    }

    #[test]
    fn hint_queue_force_all_drains_every_queued_source() {
        // Patch R: renamed from `..._every_confirmed_source`. The name now
        // matches what `drain_due(force_all=true)` actually does — it only
        // yields sources ALREADY in the queue. It does NOT enumerate the
        // registry, so a "confirmed source" with no queue entry is not
        // reconciled here. The production periodic force-reconcile is
        // `due_for_periodic_tick`, which reads the registry directly; that
        // path is covered by the integration tests in tests/reconcile.rs.
        let queue = HintQueue::new();
        let src_a = SourceId("src_1".to_string());
        // Source A has a pending hint; no other source is in the queue.
        queue.record_hint(&src_a);
        // Force-drain yields only queued sources — here just src_a.
        let due = queue.drain_due(Duration::from_secs(60), true);
        assert_eq!(due, vec![src_a.clone()]);
        assert!(!queue.has_pending_hint(&src_a));
    }

    #[test]
    fn hint_queue_in_flight_blocks_drain_and_is_cleared() {
        let queue = HintQueue::new();
        let src = SourceId("src_1".to_string());
        queue.record_hint(&src);
        queue.mark_in_flight(&src);

        // Drain with elapsed debounce but in-flight: not yielded.
        std::thread::sleep(Duration::from_millis(5));
        let due = queue.drain_due(Duration::from_millis(1), false);
        assert!(due.is_empty());

        // After clear_in_flight, the next drain yields it. (Pending stays true
        // because drain_due bailed out before clearing it.)
        queue.clear_in_flight(&src);
        let due = queue.drain_due(Duration::from_millis(1), false);
        assert_eq!(due, vec![src]);
    }

    #[test]
    fn trigger_reconcile_rejects_unknown_source_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = fresh_state(tmp.path());
        let unknown = SourceId("src_999".to_string());
        match trigger_reconcile(unknown, &state) {
            Err(TriggerError::ReservationFailed(reason)) => {
                assert!(reason.contains("not found") || reason.contains("rowid"));
            }
            other => panic!("expected ReservationFailed, got {other:?}"),
        }
    }

    /// A confirmed source with an empty memory root: trigger_reconcile should
    /// reserve a run (begin_run) and spawn a worker. The worker will scan an
    /// empty dir successfully. We don't assert the worker's outcome here — only
    /// that the reservation path works (no AlreadyRunning, no
    /// ReservationFailed for unknown / not confirmed).
    #[test]
    fn trigger_reconcile_reserves_run_for_confirmed_source() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let memories = tmp.path().join("memories");
        std::fs::create_dir_all(&memories).expect("mkdir");
        std::fs::write(memories.join("MEMORY.md"), "boot\n").expect("write");

        let state = fresh_state(tmp.path());
        let source = {
            let conn = state.conn.lock().expect("lock");
            let registry = SourceRegistry::new(&conn);
            application::confirm_source(&registry, &candidate_for(&memories)).expect("confirm")
        };
        // Drop the connection lock before triggering.
        let outcome = trigger_reconcile(source.source_id.clone(), &state);
        assert!(outcome.is_ok(), "trigger returned {outcome:?}");

        // Wait briefly for the worker to either finish or stage. The empty
        // scan succeeds and leaves an active generation marker.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let active = {
                let conn = state.conn.lock().expect("lock");
                let store = ScanStore::new(&conn);
                let rowid = source.source_id.to_rowid().expect("rowid");
                store.active_generation(rowid).expect("active")
            };
            if active.is_some() || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let rowid = source.source_id.to_rowid().expect("rowid");
        let conn = state.conn.lock().expect("lock");
        let active = ScanStore::new(&conn).active_generation(rowid).expect("active");
        assert!(active.is_some(), "reconcile worker should have committed");
    }

    #[test]
    fn opencode_watches_only_the_source_root() {
        assert!(matches!(
            recursive_mode_for_provider("opencode"),
            RecursiveMode::NonRecursive
        ));
        assert!(matches!(
            recursive_mode_for_provider("codex"),
            RecursiveMode::Recursive
        ));
        assert!(matches!(
            recursive_mode_for_provider("claude_code"),
            RecursiveMode::Recursive
        ));
    }

    #[test]
    fn opencode_only_queues_direct_agents_event() {
        let queue = HintQueue::new();
        let source_id = SourceId("src_1".to_string());
        let root = Path::new("/tmp/opencode-project");
        let mut event = notify::Event::new(notify::EventKind::Any);
        event.paths.push(root.join("nested").join("file.rs"));

        record_event_hint_if_relevant(&queue, &source_id, "opencode", root, &event);
        assert!(!queue.has_pending_hint(&source_id));

        event.paths.clear();
        event.paths.push(root.join("AGENTS.md"));
        record_event_hint_if_relevant(&queue, &source_id, "opencode", root, &event);
        assert!(queue.has_pending_hint(&source_id));
    }
}
