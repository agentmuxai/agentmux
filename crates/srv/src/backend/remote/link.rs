// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether an SSH connection is up, as its Remotes row and the overlay on its
//! panes show it (`status`), kept true to the connection itself.
//!
//! On macOS and Linux every `ssh` srv runs on a connection shares one
//! connection with the others (`ControlMaster`, ssh.rs), and that shared
//! connection is the truth: it exists only once the user has logged in, and
//! `ssh -O check` answers then and only then. A pane's `ssh` starting is only
//! connecting (it may be waiting at a password prompt), and the connection can
//! outlive its last pane (`ControlPersist`) or end under it. So while a
//! connection is in use a watcher asks it, every half second while connecting
//! and every 15 s once up, and moves the status when the answer changes:
//! connected once logged in, disconnected once it is gone, whenever that is.
//!
//! Where ssh can't share connections (Windows) there is nothing to ask: a
//! running `ssh` counts as connected, as it always did.
//!
//! The user's Disconnect ([`disconnect`]) closes the shared connection, and
//! the connection stays down until something logs in to it again: durable
//! panes on it wait for the user ([`held`]) instead of logging back in.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;

use super::host::HostSsh;
use super::status::{self, state};
use crate::backend::mps::Broker;

/// How soon the watcher asks again while a login may be under way.
const WHILE_CONNECTING: Duration = Duration::from_millis(500);
/// How soon it asks again while the connection is up.
const WHILE_UP: Duration = Duration::from_secs(15);

static BROKER: OnceLock<Arc<Broker>> = OnceLock::new();

/// Where status changes are published, set once at startup. Until then (and
/// in unit tests) a status is recorded but not published.
pub fn init(broker: Arc<Broker>) {
    let _ = BROKER.set(broker);
}

fn broker() -> Option<&'static Broker> {
    BROKER.get().map(Arc::as_ref)
}

/// Whether ssh shares connections here, so there is one to ask.
fn shared() -> bool {
    super::ssh::shared_control_dir().is_some()
}

/// A pane's process on `connection` started (a plain pane's `ssh`, a durable
/// pane's attach). `logged_in`: it is known to be a live login already (a
/// durable pane's helper answered); otherwise it is connecting until the
/// shared connection answers.
pub fn pane_started(connection: &str, logged_in: bool) {
    release(connection);
    let shared = shared();
    status::pane_started(broker(), connection, logged_in || !shared);
    if shared {
        watch(connection);
    }
}

/// A pane's process on `connection` ended; `error` when it had no connection
/// or lost it. A connection the user disconnected ended because they asked:
/// not an error.
pub fn pane_ended(connection: &str, error: Option<&str>) {
    let error = if held(connection) { None } else { error };
    let shared = shared();
    status::pane_ended(broker(), connection, error, !shared);
    if shared {
        watch(connection);
    }
}

/// Log in over `connection` (`attempt`: a command over it), holding the
/// connection open meanwhile like a pane, so its status is connecting while
/// the user answers a prompt, connected once the command succeeded, and the
/// failure if it didn't.
pub async fn login<T>(
    connection: &str,
    attempt: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    pane_started(connection, false);
    let result = attempt.await;
    match &result {
        Ok(_) => {
            // Up now: said before the end below, which would otherwise find
            // the connection still connecting and leave it to the watcher.
            if shared() {
                status::set(broker(), connection, state::CONNECTED, None);
            }
            pane_ended(connection, None);
        }
        Err(message) => pane_ended(connection, Some(message)),
    }
    result
}

/// Something used `connection` successfully (a command over the shared
/// connection, such as a Files pane's helper starting): it is up, and its
/// status follows it from here. Nothing to say where ssh doesn't share
/// connections: nothing stays up after the command.
pub fn used(connection: &str) {
    if !shared() {
        return;
    }
    release(connection);
    status::set(broker(), connection, state::CONNECTED, None);
    watch(connection);
}

