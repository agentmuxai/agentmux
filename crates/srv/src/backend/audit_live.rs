// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tells the open Audit panes that a row was written (docs/specs/SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.1).
//!
//! The two writer threads ([`reactive::audit_store`](super::reactive::audit_store) and
//! [`audit_activity`](super::audit_activity)) call [`poke`] after each line they append. A pane that sees
//! `audit:changed` asks `audit.query` for the newest rows, so a row it is told about and a row it lists are the
//! same row, with the same id. A tool-heavy agent writes ten rows a second, so the pokes of one window
//! ([`WINDOW`]) become one event.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::mps::Broker;

/// The pokes of this long are one event.
pub const WINDOW: Duration = Duration::from_millis(100);

static POKE: OnceLock<Sender<()>> = OnceLock::new();

/// Start publishing `audit:changed` on `broker` for the rest of this process. The first call wins.
pub fn install_broker(broker: Arc<Broker>) {
    let publish = move || broker.publish_event::<super::mps_events::AuditChanged>(&(), vec![], 0);
    if let Ok(tx) = spawn(publish, WINDOW) {
        let _ = POKE.set(tx);
    }
}

/// A row was written. Nothing happens before [`install_broker`].
pub fn poke() {
    if let Some(tx) = POKE.get() {
        let _ = tx.send(());
    }
}

/// The thread that turns the pokes of each `window` into one call of `publish`. It ends when the sender is dropped.
fn spawn(publish: impl Fn() + Send + 'static, window: Duration) -> std::io::Result<Sender<()>> {
    let (tx, rx) = channel::<()>();
    std::thread::Builder::new().name("audit-live".into()).spawn(move || run(&rx, window, publish))?;
    Ok(tx)
}

fn run(rx: &Receiver<()>, window: Duration, publish: impl Fn()) {
    while rx.recv().is_ok() {
        // The rest of the window's pokes are this event.
        std::thread::sleep(window);
        while rx.try_recv().is_ok() {}
        publish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_burst_of_pokes_is_one_event_and_a_later_poke_is_another() {
        let events = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&events);
        let tx = spawn(move || { seen.fetch_add(1, Ordering::SeqCst); }, Duration::from_millis(40)).unwrap();

        for _ in 0..25 {
            tx.send(()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(events.load(Ordering::SeqCst), 1);

        tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(events.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_poke_before_the_broker_is_attached_does_nothing() {
        poke();
    }
}
