//! Debounced autosave.
//!
//! Port of upstream `NSSProject/AutosaveDebouncer.swift`, with one structural change forced by
//! Windows: a **single worker thread** with a condition variable replaces one cancelled `Task` per
//! schedule. See `docs/ADR/0010-windows-atomic-write.md`.
//!
//! The observable contract is unchanged:
//!
//! * each [`schedule`](AutosaveDebouncer::schedule) replaces any pending save for the same bundle
//!   and restarts the timer, so the write happens `delay` seconds after the **last** change;
//! * [`flush`](AutosaveDebouncer::flush) writes everything pending immediately and synchronously
//!   (call it on quit);
//! * an explicit save goes through [`exclusive`](AutosaveDebouncer::exclusive), which waits for an
//!   in-flight autosave, drops the pending one, then runs — so a superseded autosave can never
//!   land after a newer save.
//!
//! The clock is injectable so tests never sleep for real (upstream's `DebounceClock`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nss_core::{Project, ProjectReference};
use parking_lot::{Condvar, Mutex};

use crate::errors::ProjectError;

/// Writes one project to one bundle.
pub type Writer =
    Arc<dyn Fn(&Project, &ProjectReference) -> Result<(), ProjectError> + Send + Sync>;

/// Called when a debounced write fails. Autosave failures must not kill the worker, or one
/// full disk would silently stop all future saves.
pub type ErrorHandler = Arc<dyn Fn(&ProjectError, &ProjectReference) + Send + Sync>;

/// Time source for the debounce, injectable so tests never wait for a wall clock.
pub trait DebounceClock: Send + Sync + 'static {
    /// Monotonic seconds on this clock.
    fn now(&self) -> f64;

    /// A counter bumped by every [`notify`](DebounceClock::notify). A waiter compares it to notice
    /// that it was interrupted rather than woken by the clock.
    fn epoch(&self) -> u64;

    /// Blocks until `deadline` has passed **or** the epoch has moved away from `epoch`.
    ///
    /// The interrupt is not an optimisation: it is what lets the debouncer shut down and re-plan
    /// promptly. Returning early is always safe — the caller re-checks what is due.
    fn wait_until(&self, deadline: f64, epoch: u64);

    /// Bumps the epoch and wakes every thread blocked in [`wait_until`](DebounceClock::wait_until).
    fn notify(&self);
}

/// The real clock: monotonic, interruptible.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
    epoch: std::sync::atomic::AtomicU64,
    gate: Mutex<()>,
    wake: Condvar,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClock {
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            epoch: std::sync::atomic::AtomicU64::new(0),
            gate: Mutex::new(()),
            wake: Condvar::new(),
        }
    }
}

impl DebounceClock for SystemClock {
    fn now(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    fn wait_until(&self, deadline: f64, epoch: u64) {
        let mut gate = self.gate.lock();
        loop {
            if self.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let remaining = deadline - self.now();
            if remaining <= 0.0 {
                return;
            }
            // Cap each hop so a clock change (or a very long deadline) cannot park us forever.
            let hop = Duration::from_secs_f64(remaining.min(3600.0));
            self.wake.wait_for(&mut gate, hop);
        }
    }

    fn notify(&self) {
        // Take the lock so a wake-up cannot slip in between the check and the wait.
        let _gate = self.gate.lock();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        drop(_gate);
        self.wake.notify_all();
    }
}

/// A clock that only moves when the test says so. Used by this crate's tests and available to the
/// app's integration tests, which is why it is not `#[cfg(test)]`.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Mutex<f64>,
    epoch: std::sync::atomic::AtomicU64,
    waiting: AtomicUsize,
    wake: Condvar,
}

impl ManualClock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward and wakes every sleeper.
    pub fn advance(&self, by: f64) {
        *self.now.lock() += by;
        self.wake.notify_all();
    }

    #[must_use]
    pub fn now_secs(&self) -> f64 {
        *self.now.lock()
    }

    /// How many threads are blocked in `wait_until` right now.
    #[must_use]
    pub fn sleeper_count(&self) -> usize {
        self.waiting.load(Ordering::Acquire)
    }

    /// Yields until `n` sleepers are registered. Bounded, never sleeps.
    pub fn wait_for_sleepers(&self, n: usize) {
        for _ in 0..100_000 {
            if self.sleeper_count() == n {
                return;
            }
            std::thread::yield_now();
        }
    }
}

