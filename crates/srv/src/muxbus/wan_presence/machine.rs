// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The presence publisher's decisions, apart from its I/O. Given what just
//! happened ([`Event`]) and the time ([`Now`]), [`Machine::next`] settles the
//! state, says what to do ([`Action`]) and how long to wait before the next
//! [`Event::Due`]. Nothing here reads a clock, a random number or the
//! network, so every rule is tested with a fake clock; `super::Driver` runs
//! it against the relay.
//!
//! | state | entered on | next try |
//! |---|---|---|
//! | `signed_out` | no MuxBus sign-in | a sign-in (a local recheck every minute, no request) |
//! | `publishing` | the record was stored | a minute, or a change of the snapshot |
//! | `retrying` | unreachable, 5xx, 429, any other status | 5 s doubling to 5 min, full jitter; a 429's `Retry-After` |
//! | `unsupported` | 404, no presence route | about 5 min; at once on a sign-in, a network change, a wake, or a new relay version |
//! | `rejected` | 400 or 410 | 10 min or more |
//! | `signed_off` | a goodbye was stored at a sign-out | a sign-in |
//! | `off` | this install doesn't publish (`super::policy`) | turned on |
//!
//! Outside `publishing` a change of the snapshot only marks it dirty: the
//! next try sends the latest one, no sooner. A 409 means the relay already
//! has a newer record of this install's, which is as good as storing this
//! one.
//!
//! A goodbye (a v3 record, `gone`) is due when something was published and
//! the relay takes v2 ([`Machine::farewell`]): an older relay would refuse
//! v3, so it gets none. Turning publishing off says it here
//! ([`Action::Goodbye`]); a sign-out and a quit say it from outside, and a
//! sign-out tells the machine with [`Event::Goodbye`].
//!
//! Two refusals get one immediate second try before `rejected`: a record
//! the relay can't parse is sent again as v1 (an older relay knows no agent
//! states), and v1 is then kept for that relay for [`Config::v1_memory`]; a
//! record whose time the relay refuses is sent again stamped with the
//! relay's clock, learned from the `Date` of every answer.

use std::collections::HashMap;
use std::time::Duration;

use agentmux_common::install_presence::{PRESENCE_VERSION, PRESENCE_VERSION_V1};

use crate::backend::rpc_types::{PresenceOffReason, PresenceState, PresenceStatusResult};

/// The relay's `error` for a record it can't parse, which is what a relay
/// that predates the record's version answers.
pub(crate) const MALFORMED_RECORD: &str = "Malformed install presence record";

/// A clock further than this from the relay's is shown in the status.
const CLOCK_NOTE_AFTER_MS: u64 = 2 * 60 * 1000;

/// The waits, apart so tests can shorten them.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    /// Between two publishes of an unchanged record.
    pub interval: Duration,
    /// The first retry's ceiling; it doubles per failure.
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
    /// The least wait between retries, so full jitter never spins.
    pub backoff_floor: Duration,
    /// The longest `Retry-After` honoured.
    pub retry_after_cap: Duration,
    /// After a 404, jittered by a fifth either way.
    pub unsupported_retry: Duration,
    /// How often the relay's version is checked while `unsupported`.
    pub health_every: Duration,
    /// After a refusal, plus up to a fifth.
    pub rejected_retry: Duration,
    /// How often a signed-out publisher looks for a sign-in, locally.
    pub signed_out_recheck: Duration,
    /// How long a relay that took only v1 gets v1.
    pub v1_memory: Duration,
    /// An outage this long is logged as a warning, once.
    pub outage_warn_after: Duration,
}

pub(crate) const LIVE: Config = Config {
    interval: Duration::from_secs(60),
    backoff_base: Duration::from_secs(5),
    backoff_cap: Duration::from_secs(5 * 60),
    backoff_floor: Duration::from_secs(1),
    retry_after_cap: Duration::from_secs(60 * 60),
    unsupported_retry: Duration::from_secs(5 * 60),
    health_every: Duration::from_secs(60),
    rejected_retry: Duration::from_secs(10 * 60),
    signed_out_recheck: Duration::from_secs(60),
    v1_memory: Duration::from_secs(6 * 60 * 60),
    outage_warn_after: Duration::from_secs(10 * 60),
};

/// The time, as the driver read it. `mono_ms` only ever grows and drives the
/// waits; `wall_ms` is this computer's clock, for the status and the record;
/// `rand` is uniform in `[0, 1)`, for jitter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Now {
    pub mono_ms: u64,
    pub wall_ms: u64,
    pub rand: f64,
}

/// What the relay made of one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// 2xx.
    Stored,
    /// 404: the relay has no presence route.
    NoRoute,
    /// 400 [`MALFORMED_RECORD`].
    Malformed,
    /// 400 for the record's time: the relay's message.
    ClockRefused(String),
    /// Any other 400, a 410, or a record that couldn't be built: why.
    Refused(String),
    /// 429.
    TooMany { retry_after: Option<Duration> },
    /// Unreachable, 5xx or any other status: a short reason and the detail.
    Unavailable { reason: String, detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    /// The publisher started.
    Start,
    /// The last wait ran out.
    Due,
    /// The snapshot changed (while publishing: after the debounce).
    Changed,
    /// A try found no sign-in; nothing was sent.
    NoSignIn,
    /// A record of version `sent_v` was sent. `relay_date_ms` is the answer's
    /// `Date`, when it had a readable one.
    Answer { sent_v: u32, answer: Answer, relay_date_ms: Option<u64> },
    /// The relay's health check: its version, `None` when it failed.
    Health { version: Option<String>, relay_date_ms: Option<u64> },
    /// A sign-in, a sign-out or a new account.
    SignIn,
    /// This computer's network addresses changed.
    NetworkChanged,
    /// This computer woke from sleep.
    Woke,
    /// "Publish now": forget the backoff and try at once.
    PublishNow,
    /// Whether this install publishes: `None` to publish, else why not.
    Policy { off: Option<PresenceOffReason> },
    /// The settings changed: the driver reads the policy again and passes
    /// it on as [`Event::Policy`]; the machine itself ignores this.
    SettingsChanged,
    /// A goodbye went out (from [`Action::Goodbye`], or a sign-out); `stored`
    /// when the relay took it.
    Goodbye { stored: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Nothing; wait [`Step::wait`].
    Wait,
    /// Send the latest snapshot as a record of version `v`.
    Attempt { v: u32 },
    /// Ask the relay its version.
    CheckHealth,
    /// Send the goodbye, stamped `published_at_ms`, then report
    /// [`Event::Goodbye`].
    Goodbye { published_at_ms: u64 },
}

/// A goodbye is due, stamped on the relay's clock as far as it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Farewell {
    offset_ms: i64,
}

