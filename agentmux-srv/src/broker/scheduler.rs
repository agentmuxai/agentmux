// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Generic, single-flight-guarded, proactively-scheduled credential refresh.
//!
//! One `RefreshScheduler` instance can manage any number of independently
//! keyed credentials. It holds no credential state itself — callers supply
//! `is_fresh`/`refresh` closures that read/write through to the real backing
//! store (e.g. `Store::muxbus_load`/`muxbus_save`) — the scheduler only owns
//! refresh *coordination*.
//!
//! This closes two related gaps:
//! 1. **Concurrent-refresh races.** Without a lock, two callers that both
//!    observe a stale credential at the same time both attempt to refresh it
//!    independently. For providers that rotate refresh tokens on use, this
//!    can revoke the entire token family. Claude Code itself has two open
//!    production issues from exactly this failure mode (concurrent CLI
//!    sessions racing to refresh a shared credentials file with no lock).
//!    `ensure_fresh` collapses concurrent callers for the same id onto one
//!    in-flight refresh.
//! 2. **Reactive-only refresh.** `run_sweep_loop` proactively walks every
//!    registered credential on a timer instead of only refreshing when
//!    something happens to ask for one.
//!
//! Coordination state (fresh / refreshing / failed / needs-reauth) is owned
//! by the pure reducer in `super::state` — this module is a thin async
//! orchestrator: dispatch a command, execute whatever the returned events
//! say to do (call `is_fresh()`/`refresh()`), dispatch the result back in.
//! Same split as every other reducer in this codebase (e.g. `agentmux-cef`'s
//! host reducer emitting `HostEvent::Effect` for `AppState` to execute).
//!
//! Preserve-on-failure (never overwrite a valid stored credential with a
//! failed/partial refresh result) is NOT enforced here — it's the
//! responsibility of each registered `refresh` closure, since only the
//! caller knows its own store's write semantics. See `broker::mod` doc.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, Notify};

use super::state::{self, Command, CredentialState, Event, RefreshErrorKind};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct RegisteredClosures {
    is_fresh: Box<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>,
    refresh: Box<dyn Fn() -> BoxFuture<'static, Result<(), RefreshErrorKind>> + Send + Sync>,
}

// Bundled behind ONE mutex (not three) so `states`/`generations`/
// `next_generation` are always read and written atomically — the reducer
// needs all three together to tell a stale, superseded-registration
// completion apart from a current one (see `state::update`'s doc comment).
#[derive(Default)]
struct ReducerTables {
    states: HashMap<String, CredentialState>,
    generations: HashMap<String, u64>,
    next_generation: u64,
}

pub struct RefreshScheduler {
    closures: AsyncMutex<HashMap<String, Arc<RegisteredClosures>>>,
    // Reducer-owned coordination state. std::sync::Mutex, not AsyncMutex —
    // the reducer never awaits, every hold is sub-millisecond, matching the
    // discipline every other reducer in this codebase follows.
    tables: Mutex<ReducerTables>,
    // Per-id wake signal for concurrent `ensure_fresh` callers waiting on an
    // in-flight check/refresh (the reducer decides single-flight; this just
    // wakes waiters once that decision resolves).
    notifiers: Mutex<HashMap<String, Arc<Notify>>>,
}

impl Default for RefreshScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshScheduler {
    pub fn new() -> Self {
        Self {
            closures: AsyncMutex::new(HashMap::new()),
            tables: Mutex::new(ReducerTables::default()),
            notifiers: Mutex::new(HashMap::new()),
        }
    }