impl DebounceClock for ManualClock {
    fn now(&self) -> f64 {
        *self.now.lock()
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    fn wait_until(&self, deadline: f64, epoch: u64) {
        self.waiting.fetch_add(1, Ordering::AcqRel);
        let mut now = self.now.lock();
        while *now < deadline && self.epoch.load(Ordering::Acquire) == epoch {
            self.wake.wait(&mut now);
        }
        drop(now);
        self.waiting.fetch_sub(1, Ordering::AcqRel);
    }

    fn notify(&self) {
        // Take the lock so a wake-up cannot be lost before the sleeper registers.
        let _now = self.now.lock();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        drop(_now);
        self.wake.notify_all();
    }
}

struct Pending {
    reference: ProjectReference,
    project: Project,
    deadline: f64,
}

#[derive(Default)]
struct State {
    /// One entry per bundle: a newer `schedule` replaces the old entry, which is exactly how the
    /// timer restarts and how a superseded autosave can never write (its entry is gone).
    pending: HashMap<PathBuf, Pending>,
}

struct Shared {
    delay: f64,
    clock: Arc<dyn DebounceClock>,
    writer: Writer,
    on_error: Option<ErrorHandler>,
    state: Mutex<State>,
    wake: Condvar,
    write_lock: Mutex<()>,
    shutdown: AtomicBool,
}

impl Shared {
    fn write(&self, pending: &Pending) {
        if let Err(e) = (self.writer)(&pending.project, &pending.reference) {
            if let Some(handler) = &self.on_error {
                handler(&e, &pending.reference);
            }
        }
    }

    /// Removes and returns every pending entry whose deadline has passed.
    fn take_due(&self, now: f64) -> Vec<Pending> {
        let mut state = self.state.lock();
        let due: Vec<PathBuf> = state
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= now)
            .map(|(k, _)| k.clone())
            .collect();
        due.into_iter()
            .filter_map(|k| state.pending.remove(&k))
            .collect()
    }

    fn run(self: Arc<Self>) {
        loop {
            let (deadline, epoch) = {
                let mut state = self.state.lock();
                loop {
                    if self.shutdown.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some(d) = state.pending.values().map(|p| p.deadline).reduce(f64::min) {
                        // Read the epoch under the same lock a `schedule` takes, so a save that
                        // arrives right now cannot be missed by the wait below.
                        break (d, self.clock.epoch());
                    }
                    self.wake.wait(&mut state);
                }
            };
            // Sleep outside the state lock: `schedule` must stay responsive while we wait.
            self.clock.wait_until(deadline, epoch);
            if self.shutdown.load(Ordering::Acquire) {
                return;
            }
            // A newer schedule may have pushed the deadline out while we slept. Hold `write_lock`
            // across `take_due` + `write` (matching `exclusive()`/`flush()` lock order) so an
            // explicit save can never run between popping an old pending entry and writing it.
            loop {
                let _writing = self.write_lock.lock();
                if self.shutdown.load(Ordering::Acquire) {
                    return;
                }
                let due = self.take_due(self.clock.now());
                if due.is_empty() {
                    break;
                }
                for pending in due {
                    self.write(&pending);
                }
            }
        }
    }
}

/// Debounced autosave, one timer per bundle.
pub struct AutosaveDebouncer {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for AutosaveDebouncer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutosaveDebouncer")
            .field("delay", &self.shared.delay)
            .finish()
    }
}

