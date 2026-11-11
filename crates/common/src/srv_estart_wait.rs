// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Waiting for srv's readiness line (`AGENTMUXSRV-ESTART`), shared by the launcher and the CEF host.
//!
//! The caller reads srv's stderr, parses each line with [`crate::srv_stderr`], and feeds four channels; this waits
//! on them. A `MIGRATING` line extends the deadline from 30 seconds to 30 minutes: a shorter cap risks killing srv
//! mid-migration, and 0011_shared_store_backfill's skip-guards check for non-empty shared tables, so a partial copy
//! followed by a kill would strand rows on the next retry.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};

/// How long srv has to say it is ready.
pub const NORMAL_TIMEOUT: Duration = Duration::from_secs(30);
/// The deadline once srv says it is migrating.
pub const MIGRATION_TIMEOUT: Duration = Duration::from_secs(1800);

/// How the wait ended.
pub enum EstartWait<T> {
    /// srv is ready (`Some`), or the ready channel closed without it (`None`): srv died before ESTART.
    Ready(Option<T>),
    Timeout,
    /// srv reported a fatal migration failure and exits 1 without ESTART.
    MigrationFailed(String),
    /// srv's data needs `migrate --prepare`; srv exits right after the line.
    NeedsPrepare(String),
}

/// What the stderr reader sends. `needs_prepare` is `None` for a caller that does not run the prepare step.
pub struct EstartChannels<T> {
    pub ready: mpsc::Receiver<T>,
    pub migrating: mpsc::Receiver<()>,
    pub failed: mpsc::Receiver<String>,
    pub needs_prepare: Option<mpsc::Receiver<String>>,
}

