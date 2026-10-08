// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether cloud (MuxBus) messages are reaching this channel's agents, made
//! visible: a sign-in that stops working, or a relay that can't be reached,
//! used to leave nothing but a log line while agents silently stopped getting
//! review, CI and cross-machine messages.
//!
//! **State** (`muxbus.status`'s `delivery`, and the `muxbus:status` event on
//! every change) is derived in [`machine::derive`] from two things only: what
//! the cloud subscriber's loop last reported ([`set_link`], [`saw_session`])
//! and the credential broker's state for the sign-in. It never reads the
//! credential store itself.
//!
//! **What follows from it** ([`machine::Effect`]):
//! - one OS notification when a needs-sign-in episode starts (the notify
//!   Router's `CloudSignedOut` kind), taken down when it ends;
//! - after [`machine::PAUSE_NOTICE_MS`] paused, one AgentMux system note to
//!   every agent subscribed to the cloud, and one more when delivery resumes,
//!   with what was delivered and what expired meanwhile ([`notes`]). The
//!   notes are delivered locally by srv, never through the relay; see
//!   [`notes::NOTE_MARKER`] for why one can't be forged.

mod machine;
pub mod notes;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use agentmux_common::time::now_ms;
pub use machine::Link;
use machine::{Effect, Machine};
use notes::{ExpiredItem, Note, NoteBook, RetryQueue};

use crate::backend::eventbus::{EventBus, WSEventType};
use crate::backend::mps::Broker;
use crate::backend::rpc_types::MuxBusDeliveryStatus;
use crate::broker::CredentialState;

/// The WebSocket event carrying a changed [`MuxBusDeliveryStatus`].
pub const EVENT_MUXBUS_STATUS: &str = "muxbus:status";

/// How often the state is re-derived (the broker can move on its own) and
/// the time-based notes are checked.
const TICK: Duration = Duration::from_secs(15);

struct Sinks {
    event_bus: Arc<EventBus>,
    broker: Arc<Broker>,
}

struct Runtime {
    machine: Mutex<Machine>,
    book: Mutex<NoteBook>,
    retry: Mutex<RetryQueue>,
    sinks: OnceLock<Sinks>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn rt() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime {
        machine: Mutex::new(Machine::new(now_ms())),
        book: Mutex::new(NoteBook::default()),
        retry: Mutex::new(RetryQueue::default()),
        sinks: OnceLock::new(),
    })
}

fn broker_state() -> Option<CredentialState> {
    crate::broker::get_global().and_then(|s| s.state(crate::muxbus::CREDENTIAL_ID))
}

fn subscribed_agents() -> Vec<String> {
    crate::muxbus::cloud_subscriber::get_global_subscriber()
        .map(|s| s.subscribed_agents())
        .unwrap_or_default()
}

/// Where status changes and the notification go. Call once at startup;
/// starts the tick. Before this, state is still tracked but nothing is sent.
pub fn install(event_bus: Arc<EventBus>, broker: Arc<Broker>) {
    if rt().sinks.set(Sinks { event_bus, broker }).is_err() {
        return;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            run_tick();
        }
    });
}

fn run_tick() {
    let broker = broker_state();
    let now = now_ms();
    let agents = subscribed_agents();
    let effects = {
        let mut machine = rt().machine.lock().unwrap_or_else(|e| e.into_inner());
        machine.keep_pull_errors_of(&agents);
        machine.tick(broker.as_ref(), now)
    };
    apply(effects);
    send_owed_sign_in_notice();
    let retries = rt().retry.lock().unwrap_or_else(|e| e.into_inner()).take();
    for (note, attempts) in retries {
        deliver_one(note, attempts + 1);
    }
    let notes = {
        let mut book = rt().book.lock().unwrap_or_else(|e| e.into_inner());
        let mut notes = book.pause_late_joiners(&agents);
        notes.extend(book.tick(now));
        notes
    };
    deliver(notes);
}

/// The current state, for `muxbus.status`.
pub fn status() -> MuxBusDeliveryStatus {
    rt().machine.lock().unwrap_or_else(|e| e.into_inner()).status().clone()
}

/// The subscriber found a stored sign-in, or a sign-in just succeeded.
pub fn saw_session(email: Option<String>) {
    let broker = broker_state();
    let effects = rt().machine.lock().unwrap_or_else(|e| e.into_inner()).saw_session(email, broker.as_ref(), now_ms());
    apply(effects);
}

/// The user signed out (`muxbus.disconnect`).
pub fn signed_out() {
    let effects = rt().machine.lock().unwrap_or_else(|e| e.into_inner()).signed_out(now_ms());
    apply(effects);
}

