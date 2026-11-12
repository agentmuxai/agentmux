// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What bounds one WebSocket session of the cloud subscriber: how long it may
//! take to open and how long it may go without a word from the relay. Without
//! them a socket that went dead (or a handshake that never finished) held the
//! session open for good while the status said connected.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::collections::HashSet;

use super::supervisor::beat;
use super::{shared_token_now, CLIENT_PING_INTERVAL_SECS};
use crate::backend::storage::store::Store;

/// The client of the subscriber's REST calls and token refreshes. A connection that
/// cannot be made in 10 s is a failure to retry, not a wait.
pub(super) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(crate::muxbus::relay::USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// How long a connection may take to open, and how long it may be silent.
#[derive(Clone, Copy)]
pub(super) struct SessionLimits {
    /// The WebSocket handshake.
    pub(super) connect: Duration,
    /// Nothing heard from the relay (a pong to our ping counts): the socket is
    /// closed and made again. The relay answers every ping, so two unanswered
    /// ones (and a margin, still under the relay's own 10 minute cut) mean the
    /// connection is dead whatever the OS still says about it.
    pub(super) silence: Duration,
    /// How often the silence is checked.
    pub(super) check_every: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        SessionLimits {
            connect: Duration::from_secs(30),
            silence: Duration::from_secs(2 * CLIENT_PING_INTERVAL_SECS + 60),
            check_every: Duration::from_secs(30),
        }
    }
}

/// Notices a session that stopped hearing from the relay, and stamps the
/// subscriber's progress while the session waits.
pub(super) struct Watchdog {
    last_heard: Instant,
    silence: Duration,
    check: tokio::time::Interval,
}

impl Watchdog {
    pub(super) async fn new(limits: SessionLimits) -> Self {
        let mut check = tokio::time::interval(limits.check_every);
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        check.tick().await; // the first tick is immediate: the session just opened
        Watchdog { last_heard: Instant::now(), silence: limits.silence, check }
    }

    /// Something came from the relay: a wake, a pong, a close.
    pub(super) fn heard(&mut self) {
        self.last_heard = Instant::now();
    }

    /// Resolves, with why, once the relay has been silent for too long. A session
    /// stuck in a handler never gets here, and the supervisor sees the missing stamp.
    pub(super) async fn silent(&mut self) -> String {
        loop {
            self.check.tick().await;
            beat();
            if self.last_heard.elapsed() > self.silence {
                return format!("no word from the relay for {}s", self.last_heard.elapsed().as_secs());
            }
        }
    }
}

/// An agent went away here: let another instance have it, once the release has a fresh token.
pub(super) fn release_in_background(id: String, base: String, token: String, http: reqwest::Client, mstore: Arc<Store>, agents: Arc<Mutex<HashSet<String>>>) {
    tokio::spawn(async move {
        let token = shared_token_now(&mstore, &token).await;
        // Skip if it was subscribed again meanwhile: its
        // fresh claim must not be released (Codex P2, #3897).
        let still_gone = || !agents.lock().unwrap().contains(&id);
        crate::muxbus::wan_lease::release(&base, &id, &token, &http, still_gone).await
    });
}
