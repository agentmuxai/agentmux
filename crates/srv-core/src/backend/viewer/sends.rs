// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each paired device has sent lately: a rate limit, and the
//! `message_id`s already taken, so a retry after a lost reply is answered
//! from memory instead of delivered twice (agentmux-mobile's
//! SPEC_SEND_FROM_THE_PHONE_2026_10_13 §4, §5.2.1). Memory only: a restart
//! forgets it, which a retry 10 minutes later would too.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Sends one device may make in [`RATE_WINDOW`].
pub const RATE_LIMIT: usize = 30;
pub const RATE_WINDOW: Duration = Duration::from_secs(60);
/// How long a `message_id` is remembered.
pub const REMEMBER: Duration = Duration::from_secs(600);

/// How a delivered message reached the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// The agent has it.
    Delivered,
    /// Waiting for the agent's process (starting, restarting), which reads
    /// it when it can.
    Queued,
}

impl Sent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Queued => "queued",
        }
    }
}

/// Whether a send may go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// A new message: deliver it, then [`SendLedger::finish`].
    New,
    /// Already delivered: answer as before.
    Duplicate(Sent),
    /// The same message is being delivered right now.
    InFlight,
    /// Too many sends; try again after this long.
    RateLimited(Duration),
}

#[derive(Default)]
struct DeviceSends {
    /// When each send in the window was admitted, oldest first.
    recent: VecDeque<Instant>,
    /// `message_id` → its outcome (`None` while in flight) and when it came.
    ids: HashMap<String, (Option<Sent>, Instant)>,
}

#[derive(Default)]
pub struct SendLedger {
    by_device: HashMap<String, DeviceSends>,
}

impl SendLedger {
    pub fn admit(&mut self, device_id: &str, message_id: &str, now: Instant) -> Admit {
        let sends = self.by_device.entry(device_id.to_string()).or_default();
        sends.ids.retain(|_, (_, at)| now.duration_since(*at) < REMEMBER);
        while sends.recent.front().is_some_and(|at| now.duration_since(*at) >= RATE_WINDOW) {
            sends.recent.pop_front();
        }
        match sends.ids.get(message_id) {
            Some((Some(sent), _)) => return Admit::Duplicate(*sent),
            Some((None, _)) => return Admit::InFlight,
            None => {}
        }
        if sends.recent.len() >= RATE_LIMIT {
            let oldest = *sends.recent.front().expect("non-empty at the limit");
            return Admit::RateLimited(RATE_WINDOW.saturating_sub(now.duration_since(oldest)));
        }
        sends.recent.push_back(now);
        sends.ids.insert(message_id.to_string(), (None, now));
        Admit::New
    }

    /// Record how an admitted message went. `None`: it failed, and a retry
    /// may try again.
    pub fn finish(&mut self, device_id: &str, message_id: &str, outcome: Option<Sent>) {
        let Some(sends) = self.by_device.get_mut(device_id) else { return };
        match outcome {
            Some(sent) => {
                if let Some(entry) = sends.ids.get_mut(message_id) {
                    entry.0 = Some(sent);
                }
            }
            None => {
                sends.ids.remove(message_id);
            }
        }
    }

    /// Forget a revoked device.
    pub fn forget_device(&mut self, device_id: &str) {
        self.by_device.remove(device_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_id_is_delivered_once_and_a_failed_one_can_be_retried() {
        let mut ledger = SendLedger::default();
        let t = Instant::now();
        assert_eq!(ledger.admit("d", "m1", t), Admit::New);
        assert_eq!(ledger.admit("d", "m1", t), Admit::InFlight, "still being delivered");
        ledger.finish("d", "m1", Some(Sent::Queued));
        assert_eq!(ledger.admit("d", "m1", t), Admit::Duplicate(Sent::Queued));
        assert_eq!(ledger.admit("other", "m1", t), Admit::New, "ids are per device");

        assert_eq!(ledger.admit("d", "m2", t), Admit::New);
        ledger.finish("d", "m2", None);
        assert_eq!(ledger.admit("d", "m2", t), Admit::New, "a failed send may be tried again");

        // Remembered for REMEMBER, then a new message as far as srv knows.
        assert_eq!(ledger.admit("d", "m1", t + REMEMBER), Admit::New);
    }

    #[test]
    fn thirty_sends_a_minute_then_wait_for_the_oldest_to_leave_the_window() {
        let mut ledger = SendLedger::default();
        let t = Instant::now();
        for i in 0..RATE_LIMIT {
            let at = t + Duration::from_secs(i as u64);
            assert_eq!(ledger.admit("d", &format!("m{i}"), at), Admit::New);
            ledger.finish("d", &format!("m{i}"), Some(Sent::Delivered));
        }
        let at = t + Duration::from_secs(40);
        assert_eq!(ledger.admit("d", "late", at), Admit::RateLimited(Duration::from_secs(20)));
        assert_eq!(ledger.admit("d", "m3", at), Admit::Duplicate(Sent::Delivered), "a retry is answered even when limited");
        assert_eq!(ledger.admit("other", "x", at), Admit::New, "the limit is per device");
        assert_eq!(ledger.admit("d", "late", t + RATE_WINDOW), Admit::New, "the oldest left the window");
        ledger.forget_device("d");
        assert_eq!(ledger.admit("d", "m3", at), Admit::New, "a revoked device's ids are gone");
    }
}