/// What the subscriber's loop is doing now.
pub fn set_link(link: Link) {
    let broker = broker_state();
    let effects = rt().machine.lock().unwrap_or_else(|e| e.into_inner()).set_link(link, broker.as_ref(), now_ms());
    apply(effects);
}

/// One agent's pull answered: `delivered` messages reached it and the relay
/// reported `expired`. Sends its resume or expiry note, if it has one; call
/// after the pull's own messages are delivered, so the note comes last.
pub fn fetched(agent: &str, delivered: usize, expired: &[ExpiredItem]) {
    let broker = broker_state();
    let effects = rt().machine.lock().unwrap_or_else(|e| e.into_inner()).relay_answered(agent, broker.as_ref(), now_ms());
    apply(effects);
    let note = rt().book.lock().unwrap_or_else(|e| e.into_inner()).fetched(agent, delivered, expired);
    deliver(note.into_iter().collect());
}

/// `agent`'s pull (fetch or claim) got no usable answer from the relay,
/// though the WebSocket may be open: its messages aren't arriving.
pub fn pull_failed(agent: &str, reason: impl Into<String>) {
    let broker = broker_state();
    let effects =
        rt().machine.lock().unwrap_or_else(|e| e.into_inner()).pull_failed(agent, reason.into(), broker.as_ref(), now_ms());
    apply(effects);
}

/// The episode's notification is owed but had no router to go to (no window
/// had connected yet); [`run_tick`] sends it once one exists.
static SIGN_IN_NOTICE_OWED: AtomicBool = AtomicBool::new(false);

fn send_owed_sign_in_notice() {
    if SIGN_IN_NOTICE_OWED.load(Ordering::SeqCst) {
        if let Some(router) = router() {
            if SIGN_IN_NOTICE_OWED.swap(false, Ordering::SeqCst) {
                router.cloud_signed_out();
            }
        }
    }
}

fn apply(effects: Vec<Effect>) {
    for effect in effects {
        match effect {
            Effect::Changed(status) => {
                tracing::info!(
                    state = ?status.state,
                    error = status.last_error.as_deref().unwrap_or(""),
                    "muxbus delivery: state changed"
                );
                if let Some(sinks) = rt().sinks.get() {
                    sinks.event_bus.broadcast_event(&WSEventType {
                        eventtype: EVENT_MUXBUS_STATUS.to_string(),
                        oref: String::new(),
                        data: serde_json::to_value(&status).ok(),
                    });
                }
            }
            Effect::NotifySignInNeeded => {
                SIGN_IN_NOTICE_OWED.store(true, Ordering::SeqCst);
                send_owed_sign_in_notice();
            }
            Effect::RetractSignInNeeded => {
                SIGN_IN_NOTICE_OWED.store(false, Ordering::SeqCst);
                if let Some(router) = router() {
                    router.resolve("", crate::backend::notify::policy::Family::Cloud);
                }
            }
            Effect::PauseNote { since_ms, reason } => {
                tracing::warn!(reason, "muxbus delivery: paused for two minutes, telling the agents");
                let notes = rt().book.lock().unwrap_or_else(|e| e.into_inner()).pause(since_ms, reason, &subscribed_agents());
                deliver(notes);
            }
            Effect::Resumed { at_ms } => {
                rt().retry.lock().unwrap_or_else(|e| e.into_inner()).drop_pause_notes();
                rt().book.lock().unwrap_or_else(|e| e.into_inner()).resumed(at_ms);
            }
        }
    }
}

fn router() -> Option<Arc<crate::backend::notify::router::Router>> {
    let sinks = rt().sinks.get()?;
    let router = crate::backend::notify::router::get(&sinks.broker);
    if router.is_none() {
        tracing::debug!("muxbus delivery: no notification router yet (no window connected)");
    }
    router
}

fn deliver(notes: Vec<Note>) {
    for note in notes {
        deliver_one(note, 1);
    }
}

/// Deliver `note` (its `attempt`-th try); a failure is retried on later ticks.
fn deliver_one(note: Note, attempt: u32) {
    let handler = crate::backend::reactive::handler::get_global_handler();
    if let Err(e) = handler.deliver_system_note(&note.agent, &note.text) {
        let agent = note.agent.clone();
        if rt().retry.lock().unwrap_or_else(|e| e.into_inner()).failed(note, attempt) {
            tracing::info!(agent = %agent, attempt, error = %e, "muxbus delivery: system note not delivered, will retry");
        } else {
            tracing::warn!(agent = %agent, error = %e, "muxbus delivery: system note not delivered, giving up");
        }
    }
}