async fn recv_prepare(rx: &mut Option<mpsc::Receiver<String>>) -> Option<String> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Second guard for the ESTART-vs-failure race (#3043). The stderr reader sends the
/// migration-failure reason and then drops every sender as srv exits, so by the time the waiter's `select!` runs,
/// BOTH "failure buffered" and "ready channel closed" are ready. `biased;` makes the failure arm win when both are
/// seen in one poll; this covers the remaining shape, where the closed channel was observed first and the reason is
/// sitting in `failed` unread. Every other outcome passes through unchanged.
fn prefer_buffered_failure<T>(outcome: EstartWait<T>, failed: &mut mpsc::Receiver<String>) -> EstartWait<T> {
    match outcome {
        EstartWait::Ready(None) => match failed.try_recv() {
            Ok(reason) => EstartWait::MigrationFailed(reason),
            Err(_) => EstartWait::Ready(None),
        },
        other => other,
    }
}

/// Wait for srv to be ready, fail, or run out of time. `on_extend` runs when a `MIGRATING` line pushes the
/// deadline out, for the caller's log line. The caller kills srv on every outcome but `Ready(Some(_))`.
pub async fn wait_for_estart<T>(ch: &mut EstartChannels<T>, mut on_extend: impl FnMut()) -> EstartWait<T> {
    let mut deadline = Instant::now() + NORMAL_TIMEOUT;
    let sleep = sleep_until(deadline);
    tokio::pin!(sleep);
    let outcome = loop {
        tokio::select! {
            // `biased;` polls the arms in order, failure first: the reader sends the reason and then hits EOF
            // (srv exits 1 right after the line), dropping every sender before this is next polled. Unbiased,
            // `ready` resolving to None could win over the buffered reason and degrade to the generic
            // "channel closed". `prefer_buffered_failure` is the second guard.
            biased;
            Some(reason) = ch.failed.recv() => break EstartWait::MigrationFailed(reason),
            Some(reason) = recv_prepare(&mut ch.needs_prepare) => break EstartWait::NeedsPrepare(reason),
            result = ch.ready.recv() => break EstartWait::Ready(result),
            _ = &mut sleep => break EstartWait::Timeout,
            Some(()) = ch.migrating.recv() => {
                let new_deadline = Instant::now() + MIGRATION_TIMEOUT;
                if new_deadline > deadline {
                    deadline = new_deadline;
                    sleep.as_mut().reset(deadline);
                    on_extend();
                }
            }
        }
    };
    prefer_buffered_failure(outcome, &mut ch.failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Senders<T> = (mpsc::Sender<T>, mpsc::Sender<String>, mpsc::Sender<()>);

    fn channels<T>() -> (EstartChannels<T>, Senders<T>) {
        let (ready_tx, ready) = mpsc::channel(1);
        let (migrating_tx, migrating) = mpsc::channel(4);
        let (failed_tx, failed) = mpsc::channel(1);
        (EstartChannels { ready, migrating, failed, needs_prepare: None }, (ready_tx, failed_tx, migrating_tx))
    }

    #[test]
    fn closed_ready_channel_with_buffered_failure_becomes_migration_failed() {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        // The reader's exact sequence: send the reason, then drop the sender.
        tx.try_send("migration 0007 failed: disk I/O error".into()).unwrap();
        drop(tx);
        match prefer_buffered_failure(EstartWait::<u8>::Ready(None), &mut rx) {
            EstartWait::MigrationFailed(r) => assert_eq!(r, "migration 0007 failed: disk I/O error"),
            _ => panic!("buffered failure reason must win over a closed ready channel"),
        }
    }

    #[test]
    fn closed_ready_channel_without_failure_stays_channel_closed() {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        drop(tx); // srv died before ESTART for some other reason: no failure line
        assert!(matches!(prefer_buffered_failure(EstartWait::<u8>::Ready(None), &mut rx), EstartWait::Ready(None)));
    }

    #[test]
    fn other_outcomes_pass_through_untouched() {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        tx.try_send("should not be consumed".into()).unwrap();
        assert!(matches!(prefer_buffered_failure(EstartWait::<u8>::Timeout, &mut rx), EstartWait::Timeout));
        assert!(matches!(
            prefer_buffered_failure(EstartWait::<u8>::MigrationFailed("x".into()), &mut rx),
            EstartWait::MigrationFailed(_)
        ));
    }

    #[tokio::test]
    async fn a_ready_line_ends_the_wait() {
        let (mut ch, (ready_tx, _failed_tx, _migrating_tx)) = channels::<u8>();
        ready_tx.send(7).await.unwrap();
        assert!(matches!(wait_for_estart(&mut ch, || {}).await, EstartWait::Ready(Some(7))));
    }

    #[tokio::test]
    async fn a_failure_then_a_closed_reader_is_the_failure() {
        let (mut ch, (ready_tx, failed_tx, migrating_tx)) = channels::<u8>();
        failed_tx.send("disk full".into()).await.unwrap();
        drop((ready_tx, failed_tx, migrating_tx));
        match wait_for_estart(&mut ch, || {}).await {
            EstartWait::MigrationFailed(r) => assert_eq!(r, "disk full"),
            _ => panic!("the reason must win over the closed channels"),
        }
    }

    #[tokio::test]
    async fn a_closed_reader_with_nothing_buffered_is_a_closed_channel() {
        let (mut ch, senders) = channels::<u8>();
        drop(senders);
        assert!(matches!(wait_for_estart(&mut ch, || {}).await, EstartWait::Ready(None)));
    }

    #[tokio::test]
    async fn a_prepare_line_ends_the_wait_when_the_caller_listens_for_it() {
        let (mut ch, _senders) = channels::<u8>();
        let (prepare_tx, prepare_rx) = mpsc::channel(1);
        ch.needs_prepare = Some(prepare_rx);
        prepare_tx.send("old schema".into()).await.unwrap();
        match wait_for_estart(&mut ch, || {}).await {
            EstartWait::NeedsPrepare(r) => assert_eq!(r, "old schema"),
            _ => panic!("expected NeedsPrepare"),
        }
    }

    #[tokio::test]
    async fn a_migrating_line_extends_the_deadline_only_once_per_push() {
        let (mut ch, (ready_tx, _failed_tx, migrating_tx)) = channels::<u8>();
        migrating_tx.send(()).await.unwrap();
        ready_tx.send(1).await.unwrap();
        let mut extended = 0;
        // The ready value is already buffered, so the wait ends at once; it extends at most once for the one ping.
        let outcome = wait_for_estart(&mut ch, || extended += 1).await;
        assert!(matches!(outcome, EstartWait::Ready(Some(1))));
        assert!(extended <= 1);
    }
}
