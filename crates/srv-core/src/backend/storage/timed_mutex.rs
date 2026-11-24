// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A mutex that says when it was waited on, or held, for a long time.
//!
//! The object store keeps one SQLite connection behind one lock, and every store call in the process takes it, many
//! of them from async handlers. When something holds it for long (a commit waiting on a busy disk), every caller
//! queues; a caller on a runtime worker takes the worker with it. Nothing said so: this does. A wait or a hold of
//! [`SLOW`] or more logs one line, rate-limited, with where the lock was taken (a source location, never a key, a
//! path or content), how long the caller waited and held, and whether it was a runtime worker (`thread_roles`).
//! The behaviour of the lock is not changed.

use std::ops::{Deref, DerefMut};
use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult};
use std::time::{Duration, Instant};

/// A wait or a hold this long is logged.
pub const SLOW: Duration = Duration::from_millis(250);
/// At most one line this often; the lines left out are counted in the next one.
const LOG_EVERY: Duration = Duration::from_millis(500);

/// A [`Mutex`] that logs a slow wait or hold. `store` names which store it is, in the line.
pub struct TimedMutex<T> {
    inner: Mutex<T>,
    store: &'static str,
}

/// The lock, held: derefs to the value, and logs on drop when this was slow.
pub struct TimedGuard<'a, T> {
    guard: MutexGuard<'a, T>,
    store: &'static str,
    site: &'static Location<'static>,
    waited: Duration,
    got: Instant,
}

impl<T> TimedMutex<T> {
    pub fn new(value: T, store: &'static str) -> Self {
        Self {
            inner: Mutex::new(value),
            store,
        }
    }

    /// As [`Mutex::lock`]; `#[track_caller]` carries the call site into the line.
    #[track_caller]
    pub fn lock(&self) -> LockResult<TimedGuard<'_, T>> {
        let site = Location::caller();
        let asked = Instant::now();
        match self.inner.lock() {
            Ok(guard) => Ok(self.wrap(guard, site, asked)),
            Err(poisoned) => Err(PoisonError::new(self.wrap(
                poisoned.into_inner(),
                site,
                asked,
            ))),
        }
    }

    /// As [`Mutex::try_lock`]: no wait, so no wait to report; a hold is still timed.
    #[track_caller]
    pub fn try_lock(&self) -> TryLockResult<TimedGuard<'_, T>> {
        let site = Location::caller();
        let asked = Instant::now();
        match self.inner.try_lock() {
            Ok(guard) => Ok(self.wrap(guard, site, asked)),
            Err(TryLockError::Poisoned(p)) => Err(TryLockError::Poisoned(PoisonError::new(
                self.wrap(p.into_inner(), site, asked),
            ))),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }

    fn wrap<'a>(
        &self,
        guard: MutexGuard<'a, T>,
        site: &'static Location<'static>,
        asked: Instant,
    ) -> TimedGuard<'a, T> {
        let got = Instant::now();
        TimedGuard {
            guard,
            store: self.store,
            site,
            waited: got.duration_since(asked),
            got,
        }
    }
}

impl<T> TimedGuard<'_, T> {
    /// How long taking the lock waited.
    pub fn waited(&self) -> Duration {
        self.waited
    }
}

impl<T> Deref for TimedGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for TimedGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for TimedGuard<'_, T> {
    fn drop(&mut self) {
        let held = self.got.elapsed();
        if self.waited < SLOW && held < SLOW {
            return;
        }
        if let Some(suppressed) = SPACER.admit(epoch_ms()) {
            tracing::warn!(
                store = self.store,
                site = %self.site,
                waited_ms = self.waited.as_millis() as u64,
                held_ms = held.as_millis() as u64,
                on_worker = crate::backend::thread_roles::on_worker(),
                suppressed,
                "store lock: a wait or a hold was slow"
            );
        }
    }
}

/// Spaces the lines: one per [`LOG_EVERY`], and the count of the ones left out goes with the next.
struct Spacer {
    /// When the last line was admitted, in [`epoch_ms`] terms; zero means never.
    last_ms: AtomicU64,
    left_out: AtomicU64,
}

impl Spacer {
    const fn new() -> Self {
        Self {
            last_ms: AtomicU64::new(0),
            left_out: AtomicU64::new(0),
        }
    }

    /// Whether a line may be logged at `now_ms`: the number of lines left out since the last one, or nothing.
    fn admit(&self, now_ms: u64) -> Option<u64> {
        let last = self.last_ms.load(Ordering::Acquire);
        let due = last == 0 || now_ms.saturating_sub(last) >= LOG_EVERY.as_millis() as u64;
        if due
            && self
                .last_ms
                .compare_exchange(last, now_ms, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return Some(self.left_out.swap(0, Ordering::AcqRel));
        }
        self.left_out.fetch_add(1, Ordering::AcqRel);
        None
    }
}

static SPACER: Spacer = Spacer::new();

/// Milliseconds since the first call (kept above zero so that zero means "never logged").
fn epoch_ms() -> u64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn it_locks_and_derefs_like_a_mutex() {
        let m = TimedMutex::new(5, "test");
        *m.lock().unwrap() += 1;
        assert_eq!(*m.lock().unwrap(), 6);
        let held = m.lock().unwrap();
        assert!(matches!(m.try_lock(), Err(TryLockError::WouldBlock)));
        drop(held);
        assert!(m.try_lock().is_ok());
    }

    #[test]
    fn a_poisoned_lock_still_gives_its_value() {
        let m = Arc::new(TimedMutex::new(1, "test"));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        let value = *m.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(value, 1);
    }

    #[test]
    fn a_caller_that_waits_for_a_held_lock_measures_the_wait() {
        let m = Arc::new(TimedMutex::new((), "test"));
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let holder = {
            let m = m.clone();
            std::thread::spawn(move || {
                let _g = m.lock().unwrap();
                held_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(400));
            })
        };
        held_rx.recv().unwrap();
        let guard = m.lock().unwrap();
        assert!(guard.waited() >= SLOW, "waited only {:?}", guard.waited());
        drop(guard);
        holder.join().unwrap();
    }

    #[test]
    fn lines_are_spaced_and_the_ones_left_out_are_counted() {
        let spacer = Spacer::new();
        assert_eq!(spacer.admit(1_000), Some(0));
        assert_eq!(spacer.admit(1_100), None);
        assert_eq!(spacer.admit(1_200), None);
        assert_eq!(spacer.admit(1_600), Some(2));
        assert_eq!(spacer.admit(1_700), None);
        assert_eq!(spacer.admit(2_100), Some(1));
    }
}