impl Drop for AutosaveDebouncer {
    fn drop(&mut self) {
        // Hold `state` while publishing `shutdown` so the worker cannot miss the wakeup between
        // reading `shutdown == false` and parking in `self.wake.wait(&mut state)`.
        {
            let _state = self.shared.state.lock();
            self.shared.shutdown.store(true, Ordering::Release);
        }
        self.shared.wake.notify_all();
        self.shared.clock.notify();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl AutosaveDebouncer {
    /// `delay` seconds after the last change, `writer` persists the bundle.
    #[must_use]
    pub fn new(
        delay: f64,
        clock: Arc<dyn DebounceClock>,
        on_error: Option<ErrorHandler>,
        writer: Writer,
    ) -> Self {
        let shared = Arc::new(Shared {
            delay: delay.max(0.0),
            clock,
            writer,
            on_error,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            write_lock: Mutex::new(()),
            shutdown: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("nss-autosave".to_owned())
            .spawn(move || worker_shared.run())
            .ok();
        Self { shared, worker }
    }

    /// Standard 1 s delay on the real clock.
    #[must_use]
    pub fn with_writer(writer: Writer) -> Self {
        Self::new(1.0, Arc::new(SystemClock::new()), None, writer)
    }

    /// Schedules (or re-schedules) the save for `reference`.
    pub fn schedule(&self, project: Project, reference: &ProjectReference) {
        let key = path_key(&reference.url);
        let deadline = self.shared.clock.now() + self.shared.delay;
        self.shared.state.lock().pending.insert(
            key,
            Pending {
                reference: reference.clone(),
                project,
                deadline,
            },
        );
        // Wake the worker so an earlier deadline (or an empty queue it is blocked on) is seen.
        self.shared.wake.notify_all();
        self.shared.clock.notify();
    }

    /// Drops the pending save for `reference`, for example because an explicit save superseded it.
    pub fn cancel(&self, reference: &ProjectReference) {
        self.shared
            .state
            .lock()
            .pending
            .remove(&path_key(&reference.url));
        self.shared.wake.notify_all();
        self.shared.clock.notify();
    }

    /// Runs an explicit save exclusively: waits for any in-flight autosave write to finish, drops
    /// the pending autosave for `reference`, then runs `body`. Whatever `body` writes is therefore
    /// the last write for that bundle.
    pub fn exclusive<T>(&self, reference: &ProjectReference, body: impl FnOnce() -> T) -> T {
        let _writing = self.shared.write_lock.lock();
        self.shared
            .state
            .lock()
            .pending
            .remove(&path_key(&reference.url));
        let out = body();
        drop(_writing);
        self.shared.wake.notify_all();
        out
    }

    #[must_use]
    pub fn has_pending(&self, reference: &ProjectReference) -> bool {
        self.shared
            .state
            .lock()
            .pending
            .contains_key(&path_key(&reference.url))
    }

    /// Writes every pending project now, synchronously. Returns the number written.
    pub fn flush(&self) -> usize {
        let taken = {
            let _writing = self.shared.write_lock.lock();
            let mut state = self.shared.state.lock();
            let all: Vec<Pending> = state.pending.drain().map(|(_, p)| p).collect();
            drop(state);
            for pending in &all {
                self.shared.write(pending);
            }
            all.len()
        };
        self.shared.wake.notify_all();
        self.shared.clock.notify();
        taken
    }
}

/// Identity of a bundle for the debounce map.
///
/// Windows paths are case-insensitive and accept both `/` and `\`, so two spellings of the same
/// bundle must not be two timers. `fs::canonicalize` would also fold `.`/`..`, but it fails for a
/// bundle that does not exist yet and returns verbatim (`\\?\`) paths on Windows, which would then
/// not match a plain path; a normalized, lower-cased spelling is predictable and enough.
pub(crate) fn path_key(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        PathBuf::from(text.replace('\\', "/").to_lowercase())
    } else {
        PathBuf::from(text.into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{PixelSize, RecordingAssets};
    use std::sync::Mutex as StdMutex;

    fn project(name: &str) -> Project {
        Project::new(
            name,
            RecordingAssets::new(PixelSize::new(100, 100), 60.0, 1.0),
        )
    }

    fn reference(leaf: &str) -> ProjectReference {
        ProjectReference::new(PathBuf::from(format!("/tmp/{leaf}.nssproj")))
    }

    /// Collects writes made by a debouncer.
    #[derive(Default)]
    struct WriteLog {
        entries: StdMutex<Vec<(String, PathBuf)>>,
    }

    impl WriteLog {
        fn names(&self) -> Vec<String> {
            self.entries
                .lock()
                .unwrap()
                .iter()
                .map(|(n, _)| n.clone())
                .collect()
        }

        fn len(&self) -> usize {
            self.entries.lock().unwrap().len()
        }
    }

    fn debouncer(delay: f64, clock: Arc<ManualClock>, log: Arc<WriteLog>) -> AutosaveDebouncer {
        let writer: Writer = Arc::new(move |p: &Project, r: &ProjectReference| {
            log.entries
                .lock()
                .unwrap()
                .push((p.name.clone(), r.url.clone()));
            Ok(())
        });
        AutosaveDebouncer::new(delay, clock, None, writer)
    }

    fn settle(check: impl Fn() -> bool) {
        for _ in 0..20_000 {
            if check() {
                return;
            }
            std::thread::yield_now();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_change_restarts_the_timer_and_only_the_last_save_is_written() {
        // Mirrors upstream `debounceWritesOnceAfterTheLastChange`.
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let d = debouncer(1.0, Arc::clone(&clock), Arc::clone(&log));
        let r = reference("a");

        d.schedule(project("v1"), &r);
        clock.wait_for_sleepers(1);
        clock.advance(0.6);
        d.schedule(project("v2"), &r); // restarts the timer
        clock.wait_for_sleepers(1);
        clock.advance(0.6); // 1.2 s since v1, only 0.6 s since v2
        std::thread::sleep(Duration::from_millis(30));
        assert!(log.names().is_empty());
        assert!(d.has_pending(&r));

        clock.advance(0.5);
        settle(|| log.len() == 1);
        assert_eq!(log.names(), vec!["v2".to_owned()]);
        assert!(!d.has_pending(&r));
    }

    #[test]
    fn bundles_are_independent() {
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let d = debouncer(1.0, Arc::clone(&clock), Arc::clone(&log));
        d.schedule(project("a"), &reference("a"));
        d.schedule(project("b"), &reference("b"));
        clock.wait_for_sleepers(1);
        clock.advance(1.0);
        settle(|| log.len() == 2);
        let mut names = log.names();
        names.sort();
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn flush_writes_pending_now_and_the_timer_does_not_write_again() {
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let d = debouncer(1.0, Arc::clone(&clock), Arc::clone(&log));
        let r = reference("a");
        d.schedule(project("v1"), &r);
        clock.wait_for_sleepers(1);
        assert_eq!(d.flush(), 1);
        assert_eq!(log.names(), vec!["v1".to_owned()]);
        clock.advance(5.0);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(log.names(), vec!["v1".to_owned()]);
        assert_eq!(d.flush(), 0);
    }

    #[test]
    fn cancel_drops_pending_and_write_errors_are_reported() {
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let d = debouncer(1.0, Arc::clone(&clock), Arc::clone(&log));
        let r = reference("a");
        d.schedule(project("v1"), &r);
        d.cancel(&r);
        assert_eq!(d.flush(), 0);
        assert!(log.names().is_empty());

        let errors = Arc::new(StdMutex::new(0usize));
        let counter = Arc::clone(&errors);
        let failing: Writer = Arc::new(|_: &Project, _: &ProjectReference| {
            Err(ProjectError::Corrupt("boom".to_owned()))
        });
        let d = AutosaveDebouncer::new(
            1.0,
            Arc::clone(&clock) as Arc<dyn DebounceClock>,
            Some(Arc::new(move |_, _| *counter.lock().unwrap() += 1)),
            failing,
        );
        d.schedule(project("v1"), &r);
        assert_eq!(d.flush(), 1);
        assert_eq!(*errors.lock().unwrap(), 1);
    }

    #[test]
    fn an_explicit_save_beats_a_pending_autosave() {
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let d = debouncer(1.0, Arc::clone(&clock), Arc::clone(&log));
        let r = reference("a");
        d.schedule(project("P1"), &r);
        clock.wait_for_sleepers(1);
        d.exclusive(&r, || {
            log.entries
                .lock()
                .unwrap()
                .push(("P2".to_owned(), r.url.clone()));
        });
        clock.advance(1.0);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(log.names(), vec!["P2".to_owned()]);
    }

    #[test]
    fn an_explicit_save_during_an_in_flight_write_lands_last() {
        // Mirrors upstream `explicitSaveDuringInFlightAutosaveWriteLandsLast`: the writer blocks
        // until the test opens the gate, and the explicit save must queue behind it.
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let entered = Arc::new(AtomicBool::new(false));

        let writer: Writer = {
            let log = Arc::clone(&log);
            let gate = Arc::clone(&gate);
            let entered = Arc::clone(&entered);
            Arc::new(move |p: &Project, r: &ProjectReference| {
                entered.store(true, Ordering::Release);
                let mut open = gate.0.lock();
                while !*open {
                    gate.1.wait(&mut open);
                }
                log.entries
                    .lock()
                    .unwrap()
                    .push((p.name.clone(), r.url.clone()));
                Ok(())
            })
        };
        let d = AutosaveDebouncer::new(
            1.0,
            Arc::clone(&clock) as Arc<dyn DebounceClock>,
            None,
            writer,
        );
        let r = reference("a");

        d.schedule(project("P1"), &r);
        clock.wait_for_sleepers(1);
        clock.advance(1.0);
        settle(|| entered.load(Ordering::Acquire));

        // Run the exclusive save on this thread while the writer is parked, then open the gate
        // from a helper thread once the exclusive call has had time to block.
        let opener = {
            let gate = Arc::clone(&gate);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                *gate.0.lock() = true;
                gate.1.notify_all();
            })
        };
        d.exclusive(&r, || {
            log.entries
                .lock()
                .unwrap()
                .push(("P2".to_owned(), r.url.clone()));
        });
        let _ = opener.join();

        assert_eq!(log.names(), vec!["P1".to_owned(), "P2".to_owned()]);
        assert!(!d.has_pending(&r));
    }

    #[test]
    fn dropping_the_debouncer_does_not_hang_with_a_pending_save() {
        let clock = Arc::new(ManualClock::new());
        let log = Arc::new(WriteLog::default());
        let d = debouncer(1.0, Arc::clone(&clock), Arc::clone(&log));
        d.schedule(project("v1"), &reference("a"));
        clock.wait_for_sleepers(1);
        drop(d); // must not deadlock on the sleeping worker
        assert!(log.names().is_empty());
    }

    #[test]
    fn path_keys_fold_case_on_windows() {
        let a = path_key(Path::new("C:/Users/x/Videos/Demo.nssproj"));
        let b = path_key(Path::new("c:/users/X/Videos/demo.nssproj"));
        if cfg!(windows) {
            assert_eq!(a, b);
        } else {
            assert_ne!(a, b);
        }
    }
}