impl Farewell {
    /// `published_at_ms` for a goodbye sent at `wall_ms`.
    pub(crate) fn stamp(&self, wall_ms: u64) -> u64 {
        u64::try_from(wall_ms as i64 + self.offset_ms).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Log {
    pub warn: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    pub action: Action,
    /// Until the next [`Event::Due`]; meaningful when `action` is `Wait`.
    pub wait: Duration,
    pub logs: Vec<Log>,
}

/// The second tries already spent on the record being sent.
#[derive(Debug, Default, Clone, Copy)]
struct Followups {
    v1: bool,
    clock: bool,
}

#[derive(Debug)]
pub(crate) struct Machine {
    config: Config,
    relay: String,
    /// `None` until the first state is entered.
    state: Option<PresenceState>,
    since_wall_ms: u64,
    reason: Option<String>,
    last_ok_wall_ms: Option<u64>,
    failures: u32,
    due_ms: u64,
    health_due_ms: Option<u64>,
    dirty: bool,
    followups: Followups,
    /// Relays that took only v1, until when.
    v1_until_ms: HashMap<String, u64>,
    offset_ms: Option<i64>,
    relay_version: Option<String>,
    /// The relay's version just after its 404.
    version_at_404: Option<String>,
    outage_since_ms: Option<u64>,
    outage_warned: bool,
    /// Why this install doesn't publish; `None` while it does.
    off: Option<PresenceOffReason>,
    /// This machine asked for a goodbye ([`Action::Goodbye`]) whose result
    /// hasn't come back yet. A sign-out reports [`Event::Goodbye`] too, even
    /// when none was due, so only this says the result is worth logging.
    goodbye_sent: bool,
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn scaled(d: Duration, factor: f64) -> u64 {
    (ms(d) as f64 * factor) as u64
}

/// "12 min", rounded, at least 1.
fn minutes(ms: u64) -> u64 {
    ((ms + 30_000) / 60_000).max(1)
}

impl Machine {
    pub(crate) fn new(config: Config, relay: &str) -> Self {
        Self {
            config,
            relay: relay.trim_end_matches('/').to_string(),
            state: None,
            since_wall_ms: 0,
            reason: None,
            last_ok_wall_ms: None,
            failures: 0,
            due_ms: 0,
            health_due_ms: None,
            dirty: false,
            followups: Followups::default(),
            v1_until_ms: HashMap::new(),
            offset_ms: None,
            relay_version: None,
            version_at_404: None,
            outage_since_ms: None,
            outage_warned: false,
            off: None,
            goodbye_sent: false,
        }
    }

    pub(crate) fn state(&self) -> Option<PresenceState> {
        self.state
    }

    /// Whether a snapshot change is published (after the debounce), rather
    /// than only marked dirty.
    pub(crate) fn follows_changes(&self) -> bool {
        self.state == Some(PresenceState::Publishing)
    }

    #[cfg(test)]
    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    #[cfg(test)]
    pub(crate) fn set_relay(&mut self, relay: &str) {
        self.relay = relay.trim_end_matches('/').to_string();
    }

    /// `published_at_ms` for a record sent at `wall_ms`: on the relay's
    /// clock when an answer has told it, else this computer's.
    pub(crate) fn stamp(&self, wall_ms: u64) -> u64 {
        Farewell { offset_ms: self.offset_ms.unwrap_or(0) }.stamp(wall_ms)
    }

    /// Whether a goodbye is due if this install went away now: something
    /// was published, publishing is on and no goodbye has been said since,
    /// and the relay takes v2 (one that took only v1 would refuse v3).
    pub(crate) fn farewell(&self, now: &Now) -> Option<Farewell> {
        let due = self.off.is_none()
            && self.state != Some(PresenceState::SignedOff)
            && self.last_ok_wall_ms.is_some()
            && self.record_version(now) == PRESENCE_VERSION;
        due.then(|| Farewell { offset_ms: self.offset_ms.unwrap_or(0) })
    }

    /// The version a fresh try sends: v1 while this relay is remembered as
    /// taking only v1.
    pub(crate) fn record_version(&self, now: &Now) -> u32 {
        match self.v1_until_ms.get(&self.relay) {
            Some(&until) if now.mono_ms < until => PRESENCE_VERSION_V1,
            _ => PRESENCE_VERSION,
        }
    }

    pub(crate) fn next(&mut self, event: Event, now: Now) -> Step {
        let mut logs = Vec::new();
        let action = if self.off.is_some() {
            self.next_while_off(event, &now, &mut logs)
        } else {
            self.next_while_on(event, now, &mut logs)
        };
        self.warn_long_outage(&now, &mut logs);
        let due = match self.health_due_ms {
            Some(h) if self.state == Some(PresenceState::Unsupported) => self.due_ms.min(h),
            _ => self.due_ms,
        };
        Step { action, wait: Duration::from_millis(due.saturating_sub(now.mono_ms)), logs }
    }

    /// Off: nothing is sent until publishing is turned on again; a goodbye
    /// in flight when it was turned off lands here.
    fn next_while_off(&mut self, event: Event, now: &Now, logs: &mut Vec<Log>) -> Action {
        match event {
            Event::Policy { off: None } => {
                self.off = None;
                self.failures = 0;
                logs.push(Log { warn: false, text: "publishing is on".into() });
                self.attempt(None, now)
            }
            Event::Policy { off: Some(reason) } => {
                self.enter_off(reason, now, logs);
                Action::Wait
            }
            Event::Goodbye { stored } => {
                let reason = self.off.unwrap_or(PresenceOffReason::Setting);
                if std::mem::take(&mut self.goodbye_sent) {
                    logs.push(Log { warn: false, text: goodbye_text(stored).into() });
                }
                self.enter_off(reason, now, logs);
                Action::Wait
            }
            Event::Changed => {
                self.dirty = true;
                Action::Wait
            }
            _ => Action::Wait,
        }
    }

    /// Stop publishing for `reason`; one log line per new reason.
    fn enter_off(&mut self, reason: PresenceOffReason, now: &Now, logs: &mut Vec<Log>) {
        let changed = self.off != Some(reason) || self.state != Some(PresenceState::Off);
        self.off = Some(reason);
        self.failures = 0;
        self.dirty = false;
        let text = format!("off: {}", off_text(reason));
        let before = logs.len();
        self.enter(PresenceState::Off, None, &text, now, logs);
        if changed && logs.len() == before {
            logs.push(Log { warn: false, text });
        }
        self.due_ms = now.mono_ms + ms(self.config.interval);
    }

    fn next_while_on(&mut self, event: Event, now: Now, logs: &mut Vec<Log>) -> Action {
        match event {
            Event::Policy { off: None } | Event::SettingsChanged => {
                if self.state.is_none() {
                    self.attempt(None, &now)
                } else {
                    Action::Wait
                }
            }
            Event::Policy { off: Some(reason) } => match self.farewell(&now) {
                Some(farewell) => {
                    self.off = Some(reason);
                    self.goodbye_sent = true;
                    Action::Goodbye { published_at_ms: farewell.stamp(now.wall_ms) }
                }
                None => {
                    self.enter_off(reason, &now, logs);
                    Action::Wait
                }
            },
            Event::Goodbye { stored: true } => {
                self.failures = 0;
                self.enter(PresenceState::SignedOff, None, goodbye_text(true), &now, logs);
                self.due_ms = now.mono_ms + ms(self.config.signed_out_recheck);
                Action::Wait
            }
            Event::Goodbye { stored: false } => {
                self.failures = 0;
                self.attempt(None, &now)
            }
            Event::Start => self.attempt(None, &now),
            Event::PublishNow | Event::SignIn => {
                self.failures = 0;
                self.attempt(None, &now)
            }
            Event::Due => {
                let health_first = self.state == Some(PresenceState::Unsupported) && now.mono_ms < self.due_ms;
                if health_first {
                    self.health_due_ms = None;
                    Action::CheckHealth
                } else {
                    self.attempt(None, &now)
                }
            }
            Event::Changed => {
                if self.follows_changes() {
                    self.attempt(None, &now)
                } else {
                    self.dirty = true;
                    Action::Wait
                }
            }
            Event::NetworkChanged | Event::Woke => match self.state {
                Some(PresenceState::Publishing | PresenceState::Retrying | PresenceState::Unsupported) => {
                    self.attempt(None, &now)
                }
                _ => Action::Wait,
            },
            Event::NoSignIn => {
                self.failures = 0;
                // After a goodbye, "signed off" says more than "signed out".
                if self.state != Some(PresenceState::SignedOff) {
                    let text = "not signed in to MuxBus, nothing is published";
                    self.enter(PresenceState::SignedOut, None, text, &now, logs);
                }
                self.due_ms = now.mono_ms + ms(self.config.signed_out_recheck);
                Action::Wait
            }
            Event::Health { version, relay_date_ms } => {
                self.observe_date(relay_date_ms, &now, logs);
                self.on_health(version, &now, logs)
            }
            Event::Answer { sent_v, answer, relay_date_ms } => {
                self.observe_date(relay_date_ms, &now, logs);
                self.on_answer(sent_v, answer, &now, logs)
            }
        }
    }

    /// Send the latest snapshot. `v` is set for a second try of the same
    /// record; a fresh try forgets the second tries spent.
    fn attempt(&mut self, v: Option<u32>, now: &Now) -> Action {
        if v.is_none() {
            self.followups = Followups::default();
        }
        self.dirty = false;
        Action::Attempt { v: v.unwrap_or_else(|| self.record_version(now)) }
    }

    fn on_answer(&mut self, sent_v: u32, answer: Answer, now: &Now, logs: &mut Vec<Log>) -> Action {
        match answer {
            Answer::Stored => {
                if sent_v == PRESENCE_VERSION_V1 && self.followups.v1 {
                    self.v1_until_ms.insert(self.relay.clone(), now.mono_ms + ms(self.config.v1_memory));
                }
                self.failures = 0;
                self.last_ok_wall_ms = Some(now.wall_ms);
                let text = if sent_v == PRESENCE_VERSION_V1 {
                    "published, as v1: the relay is on an older version and gets no agent states"
                } else {
                    "published"
                };
                self.enter(PresenceState::Publishing, None, text, now, logs);
                self.due_ms = now.mono_ms + ms(self.config.interval);
                Action::Wait
            }
            Answer::Malformed if sent_v != PRESENCE_VERSION_V1 && !self.followups.v1 => {
                self.followups.v1 = true;
                self.attempt(Some(PRESENCE_VERSION_V1), now)
            }
            Answer::ClockRefused(_) if !self.followups.clock => {
                self.followups.clock = true;
                self.attempt(Some(sent_v), now)
            }
            Answer::Malformed => self.reject(MALFORMED_RECORD.to_string(), now, logs),
            Answer::ClockRefused(why) | Answer::Refused(why) => self.reject(why, now, logs),
            Answer::NoRoute => {
                self.enter(
                    PresenceState::Unsupported,
                    Some("the relay has no presence route yet".into()),
                    "the relay has no presence route yet, checking again within minutes",
                    now,
                    logs,
                );
                self.due_ms = now.mono_ms + scaled(self.config.unsupported_retry, 0.8 + 0.4 * now.rand);
                // A version to compare later ones with, read now.
                self.version_at_404 = None;
                self.health_due_ms = None;
                Action::CheckHealth
            }
            Answer::TooMany { retry_after } => {
                self.failures = self.failures.saturating_add(1);
                let wait = match retry_after {
                    Some(d) => ms(d.min(self.config.retry_after_cap)).max(ms(self.config.backoff_floor)),
                    None => self.backoff(now),
                };
                let reason = "too many requests".to_string();
                let text = format!("retrying: {reason}");
                self.enter(PresenceState::Retrying, Some(reason), &text, now, logs);
                self.due_ms = now.mono_ms + wait;
                Action::Wait
            }
            Answer::Unavailable { reason, detail } => {
                self.failures = self.failures.saturating_add(1);
                let wait = self.backoff(now);
                let text = format!("retrying: {reason} ({detail})");
                self.enter(PresenceState::Retrying, Some(reason), &text, now, logs);
                self.due_ms = now.mono_ms + wait;
                Action::Wait
            }
        }
    }

    fn reject(&mut self, why: String, now: &Now, logs: &mut Vec<Log>) -> Action {
        let text = format!("the relay refused the record: {why}");
        self.enter(PresenceState::Rejected, Some(why), &text, now, logs);
        self.due_ms = now.mono_ms + scaled(self.config.rejected_retry, 1.0 + 0.2 * now.rand);
        Action::Wait
    }

    fn on_health(&mut self, version: Option<String>, now: &Now, logs: &mut Vec<Log>) -> Action {
        if let Some(v) = &version {
            self.relay_version = Some(v.clone());
        }
        if self.state != Some(PresenceState::Unsupported) {
            return Action::Wait;
        }
        match (version, &self.version_at_404) {
            (Some(v), Some(before)) if v != *before => {
                logs.push(Log { warn: false, text: format!("the relay moved from {before} to {v}, trying again") });
                self.attempt(None, now)
            }
            (Some(v), None) => {
                self.version_at_404 = Some(v);
                self.health_due_ms = Some(now.mono_ms + ms(self.config.health_every));
                Action::Wait
            }
            _ => {
                self.health_due_ms = Some(now.mono_ms + ms(self.config.health_every));
                Action::Wait
            }
        }
    }

    /// Full jitter: anywhere up to `base * 2^(failures - 1)`, capped, but no
    /// less than the floor.
    fn backoff(&self, now: &Now) -> u64 {
        let doublings = self.failures.saturating_sub(1).min(20);
        let ceiling = ms(self.config.backoff_base).saturating_mul(1 << doublings).min(ms(self.config.backoff_cap));
        ((ceiling as f64 * now.rand) as u64).max(ms(self.config.backoff_floor))
    }

    /// Learn the relay's clock from an answer's `Date`.
    fn observe_date(&mut self, relay_date_ms: Option<u64>, now: &Now, logs: &mut Vec<Log>) {
        let Some(date) = relay_date_ms else { return };
        let skewed_before = self.clock_skewed();
        self.offset_ms = Some(date as i64 - now.wall_ms as i64);
        if self.clock_skewed() && !skewed_before {
            let off = self.offset_ms.unwrap_or(0).unsigned_abs();
            logs.push(Log {
                warn: false,
                text: format!("this computer's clock differs from the relay's by {} min; records carry the relay's time", minutes(off)),
            });
        }
    }

    fn clock_skewed(&self) -> bool {
        self.offset_ms.is_some_and(|o| o.unsigned_abs() > CLOCK_NOTE_AFTER_MS)
    }

    /// Enter `state` for `reason`; one log line when the state changes.
    fn enter(&mut self, state: PresenceState, reason: Option<String>, text: &str, now: &Now, logs: &mut Vec<Log>) {
        let failing = matches!(state, PresenceState::Retrying | PresenceState::Unsupported | PresenceState::Rejected);
        if failing {
            self.outage_since_ms.get_or_insert(now.mono_ms);
        } else {
            self.outage_since_ms = None;
            self.outage_warned = false;
        }
        if state != PresenceState::Unsupported {
            self.health_due_ms = None;
        }
        self.reason = reason;
        if self.state != Some(state) {
            self.state = Some(state);
            self.since_wall_ms = now.wall_ms;
            logs.push(Log { warn: false, text: text.to_string() });
        }
    }

    fn warn_long_outage(&mut self, now: &Now, logs: &mut Vec<Log>) {
        let Some(since) = self.outage_since_ms else { return };
        let lasted = now.mono_ms.saturating_sub(since);
        if self.outage_warned || lasted < ms(self.config.outage_warn_after) {
            return;
        }
        self.outage_warned = true;
        let why = self.reason.as_deref().unwrap_or("unknown");
        logs.push(Log { warn: true, text: format!("not published for {} min: {why}", minutes(lasted)) });
    }

    /// The status for `presence.status`; `None` before the first state.
    pub(crate) fn status(&self, now: &Now) -> Option<PresenceStatusResult> {
        let state = self.state?;
        let waits_for_nothing =
            matches!(state, PresenceState::SignedOut | PresenceState::SignedOff | PresenceState::Off);
        let next_try_ms = (!waits_for_nothing).then(|| now.wall_ms + self.due_ms.saturating_sub(now.mono_ms));
        let mut notes = Vec::new();
        if self.clock_skewed() {
            let off = self.offset_ms.unwrap_or(0).unsigned_abs();
            notes.push(format!("Your clock differs from the cloud's by {} min", minutes(off)));
        }
        let record_version = self.record_version(now);
        if record_version == PRESENCE_VERSION_V1 {
            notes.push("The cloud runs an older version, so your devices don't see agent states".to_string());
        }
        Some(PresenceStatusResult {
            state,
            since_ms: self.since_wall_ms,
            last_ok_ms: self.last_ok_wall_ms,
            last_error: self.reason.clone(),
            next_try_ms,
            relay_version: self.relay_version.clone(),
            offset_ms: self.offset_ms,
            record_version,
            note: (!notes.is_empty()).then(|| notes.join(". ")),
            off_reason: if state == PresenceState::Off { self.off } else { None },
        })
    }
}

fn goodbye_text(stored: bool) -> &'static str {
    if stored {
        "said goodbye: devices show this computer as offline"
    } else {
        "the goodbye didn't reach the relay; devices notice in a few minutes"
    }
}

fn off_text(reason: PresenceOffReason) -> &'static str {
    match reason {
        PresenceOffReason::Setting => "turned off in Settings",
        PresenceOffReason::DevBuild => "a dev build doesn't publish",
        PresenceOffReason::Headless => "a headless install doesn't publish",
        PresenceOffReason::IsolatedHome => "an isolated home doesn't publish",
        PresenceOffReason::TestHarness => "an install under test doesn't publish",
    }
}

/// What the relay's status and `error` mean for the publisher.
pub(crate) fn classify(status: u16, error: Option<&str>, retry_after: Option<Duration>) -> Answer {
    let said = || error.unwrap_or_default().to_string();
    match status {
        200..=299 => Answer::Stored,
        // Newest wins on the relay: it has a newer record of this install's.
        409 => Answer::Stored,
        404 => Answer::NoRoute,
        400 if error == Some(MALFORMED_RECORD) => Answer::Malformed,
        // The relay names the field when it refuses the record's time.
        400 if error.is_some_and(|e| e.contains("published_at_ms")) => Answer::ClockRefused(said()),
        400 | 410 => Answer::Refused(error.map_or_else(|| format!("the relay answered {status}"), str::to_string)),
        429 => Answer::TooMany { retry_after },
        _ => Answer::Unavailable { reason: format!("the cloud answered {status}"), detail: said() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1000;
    const MIN: u64 = 60 * S;

    /// A machine at mono 0, wall `WALL`, that has just stored a record.
    const WALL: u64 = 1_791_000_000_000;

    fn at(mono_ms: u64) -> Now {
        Now { mono_ms, wall_ms: WALL + mono_ms, rand: 0.5 }
    }

    fn at_rand(mono_ms: u64, rand: f64) -> Now {
        Now { rand, ..at(mono_ms) }
    }

    fn answer(answer: Answer) -> Event {
        Event::Answer { sent_v: 2, answer, relay_date_ms: None }
    }

    fn unreachable() -> Event {
        answer(Answer::Unavailable { reason: "cloud unreachable".into(), detail: "connection refused".into() })
    }

    fn machine() -> Machine {
        Machine::new(LIVE, "https://relay.test")
    }

    fn publishing() -> Machine {
        let mut m = machine();
        assert_eq!(m.next(Event::Start, at(0)).action, Action::Attempt { v: 2 });
        m.next(answer(Answer::Stored), at(0));
        m
    }

    fn secs(d: Duration) -> u64 {
        d.as_secs()
    }

    #[test]
    fn a_stored_record_publishes_again_in_a_minute() {
        let mut m = machine();
        let step = m.next(answer(Answer::Stored), at(0));
        assert_eq!(m.state(), Some(PresenceState::Publishing));
        assert_eq!((step.action, step.wait), (Action::Wait, Duration::from_secs(60)));
        assert_eq!(step.logs, vec![Log { warn: false, text: "published".into() }]);
        assert_eq!(m.next(Event::Due, at(60 * S)).action, Action::Attempt { v: 2 });
    }

    #[test]
    fn while_publishing_a_change_is_sent_after_the_drivers_debounce() {
        let mut m = publishing();
        assert!(m.follows_changes());
        assert_eq!(m.next(Event::Changed, at(5 * S)).action, Action::Attempt { v: 2 });
    }

    #[test]
    fn no_sign_in_sends_nothing_and_rechecks_locally() {
        let mut m = machine();
        let step = m.next(Event::NoSignIn, at(0));
        assert_eq!(m.state(), Some(PresenceState::SignedOut));
        assert_eq!((step.action, step.wait), (Action::Wait, Duration::from_secs(60)));
        assert_eq!(m.status(&at(0)).unwrap().next_try_ms, None, "no try is promised while signed out");
        assert_eq!(m.next(Event::Changed, at(S)).action, Action::Wait, "a change while signed out sends nothing");
        assert_eq!(m.next(Event::Woke, at(2 * S)).action, Action::Wait);
        assert_eq!(m.next(Event::SignIn, at(3 * S)).action, Action::Attempt { v: 2 }, "a sign-in tries at once");
    }

    #[test]
    fn failures_back_off_exponentially_to_five_minutes() {
        let mut m = machine();
        // rand just under 1: the ceiling itself.
        let ceilings: Vec<u64> = (0..9).map(|i| secs(m.next(unreachable(), at_rand(i * S, 0.999_999)).wait)).collect();
        assert_eq!(ceilings, vec![4, 9, 19, 39, 79, 159, 299, 299, 299], "5 s, 10 s, 20 s ... capped at 300 s, less the rounding");
        assert_eq!(m.state(), Some(PresenceState::Retrying));
    }

    #[test]
    fn backoff_is_full_jitter_with_a_floor() {
        for (rand, expect_ms) in [(0.0, 1000), (0.1, 1000), (0.5, 2500), (0.9, 4500)] {
            let mut m = machine();
            assert_eq!(m.next(unreachable(), at_rand(0, rand)).wait, Duration::from_millis(expect_ms), "rand {rand}");
        }
        // The fourth failure: every draw inside [floor, 40 s].
        for i in 0..100 {
            let mut probe = Machine { failures: 3, ..machine() };
            let wait = probe.next(unreachable(), at_rand(0, i as f64 / 100.0)).wait;
            assert!(wait >= Duration::from_secs(1) && wait <= Duration::from_secs(40), "{wait:?}");
        }
    }

    #[test]
    fn a_success_resets_the_backoff() {
        let mut m = machine();
        for i in 0..5 {
            m.next(unreachable(), at(i));
        }
        m.next(answer(Answer::Stored), at(10 * S));
        let wait = m.next(unreachable(), at_rand(20 * S, 0.999_999)).wait;
        assert_eq!(secs(wait), 4, "back to the first step");
    }

    #[test]
    fn a_429_honours_retry_after_and_otherwise_backs_off() {
        let mut m = machine();
        let step = m.next(answer(Answer::TooMany { retry_after: Some(Duration::from_secs(90)) }), at(0));
        assert_eq!(m.state(), Some(PresenceState::Retrying));
        assert_eq!(step.wait, Duration::from_secs(90), "Retry-After past the backoff ceiling is still honoured");
        let step = m.next(answer(Answer::TooMany { retry_after: None }), at_rand(0, 0.999_999));
        assert_eq!(secs(step.wait), 9, "the second failure's backoff");
        let step = m.next(answer(Answer::TooMany { retry_after: Some(Duration::from_secs(86_400)) }), at(0));
        assert_eq!(step.wait, Duration::from_secs(3600), "a day is capped");
        assert_eq!(m.status(&at(0)).unwrap().last_error.as_deref(), Some("too many requests"));
    }

    #[test]
    fn a_404_waits_about_five_minutes_not_an_hour() {
        for rand in [0.0, 0.5, 0.999_999] {
            let mut m = machine();
            let step = m.next(answer(Answer::NoRoute), at_rand(0, rand));
            assert_eq!(m.state(), Some(PresenceState::Unsupported));
            assert_eq!(step.action, Action::CheckHealth, "the relay's version is read at once");
            let step = m.next(Event::Health { version: Some("1.0.0".into()), relay_date_ms: None }, at_rand(0, rand));
            assert_eq!(step.wait, Duration::from_secs(60), "the next version check comes first");
            // Version checks every minute until the retry is due.
            let mut t = 0;
            loop {
                t += 60 * S;
                let step = m.next(Event::Due, at(t));
                if step.action != Action::CheckHealth {
                    assert_eq!(step.action, Action::Attempt { v: 2 });
                    break;
                }
                m.next(Event::Health { version: Some("1.0.0".into()), relay_date_ms: None }, at(t));
            }
        }
        // The retry itself lands in [4 min, 6 min].
        for rand in [0.0, 0.25, 0.5, 0.75, 0.999_999] {
            let mut m = machine();
            m.next(answer(Answer::NoRoute), at_rand(0, rand));
            let retry_ms = m.status(&at(0)).unwrap().next_try_ms.unwrap() - WALL;
            assert!((4 * MIN..=6 * MIN).contains(&retry_ms), "rand {rand}: {retry_ms}");
        }
    }

    #[test]
    fn a_404_is_cut_short_by_a_sign_in_a_network_change_a_wake_or_publish_now() {
        for event in [Event::SignIn, Event::NetworkChanged, Event::Woke, Event::PublishNow] {
            let mut m = machine();
            m.next(answer(Answer::NoRoute), at(0));
            m.next(Event::Health { version: Some("1.0.0".into()), relay_date_ms: None }, at(0));
            assert_eq!(m.next(event.clone(), at(5 * S)).action, Action::Attempt { v: 2 }, "{event:?}");
        }
    }

    #[test]
    fn a_404_is_cut_short_by_a_new_relay_version() {
        let mut m = machine();
        m.next(answer(Answer::NoRoute), at(0));
        m.next(Event::Health { version: Some("1.0.0".into()), relay_date_ms: None }, at(0));
        assert_eq!(m.next(Event::Due, at(60 * S)).action, Action::CheckHealth);
        let step = m.next(Event::Health { version: Some("1.0.0".into()), relay_date_ms: None }, at(60 * S));
        assert_eq!(step.action, Action::Wait, "the same version: keep waiting");
        assert_eq!(m.next(Event::Due, at(120 * S)).action, Action::CheckHealth);
        let step = m.next(Event::Health { version: None, relay_date_ms: None }, at(120 * S));
        assert_eq!(step.action, Action::Wait, "a failed check changes nothing");
        assert_eq!(m.next(Event::Due, at(180 * S)).action, Action::CheckHealth);
        let step = m.next(Event::Health { version: Some("1.1.0".into()), relay_date_ms: None }, at(180 * S));
        assert_eq!(step.action, Action::Attempt { v: 2 });
        assert_eq!(m.status(&at(180 * S)).unwrap().relay_version.as_deref(), Some("1.1.0"));
        // Still 404 on the new version: that version is the new baseline.
        assert_eq!(m.next(answer(Answer::NoRoute), at(181 * S)).action, Action::CheckHealth);
        let step = m.next(Event::Health { version: Some("1.1.0".into()), relay_date_ms: None }, at(181 * S));
        assert_eq!(step.action, Action::Wait);
    }

    #[test]
    fn version_checks_come_at_most_once_a_minute() {
        let mut m = machine();
        m.next(answer(Answer::NoRoute), at(0));
        m.next(Event::Health { version: Some("1".into()), relay_date_ms: None }, at(0));
        // A change in between brings the next check no closer.
        assert_eq!(m.next(Event::Changed, at(10 * S)).wait, Duration::from_secs(50));
    }

    #[test]
    fn a_change_while_failing_only_marks_the_record_dirty() {
        for setup in [unreachable(), answer(Answer::NoRoute), answer(Answer::Refused("nope".into()))] {
            let mut m = machine();
            m.next(setup.clone(), at(0));
            let before = m.next(Event::Changed, at(S));
            assert_eq!(before.action, Action::Wait, "{setup:?}");
            assert!(m.dirty());
            let again = m.next(Event::Changed, at(2 * S));
            assert_eq!(again.wait + Duration::from_secs(1), before.wait, "the wait runs on, not restarted");
            assert!(!m.follows_changes());
        }
        let mut m = machine();
        m.next(unreachable(), at(0));
        m.next(Event::Changed, at(100));
        let step = m.next(Event::Due, at(10 * S));
        assert_eq!(step.action, Action::Attempt { v: 2 }, "the next try sends the latest");
        assert!(!m.dirty());
    }

    #[test]
    fn a_refusal_is_retried_at_most_every_ten_minutes() {
        for rand in [0.0, 0.5, 0.999_999] {
            let mut m = machine();
            let step = m.next(answer(Answer::Refused("not signed by its instance".into())), at_rand(0, rand));
            assert_eq!(m.state(), Some(PresenceState::Rejected));
            assert!(step.wait >= Duration::from_secs(600) && step.wait <= Duration::from_secs(720), "{:?}", step.wait);
            assert_eq!(m.next(Event::Woke, at(S)).action, Action::Wait, "a wake doesn't fix a refusal");
            assert_eq!(m.next(Event::NetworkChanged, at(S)).action, Action::Wait);
            assert_eq!(m.status(&at(S)).unwrap().last_error.as_deref(), Some("not signed by its instance"));
        }
        let mut m = machine();
        m.next(answer(Answer::Refused("x".into())), at(0));
        assert_eq!(m.next(Event::PublishNow, at(S)).action, Action::Attempt { v: 2 }, "but the user may ask");
    }

    #[test]
    fn a_malformed_v2_is_sent_again_as_v1_once() {
        let mut m = publishing();
        assert_eq!(m.next(Event::Due, at(60 * S)).action, Action::Attempt { v: 2 });
        let step = m.next(answer(Answer::Malformed), at(60 * S));
        assert_eq!(step.action, Action::Attempt { v: 1 });
        assert_eq!(m.state(), Some(PresenceState::Publishing), "no state change for the second try");
        let step = m.next(Event::Answer { sent_v: 1, answer: Answer::Malformed, relay_date_ms: None }, at(60 * S));
        assert_eq!(step.action, Action::Wait, "v1 failing too: no third try");
        assert_eq!(m.state(), Some(PresenceState::Rejected));
        assert_eq!(m.status(&at(60 * S)).unwrap().record_version, 2, "nothing remembered");
    }

    #[test]
    fn a_relay_that_took_v1_gets_v1_for_six_hours() {
        let mut m = machine();
        m.next(Event::Start, at(0));
        assert_eq!(m.next(answer(Answer::Malformed), at(0)).action, Action::Attempt { v: 1 });
        let step = m.next(Event::Answer { sent_v: 1, answer: Answer::Stored, relay_date_ms: None }, at(0));
        assert_eq!(m.state(), Some(PresenceState::Publishing));
        assert!(step.logs[0].text.contains("as v1"), "{:?}", step.logs);
        let status = m.status(&at(0)).unwrap();
        assert_eq!(status.record_version, 1);
        assert!(status.note.as_deref().unwrap().contains("older version"), "{status:?}");

        assert_eq!(m.next(Event::Due, at(60 * S)).action, Action::Attempt { v: 1 });
        m.next(Event::Answer { sent_v: 1, answer: Answer::Stored, relay_date_ms: None }, at(60 * S));
        // Per relay: another one gets v2.
        m.set_relay("https://other.test/");
        assert_eq!(m.next(Event::Due, at(120 * S)).action, Action::Attempt { v: 2 });
        m.set_relay("https://relay.test");
        assert_eq!(m.next(Event::Due, at(6 * 60 * MIN - 1)).action, Action::Attempt { v: 1 });
        assert_eq!(m.next(Event::Due, at(6 * 60 * MIN)).action, Action::Attempt { v: 2 }, "forgotten after 6 h");
        assert_eq!(m.status(&at(6 * 60 * MIN)).unwrap().note, None);
    }

    #[test]
    fn the_v1_memory_is_only_set_by_a_fallback_that_worked() {
        let mut m = machine();
        m.next(Event::Start, at(0));
        m.next(answer(Answer::Malformed), at(0));
        m.next(Event::Answer { sent_v: 1, answer: unreachable_answer(), relay_date_ms: None }, at(0));
        assert_eq!(m.state(), Some(PresenceState::Retrying));
        assert_eq!(m.next(Event::Due, at(MIN)).action, Action::Attempt { v: 2 }, "v2 again: v1 never landed");
    }

    fn unreachable_answer() -> Answer {
        Answer::Unavailable { reason: "cloud unreachable".into(), detail: String::new() }
    }

    #[test]
    fn records_are_stamped_with_the_relays_clock() {
        let fast = 15 * MIN; // this computer's clock is 15 min fast
        for (skew_label, local_minus_relay) in [("fast", fast as i64), ("slow", -(fast as i64))] {
            let mut m = machine();
            let now = at(0);
            assert_eq!(m.stamp(now.wall_ms), now.wall_ms, "no answer yet: the local clock");
            let relay_date = (now.wall_ms as i64 - local_minus_relay) as u64;
            let step = m.next(Event::Answer { sent_v: 2, answer: Answer::Stored, relay_date_ms: Some(relay_date) }, now);
            assert!(step.logs.iter().any(|l| l.text.contains("differs from the relay's by 15 min")), "{skew_label}");
            assert_eq!(m.stamp(now.wall_ms), relay_date, "{skew_label}");
            assert_eq!(m.stamp(now.wall_ms + 5_000), relay_date + 5_000);
            let status = m.status(&now).unwrap();
            assert_eq!(status.offset_ms, Some(-local_minus_relay));
            assert_eq!(status.note.as_deref(), Some("Your clock differs from the cloud's by 15 min"), "{skew_label}");
        }
    }

    #[test]
    fn a_small_offset_is_used_but_not_shown() {
        let mut m = machine();
        let now = at(0);
        m.next(Event::Answer { sent_v: 2, answer: Answer::Stored, relay_date_ms: Some(now.wall_ms + 90_000) }, now);
        assert_eq!(m.stamp(now.wall_ms), now.wall_ms + 90_000);
        assert_eq!(m.status(&now).unwrap().note, None);
    }

    #[test]
    fn an_answer_without_a_date_keeps_the_last_offset() {
        let mut m = machine();
        let now = at(0);
        m.next(Event::Health { version: None, relay_date_ms: Some(now.wall_ms - 1000) }, now);
        m.next(answer(Answer::Stored), now);
        assert_eq!(m.status(&now).unwrap().offset_ms, Some(-1000));
    }

    #[test]
    fn a_clock_refusal_is_sent_again_at_once_with_the_corrected_time() {
        let mut m = machine();
        m.next(Event::Start, at(0));
        let relay_date = WALL - 15 * MIN;
        let step = m.next(
            Event::Answer {
                sent_v: 2,
                answer: Answer::ClockRefused("published_at_ms is too far from the relay's clock".into()),
                relay_date_ms: Some(relay_date),
            },
            at(0),
        );
        assert_eq!(step.action, Action::Attempt { v: 2 });
        assert_eq!(m.stamp(WALL), relay_date);
        let step = m.next(
            answer(Answer::ClockRefused("published_at_ms is too far from the relay's clock".into())),
            at(0),
        );
        assert_eq!(step.action, Action::Wait, "once only");
        assert_eq!(m.state(), Some(PresenceState::Rejected));
    }

    #[test]
    fn a_long_outage_warns_once_and_each_state_change_logs_once() {
        let mut m = publishing();
        let first = m.next(unreachable(), at(MIN));
        assert_eq!(first.logs.len(), 1);
        assert!(first.logs[0].text.starts_with("retrying: cloud unreachable"), "{:?}", first.logs);
        assert!(!first.logs[0].warn);
        let mut warnings = 0;
        for i in 2..30u64 {
            let step = m.next(unreachable(), at(i * MIN));
            assert!(step.logs.iter().all(|l| l.warn), "no line per retry: {:?}", step.logs);
            warnings += step.logs.len();
            if i == 11 {
                assert_eq!(warnings, 1, "warned once 10 min in");
                assert_eq!(step.logs[0].text, "not published for 10 min: cloud unreachable");
            }
        }
        assert_eq!(warnings, 1);
        // A success ends the outage; the next one warns again.
        assert_eq!(m.next(answer(Answer::Stored), at(30 * MIN)).logs.len(), 1);
        m.next(unreachable(), at(31 * MIN));
        assert!(m.next(unreachable(), at(41 * MIN)).logs.iter().any(|l| l.warn));
    }

    #[test]
    fn the_status_says_when_and_why() {
        let mut m = machine();
        assert_eq!(m.status(&at(0)), None, "nothing to say before the first try");
        m.next(answer(Answer::Stored), at(0));
        m.next(unreachable(), at_rand(30 * S, 0.5));
        let status = m.status(&at(31 * S)).unwrap();
        assert_eq!(
            status,
            PresenceStatusResult {
                state: PresenceState::Retrying,
                since_ms: WALL + 30 * S,
                last_ok_ms: Some(WALL),
                last_error: Some("cloud unreachable".into()),
                next_try_ms: Some(WALL + 30 * S + 2500),
                relay_version: None,
                offset_ms: None,
                record_version: 2,
                note: None,
                off_reason: None,
            }
        );
    }

    #[test]
    fn statuses_and_errors_are_classified() {
        assert_eq!(classify(200, None, None), Answer::Stored);
        assert_eq!(classify(204, None, None), Answer::Stored);
        assert_eq!(classify(409, Some("a newer record is stored"), None), Answer::Stored, "newest wins: as good as stored");
        assert_eq!(classify(404, Some("Not Found"), None), Answer::NoRoute);
        assert_eq!(classify(400, Some(MALFORMED_RECORD), None), Answer::Malformed);
        assert_eq!(
            classify(400, Some("published_at_ms is too far from the relay's clock"), None),
            Answer::ClockRefused("published_at_ms is too far from the relay's clock".into())
        );
        assert_eq!(classify(400, Some("bad sig"), None), Answer::Refused("bad sig".into()));
        assert_eq!(classify(410, None, None), Answer::Refused("the relay answered 410".into()));
        let ra = Some(Duration::from_secs(7));
        assert_eq!(classify(429, None, ra), Answer::TooMany { retry_after: ra });
        assert_eq!(
            classify(503, Some("down"), None),
            Answer::Unavailable { reason: "the cloud answered 503".into(), detail: "down".into() }
        );
        assert!(matches!(classify(401, None, None), Answer::Unavailable { .. }));
    }
}