    /// Register (or replace) a credential for proactive management.
    /// `is_fresh` must be side-effect-free (may be called on every
    /// `ensure_fresh`/sweep tick) — async so a caller whose freshness check
    /// needs a blocking read (e.g. an OS keychain call, which can hang on a
    /// slow/unresponsive Secret Service D-Bus daemon on headless Linux) can
    /// route it through `tokio::task::spawn_blocking` instead of stalling
    /// the tokio worker thread this scheduler's own async tasks run on
    /// (reagent P1 on #2260). `refresh` performs the actual refresh and is
    /// responsible for persisting the result itself (and for NOT persisting
    /// anything on failure) — and for classifying a failure as
    /// `RefreshErrorKind::Transient` (worth retrying) vs
    /// `::PermanentAuthFailure` (the credential itself is rejected; only a
    /// fresh login can fix it — see that type's own doc comment).
    pub async fn register(
        &self,
        credential_id: impl Into<String>,
        is_fresh: impl Fn() -> BoxFuture<'static, bool> + Send + Sync + 'static,
        refresh: impl Fn() -> BoxFuture<'static, Result<(), RefreshErrorKind>> + Send + Sync + 'static,
    ) {
        let id: String = credential_id.into();
        {
            let mut closures = self.closures.lock().await;
            closures.insert(
                id.clone(),
                Arc::new(RegisteredClosures {
                    is_fresh: Box::new(is_fresh),
                    refresh: Box::new(refresh),
                }),
            );
        }
        {
            let mut tables = self.tables.lock().unwrap();
            let ReducerTables { states, generations, next_generation } = &mut *tables;
            state::update(states, generations, next_generation, Command::Register { id: id.clone() });
        }
        self.notifiers
            .lock()
            .unwrap()
            .entry(id)
            .or_insert_with(|| Arc::new(Notify::new()));
    }

    pub async fn unregister(&self, credential_id: &str) {
        self.closures.lock().await.remove(credential_id);
        let events = {
            let mut tables = self.tables.lock().unwrap();
            let ReducerTables { states, generations, next_generation } = &mut *tables;
            state::update(
                states,
                generations,
                next_generation,
                Command::Unregister { id: credential_id.to_string() },
            )
        };
        self.trace(&events);
        // A caller elsewhere may be parked on notified().await after seeing
        // AlreadyInFlight for this id; nothing else will ever wake it once
        // the entry is gone (reagent re-review on #2275).
        self.wake(credential_id);
    }

    /// The credential's current coordination state, if registered —
    /// diagnostics/future-UI hook (e.g. surfacing `NeedsReauth` distinctly
    /// from a routine in-progress refresh). Not used by `ensure_fresh`
    /// itself, which always dispatches through the reducer regardless.
    pub fn state(&self, credential_id: &str) -> Option<CredentialState> {
        self.tables.lock().unwrap().states.get(credential_id).cloned()
    }

    /// Ensure `credential_id` is fresh, single-flight-guarded. If another
    /// caller is already mid-refresh for this id, this waits for that
    /// attempt to finish, then re-checks freshness rather than starting a
    /// second refresh. Returns `Err` if the id was never registered.
    pub async fn ensure_fresh(&self, credential_id: &str) -> Result<(), String> {
        let id = credential_id.to_string();
        loop {
            // Create the Notified future *before* dispatching CheckRequested
            // (and thus before releasing the states lock that gates whether
            // we're about to become the AlreadyInFlight waiter). Tokio
            // guarantees a Notified future observes any notify_waiters()
            // issued after it is created, even before its first poll — so
            // creating it here closes the lost-wakeup window where the
            // in-flight refresh finishes and calls wake() between our
            // CheckRequested dispatch and a `.notified()` call made only
            // after we see AlreadyInFlight (reagent P1 on #2275: notify_
            // waiters() retains no permit for a waiter not yet registered).
            let notify = self.notifier_for(&id);
            let notified = notify.notified();

            let events = {
                let mut tables = self.tables.lock().unwrap();
                let ReducerTables { states, generations, next_generation } = &mut *tables;
                state::update(states, generations, next_generation, Command::CheckRequested { id: id.clone() })
            };
            match events.as_slice() {
                [Event::Unregistered { .. }] => {
                    return Err(format!("no credential registered as '{id}'"));
                }
                [Event::AlreadyInFlight { .. }] => {
                    notified.await;
                    continue;
                }
                [Event::RunFreshnessCheck { generation, .. }] => {
                    let generation = *generation;
                    let Some(closures) = self.closures.lock().await.get(&id).cloned() else {
                        // Unregistered concurrently between the dispatch
                        // above and this lookup. Wake any other caller
                        // parked on AlreadyInFlight for this id — this path
                        // owns the only Refreshing state that existed for
                        // it, so nothing else will ever notify them
                        // (reagent re-review on #2275).
                        self.wake(&id);
                        return Err(format!("no credential registered as '{id}'"));
                    };
                    let is_fresh = (closures.is_fresh)().await;
                    let events = {
                        let mut tables = self.tables.lock().unwrap();
                        let ReducerTables { states, generations, next_generation } = &mut *tables;
                        state::update(
                            states,
                            generations,
                            next_generation,
                            Command::FreshnessChecked { id: id.clone(), is_fresh, generation },
                        )
                    };
                    self.trace(&events);
                    if is_fresh {
                        self.wake(&id);
                        return Ok(());
                    }
                    // events is normally [RunRefresh{..}], but can be empty
                    // if this completion lost a race with a concurrent
                    // register()/unregister() — the reducer discards a
                    // stale-epoch result rather than let it clobber newer
                    // state (see state::Event::RunFreshnessCheck's doc
                    // comment). Either way THIS caller still proceeds with
                    // its own refresh using its own already-captured
                    // closures; its return value reflects what it actually
                    // observed regardless of whether the broker's shared
                    // state accepted the transition.
                    let result = (closures.refresh)().await;
                    let (msg, cmd) = match result {
                        Ok(()) => (None, Command::RefreshSucceeded { id: id.clone(), generation }),
                        Err(error) => {
                            let msg = match &error {
                                RefreshErrorKind::Transient(m) | RefreshErrorKind::PermanentAuthFailure(m) => {
                                    m.clone()
                                }
                            };
                            (Some(msg), Command::RefreshFailed { id: id.clone(), error, generation })
                        }
                    };
                    let events = {
                        let mut tables = self.tables.lock().unwrap();
                        let ReducerTables { states, generations, next_generation } = &mut *tables;
                        state::update(states, generations, next_generation, cmd)
                    };
                    self.trace(&events);
                    self.wake(&id);
                    return match msg {
                        None => Ok(()),
                        Some(m) => Err(m),
                    };
                }
                other => unreachable!("unexpected reducer events for CheckRequested: {other:?}"),
            }
        }
    }