/// The user's Disconnect: close `connection`'s shared connection (every
/// session on it ends) and keep it down until something logs in again.
pub async fn disconnect(connection: &str) {
    hold(connection);
    if let Ok(host) = HostSsh::for_connection(connection) {
        host.close_shared_connection().await;
    }
    status::set(broker(), connection, state::DISCONNECTED, None);
}

// ── Held down by the user ───────────────────────────────────────────────────

fn held_set() -> &'static Mutex<HashSet<String>> {
    static HELD: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    HELD.get_or_init(Default::default)
}

/// Whether the user disconnected `connection` and nothing has logged in to it
/// since: a durable pane that lost it waits for the user instead of
/// reconnecting.
pub fn held(connection: &str) -> bool {
    held_set().lock().unwrap_or_else(|e| e.into_inner()).contains(connection)
}

/// The line a pane on `connection` shows when the user's Disconnect ended it.
pub fn disconnected_note(connection: &str) -> String {
    format!("Disconnected from {connection}. Press Enter to reconnect.")
}

fn hold(connection: &str) {
    held_set().lock().unwrap_or_else(|e| e.into_inner()).insert(connection.to_string());
}

fn release(connection: &str) {
    held_set().lock().unwrap_or_else(|e| e.into_inner()).remove(connection);
}

// ── The watcher ─────────────────────────────────────────────────────────────

fn watches() -> &'static Mutex<HashMap<String, Arc<Notify>>> {
    static WATCHES: OnceLock<Mutex<HashMap<String, Arc<Notify>>>> = OnceLock::new();
    WATCHES.get_or_init(Default::default)
}

/// Make sure `connection` is watched, and have its watcher ask now.
fn watch(connection: &str) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let mut watches = watches().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ask_now) = watches.get(connection) {
        ask_now.notify_one();
        return;
    }
    let Ok(host) = HostSsh::for_connection(connection) else {
        return;
    };
    let ask_now = Arc::new(Notify::new());
    watches.insert(connection.to_string(), ask_now.clone());
    runtime.spawn(run(connection.to_string(), host, ask_now));
}

async fn run(connection: String, host: HostSsh, ask_now: Arc<Notify>) {
    loop {
        let up = host.shared_connection_up().await;
        let wait = {
            // Decided under the watch lock: a pane starting meanwhile is
            // either counted here, or finds this watcher gone and starts one.
            let mut watches = watches().lock().unwrap_or_else(|e| e.into_inner());
            let now = status::get(&connection);
            let step = step(up, now.as_ref().map(|s| (s.status.as_str(), s.activeconnnum)), held(&connection));
            if let Some(status) = step.status {
                status::set(broker(), &connection, status, None);
            }
            match step.next {
                Some(wait) => wait,
                None => {
                    if watches.get(&connection).is_some_and(|w| Arc::ptr_eq(w, &ask_now)) {
                        watches.remove(&connection);
                    }
                    return;
                }
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = ask_now.notified() => {}
        }
    }
}

/// What the watcher does with one answer.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    /// The status to move to, if it changes.
    status: Option<&'static str>,
    /// When to ask again; `None`: stop watching.
    next: Option<Duration>,
}

