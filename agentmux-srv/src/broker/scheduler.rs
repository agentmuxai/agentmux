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
//!    in-flight refresh via a per-id async mutex with a double-checked
//!    freshness read after acquiring it.
//! 2. **Reactive-only refresh.** `run_sweep_loop` proactively walks every
//!    registered credential on a timer instead of only refreshing when
//!    something happens to ask for one.
//!
//! Preserve-on-failure (never overwrite a valid stored credential with a
//! failed/partial refresh result) is NOT enforced here — it's the
//! responsibility of each registered `refresh` closure, since only the
//! caller knows its own store's write semantics. See `broker::mod` doc.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct Entry {
    is_fresh: Box<dyn Fn() -> bool + Send + Sync>,
    refresh: Box<dyn Fn() -> BoxFuture<'static, Result<(), String>> + Send + Sync>,
    lock: Arc<AsyncMutex<()>>,
}

pub struct RefreshScheduler {
    entries: AsyncMutex<HashMap<String, Arc<Entry>>>,
}

impl Default for RefreshScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshScheduler {
    pub fn new() -> Self {
        Self { entries: AsyncMutex::new(HashMap::new()) }
    }

    /// Register (or replace) a credential for proactive management.
    /// `is_fresh` must be cheap and side-effect-free (called under the
    /// per-id lock on every `ensure_fresh`/sweep tick); `refresh` performs
    /// the actual refresh and is responsible for persisting the result
    /// itself (and for NOT persisting anything on failure).
    pub async fn register(
        &self,
        credential_id: impl Into<String>,
        is_fresh: impl Fn() -> bool + Send + Sync + 'static,
        refresh: impl Fn() -> BoxFuture<'static, Result<(), String>> + Send + Sync + 'static,
    ) {
        let mut entries = self.entries.lock().await;
        entries.insert(
            credential_id.into(),
            Arc::new(Entry {
                is_fresh: Box::new(is_fresh),
                refresh: Box::new(refresh),
                lock: Arc::new(AsyncMutex::new(())),
            }),
        );
    }

    pub async fn unregister(&self, credential_id: &str) {
        self.entries.lock().await.remove(credential_id);
    }

    /// Ensure `credential_id` is fresh, single-flight-guarded. If another
    /// caller is already mid-refresh for this id, this waits for that
    /// refresh's lock to release, then re-checks freshness rather than
    /// starting a second refresh. Returns `Err` if the id was never
    /// registered.
    pub async fn ensure_fresh(&self, credential_id: &str) -> Result<(), String> {
        let entry = {
            let entries = self.entries.lock().await;
            entries
                .get(credential_id)
                .cloned()
                .ok_or_else(|| format!("no credential registered as '{credential_id}'"))?
        };
        let _guard = entry.lock.lock().await;
        // Double-checked: another caller may have refreshed this credential
        // while we were waiting for the lock above.
        if (entry.is_fresh)() {
            return Ok(());
        }
        (entry.refresh)().await
    }

    /// Background sweep — checks every registered credential every
    /// `interval` and proactively refreshes anything stale. Spawn once via
    /// `tokio::spawn`; runs until the process exits.
    pub async fn run_sweep_loop(self: Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let ids: Vec<String> = {
                let entries = self.entries.lock().await;
                entries.keys().cloned().collect()
            };
            for id in ids {
                if let Err(e) = self.ensure_fresh(&id).await {
                    tracing::warn!(credential_id = %id, error = %e, "broker: proactive refresh failed");
                }
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
                move || fresh_flag_check.load(Ordering::SeqCst),
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
    }

    #[tokio::test]
    async fn already_fresh_never_calls_refresh() {
        let scheduler = RefreshScheduler::new();
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let calls = refresh_calls.clone();
        scheduler
            .register(
                "test:cred",
                || true,
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
                || false, // never becomes fresh in this test — refresh always fails
                || Box::pin(async { Err("simulated refresh failure".to_string()) }),
            )
            .await;

        let result = scheduler.ensure_fresh("test:cred").await;
        assert!(result.is_err());
        // A second call must attempt the refresh again (not treat the
        // failed attempt as having succeeded).
        let result2 = scheduler.ensure_fresh("test:cred").await;
        assert!(result2.is_err());
    }

    #[tokio::test]
    async fn unregistered_credential_errors_without_panicking() {
        let scheduler = RefreshScheduler::new();
        let result = scheduler.ensure_fresh("nope").await;
        assert!(result.is_err());
    }
}