    /// Background sweep — checks every registered credential every
    /// `interval` and proactively refreshes anything stale. Skips entries
    /// currently `NeedsReauth`: retrying a credential already known to
    /// require a fresh human login is pointless and just hammers the
    /// backing server; `ensure_fresh` called on-demand still re-checks
    /// those (see `state`'s own doc comment). Spawn once via `tokio::spawn`;
    /// runs until the process exits.
    pub async fn run_sweep_loop(self: Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let ids: Vec<String> = {
                let closures = self.closures.lock().await;
                closures.keys().cloned().collect()
            };
            for id in ids {
                let needs_reauth = matches!(
                    self.tables.lock().unwrap().states.get(&id),
                    Some(CredentialState::NeedsReauth { .. })
                );
                if needs_reauth {
                    continue;
                }
                if let Err(e) = self.ensure_fresh(&id).await {
                    tracing::warn!(credential_id = %id, error = %e, "broker: proactive refresh failed");
                }
            }
        }
    }

    fn notifier_for(&self, id: &str) -> Arc<Notify> {
        self.notifiers
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    fn wake(&self, id: &str) {
        if let Some(n) = self.notifiers.lock().unwrap().get(id) {
            n.notify_waiters();
        }
    }

    /// Map diagnostics-only reducer events to `tracing` — `auth.broker.*`
    /// message prefixes land in `muxlog auth`'s existing filter regex
    /// (`\bauth\.\w+|...`) with zero changes needed there. Control-flow
    /// events (`RunFreshnessCheck`/`RunRefresh`/`AlreadyInFlight`/
    /// `Unregistered`) are acted on directly by callers and not
    /// double-logged here — every `ensure_fresh` call would otherwise emit
    /// noise even when nothing changed.
    fn trace(&self, events: &[Event]) {
        for event in events {
            match event {
                Event::BecameFresh { id } => {
                    tracing::info!(target: "identity", credential_id = %id, "auth.broker.fresh: credential is fresh");
                }
                Event::BecameFailed {
                    id,
                    consecutive_failures,
                    error,
                } => {
                    tracing::warn!(
                        target: "identity",
                        credential_id = %id,
                        consecutive_failures,
                        error = %error,
                        "auth.broker.failed: refresh attempt failed, will retry"
                    );
                }
                Event::BecameNeedsReauth { id, reason } => {
                    tracing::warn!(
                        target: "identity",
                        credential_id = %id,
                        reason = %reason,
                        "auth.broker.needs_reauth: credential needs a fresh login, no longer auto-retrying"
                    );
                }
                Event::RunFreshnessCheck { .. }
                | Event::RunRefresh { .. }
                | Event::AlreadyInFlight { .. }
                | Event::Unregistered { .. } => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn concurrent_callers_collapse_onto_one_refresh() {
        let scheduler = Arc::new(RefreshScheduler::new());
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let fresh_after_refresh = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let calls = refresh_calls.clone();
        let fresh_flag = fresh_after_refresh.clone();
        let fresh_flag_check = fresh_after_refresh.clone();
        scheduler
            .register(
                "test:cred",
                move || {
                    let fresh = fresh_flag_check.load(Ordering::SeqCst);
                    Box::pin(async move { fresh })
                },
                move || {
                    let calls = calls.clone();
                    let fresh_flag = fresh_flag.clone();
                    Box::pin(async move {
                        // Simulate real refresh latency so concurrent callers
                        // actually overlap instead of serializing trivially.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        calls.fetch_add(1, Ordering::SeqCst);
                        fresh_flag.store(true, Ordering::SeqCst);
                        Ok(())
                    })
                },
            )
            .await;

        let mut handles = Vec::new();
        for _ in 0..10 {
            let scheduler = scheduler.clone();
            handles.push(tokio::spawn(async move {
                scheduler.ensure_fresh("test:cred").await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }

        assert_eq!(
            refresh_calls.load(Ordering::SeqCst),
            1,
            "10 concurrent callers for the same credential must collapse onto exactly one refresh"
        );
        assert_eq!(scheduler.state("test:cred"), Some(CredentialState::Fresh));
    }

    #[tokio::test]
    async fn already_fresh_never_calls_refresh() {
        let scheduler = RefreshScheduler::new();
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let calls = refresh_calls.clone();
        scheduler
            .register(
                "test:cred",
                || Box::pin(async { true }),
                move || {
                    let calls = calls.clone();
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                },
            )
            .await;

        scheduler.ensure_fresh("test:cred").await.unwrap();
        scheduler.ensure_fresh("test:cred").await.unwrap();
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_failed_refresh_does_not_mark_the_credential_fresh() {
        let scheduler = RefreshScheduler::new();
        scheduler
            .register(
                "test:cred",
                || Box::pin(async { false }), // never becomes fresh in this test — refresh always fails
                || {
                    Box::pin(async {
                        Err(RefreshErrorKind::Transient(
                            "simulated refresh failure".to_string(),
                        ))
                    })
                },
            )
            .await;

        let result = scheduler.ensure_fresh("test:cred").await;
        assert!(result.is_err());
        // A second call must attempt the refresh again (not treat the
        // failed attempt as having succeeded).
        let result2 = scheduler.ensure_fresh("test:cred").await;
        assert!(result2.is_err());
        assert!(matches!(
            scheduler.state("test:cred"),
            Some(CredentialState::Failed { consecutive_failures: 2, .. })
        ));
    }

    #[tokio::test]
    async fn unregistered_credential_errors_without_panicking() {
        let scheduler = RefreshScheduler::new();
        let result = scheduler.ensure_fresh("nope").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn permanent_auth_failure_stops_the_sweep_loop_retrying_but_ensure_fresh_still_rechecks() {
        let scheduler = Arc::new(RefreshScheduler::new());
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let calls = refresh_calls.clone();
        scheduler
            .register(
                "test:cred",
                || Box::pin(async { false }),
                move || {
                    let calls = calls.clone();
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Err(RefreshErrorKind::PermanentAuthFailure("invalid_grant".into()))
                    })
                },
            )
            .await;

        assert!(scheduler.ensure_fresh("test:cred").await.is_err());
        assert!(matches!(
            scheduler.state("test:cred"),
            Some(CredentialState::NeedsReauth { .. })
        ));

        // An on-demand ensure_fresh call still re-attempts (a human may
        // have logged in again out-of-band) — this is call #2.
        assert!(scheduler.ensure_fresh("test:cred").await.is_err());
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn re_registering_a_credential_mid_refresh_does_not_orphan_a_waiting_second_caller() {
        // Pre-refactor latent race: register() on an already-registered id
        // replaced the Entry (including its per-id lock), so a SECOND
        // caller already blocked waiting on the OLD entry's lock waited
        // forever — the replacement entry's lock was a different object
        // nothing would ever signal for that waiter. Here, single-flight is
        // reducer-state-driven and waiting goes through a per-id Notify
        // that `register()` never replaces (only ensures exists), so a
        // second caller that observed AlreadyInFlight is woken regardless
        // of a concurrent re-register.
        let scheduler = Arc::new(RefreshScheduler::new());
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_wait = gate.clone();
        scheduler
            .register(
                "test:cred",
                || Box::pin(async { false }),
                move || {
                    let gate_wait = gate_wait.clone();
                    Box::pin(async move {
                        gate_wait.notified().await;
                        Ok(())
                    })
                },
            )
            .await;

        let s1 = scheduler.clone();
        let first = tokio::spawn(async move { s1.ensure_fresh("test:cred").await });
        // Give the first call time to reach the gated refresh closure and
        // land the credential in `Refreshing`.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            scheduler.state("test:cred"),
            Some(CredentialState::Refreshing { prior_failures: 0 })
        );

        // A second caller now dispatches while still Refreshing — gets
        // AlreadyInFlight and waits on the id's Notify.
        let s2 = scheduler.clone();
        let second = tokio::spawn(async move { s2.ensure_fresh("test:cred").await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!second.is_finished(), "second caller must be waiting, not returned yet");

        // Re-register while BOTH the first call is gated inside `refresh`
        // AND the second call is waiting on AlreadyInFlight.
        scheduler
            .register("test:cred", || Box::pin(async { true }), || Box::pin(async { Ok(()) }))
            .await;

        gate.notify_waiters();
        let first_result = tokio::time::timeout(Duration::from_secs(2), first)
            .await
            .expect("first caller must not hang after a concurrent re-register")
            .unwrap();
        assert!(first_result.is_ok());
        let second_result = tokio::time::timeout(Duration::from_secs(2), second)
            .await
            .expect("second (waiting) caller must not be orphaned by a concurrent re-register")
            .unwrap();
        assert!(second_result.is_ok());
    }

    #[tokio::test]
    async fn unregistering_a_credential_mid_refresh_wakes_a_waiting_second_caller() {
        // reagent re-review on #2275: unregister() dispatched
        // Command::Unregister without ever calling wake(), so a second
        // caller already parked on AlreadyInFlight's notified().await for
        // this id would hang forever once the entry was gone — nothing else
        // was ever going to notify it.
        let scheduler = Arc::new(RefreshScheduler::new());
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_wait = gate.clone();
        scheduler
            .register(
                "test:cred",
                || Box::pin(async { false }),
                move || {
                    let gate_wait = gate_wait.clone();
                    Box::pin(async move {
                        gate_wait.notified().await;
                        Ok(())
                    })
                },
            )
            .await;

        let s1 = scheduler.clone();
        let first = tokio::spawn(async move { s1.ensure_fresh("test:cred").await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            scheduler.state("test:cred"),
            Some(CredentialState::Refreshing { prior_failures: 0 })
        );

        let s2 = scheduler.clone();
        let second = tokio::spawn(async move { s2.ensure_fresh("test:cred").await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!second.is_finished(), "second caller must be waiting, not returned yet");

        // Unregister while BOTH the first call is gated inside `refresh` AND
        // the second call is waiting on AlreadyInFlight.
        scheduler.unregister("test:cred").await;

        let second_result = tokio::time::timeout(Duration::from_secs(2), second)
            .await
            .expect("second (waiting) caller must not hang after a concurrent unregister")
            .unwrap();
        assert!(
            second_result.is_err(),
            "credential is gone — the woken caller should see it as unregistered"
        );

        // Release the gate so the orphaned first call's own refresh
        // resolves and its spawned task doesn't leak past the test.
        gate.notify_waiters();
        let _ = tokio::time::timeout(Duration::from_secs(2), first).await;
    }
}