/// The watcher's decision, from the shared connection's answer (`up`; `None`
/// when it couldn't be asked), the connection's status and open panes, and
/// whether the user disconnected it ([`held`]): every login releases that
/// first, so a held connection's "up" is an answer from before the
/// Disconnect closed it, and is taken as down.
fn step(up: Option<bool>, now: Option<(&str, i32)>, held: bool) -> Step {
    let up = if held { up.map(|_| false) } else { up };
    let (status, panes) = now.unwrap_or((state::DISCONNECTED, 0));
    let change = |to: &'static str| (status != to).then_some(to);
    match up {
        Some(true) => Step { status: change(state::CONNECTED), next: Some(WHILE_UP) },
        // Not up, a pane still on it: logging in (its ssh at a prompt), or the
        // connection just went and the pane's ssh is about to say so.
        Some(false) if panes > 0 => Step { status: None, next: Some(WHILE_CONNECTING) },
        // Gone, nothing on it: settled. A failure keeps its reason.
        Some(false) => Step {
            status: (status == state::CONNECTED || status == state::CONNECTING).then_some(state::DISCONNECTED),
            next: None,
        },
        // Couldn't ask: change nothing, ask again.
        None => Step {
            status: None,
            next: Some(if status == state::CONNECTED { WHILE_UP } else { WHILE_CONNECTING }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(status: Option<&'static str>, next: Option<Duration>) -> Step {
        Step { status, next }
    }

    /// The password host turned green while ssh was still asking for the
    /// password (#4513): connecting stays connecting until the shared
    /// connection answers, which it does only after the login.
    #[test]
    fn connecting_turns_connected_only_when_the_connection_answers() {
        assert_eq!(step(Some(false), Some((state::CONNECTING, 1)), false), s(None, Some(WHILE_CONNECTING)));
        assert_eq!(step(Some(true), Some((state::CONNECTING, 1)), false), s(Some(state::CONNECTED), Some(WHILE_UP)));
        assert_eq!(step(Some(true), Some((state::CONNECTED, 1)), false), s(None, Some(WHILE_UP)));
    }

    /// A host stayed green after its connection was gone (#4513): once the
    /// connection is gone and no pane is on it, it is disconnected, also when
    /// it outlived its last pane and then expired.
    #[test]
    fn a_connection_gone_with_nothing_on_it_is_disconnected() {
        assert_eq!(step(Some(false), Some((state::CONNECTED, 0)), false), s(Some(state::DISCONNECTED), None));
        assert_eq!(step(Some(false), Some((state::CONNECTING, 0)), false), s(Some(state::DISCONNECTED), None));
        assert_eq!(step(Some(false), Some((state::DISCONNECTED, 0)), false), s(None, None));
        assert_eq!(step(Some(false), None, false), s(None, None));
    }

    /// A failure's reason stays shown; the watcher doesn't paper over it.
    #[test]
    fn a_failure_keeps_its_reason() {
        assert_eq!(step(Some(false), Some((state::ERROR, 0)), false), s(None, None));
    }

    /// Up with no pane on it (`ControlPersist`, or a Connect): still up, and
    /// watched until it expires.
    #[test]
    fn a_connection_outliving_its_panes_stays_up_and_watched() {
        assert_eq!(step(Some(true), Some((state::CONNECTED, 0)), false), s(None, Some(WHILE_UP)));
        assert_eq!(step(Some(true), Some((state::DISCONNECTED, 0)), false), s(Some(state::CONNECTED), Some(WHILE_UP)));
    }

    /// Disconnect closes the connection, but an answer the watcher got just
    /// before can arrive just after: reported, it showed the connection up
    /// again, and a finished durable pane restarted over it, undoing the
    /// Disconnect (found live). Every login releases the hold first, so a
    /// held connection's "up" is that stale answer.
    #[test]
    fn a_connection_the_user_disconnected_is_never_reported_up() {
        assert_eq!(step(Some(true), Some((state::DISCONNECTED, 0)), true), s(None, None));
        assert_eq!(step(Some(true), Some((state::DISCONNECTED, 1)), true), s(None, Some(WHILE_CONNECTING)));
    }

    /// When ssh couldn't be asked, nothing changes and it is asked again.
    #[test]
    fn no_answer_changes_nothing() {
        assert_eq!(step(None, Some((state::CONNECTED, 1)), false), s(None, Some(WHILE_UP)));
        assert_eq!(step(None, Some((state::CONNECTING, 1)), false), s(None, Some(WHILE_CONNECTING)));
    }

    /// A connection the user disconnected ends its panes because they asked:
    /// not an error, and it stays held until a login.
    #[test]
    fn a_disconnect_holds_the_connection_until_a_login() {
        let name = "test-link-held";
        hold(name);
        assert!(held(name));
        pane_ended(name, Some("Shared connection closed"));
        assert_ne!(status::get(name).map(|s| s.status), Some(state::ERROR.to_string()));
        pane_started(name, false);
        assert!(!held(name), "a login releases it");
    }
}
