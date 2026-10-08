// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pure half of [`super`]: what state cloud delivery is in, given what
//! the cloud subscriber and the credential broker last said, and what to do
//! when it changes. No clock, no I/O; every call takes `now_ms`.

use crate::backend::rpc_types::{MuxBusDeliveryState, MuxBusDeliveryStatus};
use crate::broker::CredentialState;

/// How long cloud delivery must have been paused before the agents are told:
/// a reconnect or a token refresh that recovers within this never reaches
/// them. The status bar's amber "reconnecting" waits the same time.
pub const PAUSE_NOTICE_MS: i64 = 2 * 60 * 1000;

/// What the cloud subscriber's loop is doing, as it last reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Nothing stored to sign in with: the loop waits for a sign-in.
    NoSignIn,
    /// A sign-in is stored but can't work anymore (the reason).
    SignInStale(String),
    /// Signed in, but the relay isn't reachable or the token can't be
    /// refreshed for now (the reason). The loop retries on its own.
    Unreachable(String),
    /// The WebSocket to the relay is open.
    Connected,
}

/// Something [`Machine`] decided the runtime should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// The status changed: tell the UI.
    Changed(MuxBusDeliveryStatus),
    /// A needs-sign-in episode began: one OS notification.
    NotifySignInNeeded,
    /// That episode ended: take the notification down if it's still up.
    RetractSignInNeeded,
    /// Cloud delivery has been paused for [`PAUSE_NOTICE_MS`]: tell the
    /// agents, since `since_ms`, because of `reason`.
    PauseNote { since_ms: i64, reason: &'static str },
    /// Delivery came back after a pause the agents were told about.
    Resumed { at_ms: i64 },
}

/// The state a set of inputs comes to, and why it isn't connected.
pub fn derive(
    session_known: bool,
    link: &Link,
    broker: Option<&CredentialState>,
) -> (MuxBusDeliveryState, Option<String>) {
    match link {
        Link::Connected => (MuxBusDeliveryState::Connected, None),
        _ if !session_known => (MuxBusDeliveryState::SignedOut, None),
        Link::SignInStale(reason) => (MuxBusDeliveryState::NeedsSignIn, Some(reason.clone())),
        // Signed in before, and now nothing readable is stored: the sign-in
        // was lost, not given up (a deliberate sign-out clears `session_known`).
        Link::NoSignIn => (
            MuxBusDeliveryState::NeedsSignIn,
            Some("the stored sign-in can't be read".to_string()),
        ),
        Link::Unreachable(error) => match broker {
            Some(CredentialState::NeedsReauth { rejected: true, reason, .. }) => {
                (MuxBusDeliveryState::NeedsSignIn, Some(format!("the sign-in was refused: {reason}")))
            }
            _ => (MuxBusDeliveryState::Reconnecting, Some(error.clone())),
        },
    }
}

/// Why the agents' pause note says messages are paused.
pub fn pause_reason(state: MuxBusDeliveryState) -> &'static str {
    match state {
        MuxBusDeliveryState::NeedsSignIn => "MuxBus needs a sign-in",
        _ => "AgentMux can't reach MuxBus",
    }
}

/// Cloud delivery's state over time, and the episodes that drive the
/// notification and the agents' notes.
#[derive(Debug, Clone)]
pub struct Machine {
    session_known: bool,
    email: Option<String>,
    link: Link,
    status: MuxBusDeliveryStatus,
    /// When the current pause began; open until delivery is connected again.
    /// A sign-out neither starts nor ends one.
    paused_since_ms: Option<i64>,
    /// The agents were told about the current pause.
    pause_noted: bool,
    /// The current needs-sign-in episode has had its notification.
    sign_in_notified: bool,
}

impl Machine {
    pub fn new(now_ms: i64) -> Self {
        Machine {
            session_known: false,
            email: None,
            link: Link::NoSignIn,
            status: MuxBusDeliveryStatus { since_ms: now_ms, ..Default::default() },
            paused_since_ms: None,
            pause_noted: false,
            sign_in_notified: false,
        }
    }

    pub fn status(&self) -> &MuxBusDeliveryStatus {
        &self.status
    }

    /// When the current pause began, if one is open.
    pub fn paused_since_ms(&self) -> Option<i64> {
        self.paused_since_ms
    }

    /// The subscriber found a sign-in to use (`email` when it could read one).
    pub fn saw_session(&mut self, email: Option<String>, broker: Option<&CredentialState>, now_ms: i64) -> Vec<Effect> {
        self.session_known = true;
        if email.as_deref().is_some_and(|e| !e.is_empty()) {
            self.email = email;
        }
        // There is a sign-in now, so the loop is on its way to connecting:
        // not a lost one (which would notify).
        if self.link == Link::NoSignIn {
            self.link = Link::Unreachable("connecting".to_string());
        }
        self.update(broker, now_ms)
    }

    /// The user signed out on purpose: not a lost sign-in.
    pub fn signed_out(&mut self, now_ms: i64) -> Vec<Effect> {
        self.session_known = false;
        self.email = None;
        self.link = Link::NoSignIn;
        self.update(None, now_ms)
    }

    pub fn set_link(&mut self, link: Link, broker: Option<&CredentialState>, now_ms: i64) -> Vec<Effect> {
        self.link = link;
        self.update(broker, now_ms)
    }

    /// The relay answered a pull: delivery is healthy as of `now_ms`.
    pub fn relay_answered(&mut self, now_ms: i64) {
        self.status.last_ok_ms = Some(now_ms);
    }

    /// Re-derive (the broker may have moved on its own) and release the
    /// pause note once it is due.
    pub fn tick(&mut self, broker: Option<&CredentialState>, now_ms: i64) -> Vec<Effect> {
        let mut out = self.update(broker, now_ms);
        if let Some(since_ms) = self.paused_since_ms {
            if !self.pause_noted && self.status.state.is_paused() && now_ms - since_ms >= PAUSE_NOTICE_MS {
                self.pause_noted = true;
                out.push(Effect::PauseNote { since_ms, reason: pause_reason(self.status.state) });
            }
        }
        out
    }

    fn update(&mut self, broker: Option<&CredentialState>, now_ms: i64) -> Vec<Effect> {
        let (state, error) = derive(self.session_known, &self.link, broker);
        let mut out = Vec::new();
        let prev = self.status.state;
        let changed = state != prev || error != self.status.last_error || self.email != self.status.account_email;
        if state == MuxBusDeliveryState::Connected || prev == MuxBusDeliveryState::Connected {
            self.status.last_ok_ms = Some(now_ms);
        }
        if state != prev {
            self.status.since_ms = now_ms;
            if state == MuxBusDeliveryState::NeedsSignIn && !self.sign_in_notified {
                self.sign_in_notified = true;
                out.push(Effect::NotifySignInNeeded);
            }
            if matches!(state, MuxBusDeliveryState::Connected | MuxBusDeliveryState::SignedOut) && self.sign_in_notified {
                self.sign_in_notified = false;
                out.push(Effect::RetractSignInNeeded);
            }
            // A sign-out ends a pause the agents were never told about, so a
            // later sign-in starts a new one rather than dating it from the
            // old outage. A pause they were told about stays open for its
            // resume note.
            if state == MuxBusDeliveryState::SignedOut && !self.pause_noted {
                self.paused_since_ms = None;
            }
            if state.is_paused() && self.paused_since_ms.is_none() {
                self.paused_since_ms = Some(now_ms);
            }
            if state == MuxBusDeliveryState::Connected {
                if self.paused_since_ms.take().is_some() && self.pause_noted {
                    out.push(Effect::Resumed { at_ms: now_ms });
                }
                self.pause_noted = false;
            }
        }
        self.status.state = state;
        self.status.last_error = error;
        self.status.account_email = self.email.clone();
        if changed {
            out.insert(0, Effect::Changed(self.status.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MuxBusDeliveryState::*;

    const T0: i64 = 1_791_000_000_000;

    fn refused() -> CredentialState {
        CredentialState::NeedsReauth { since_unix: 0, reason: "invalid_grant".into(), rejected: true }
    }

    fn piled_up() -> CredentialState {
        CredentialState::NeedsReauth { since_unix: 0, reason: "network".into(), rejected: false }
    }

    fn connected_machine() -> Machine {
        let mut m = Machine::new(T0);
        m.saw_session(Some("a@b.c".into()), None, T0);
        m.set_link(Link::Connected, None, T0);
        m
    }

    fn kinds(effects: &[Effect]) -> Vec<&'static str> {
        effects
            .iter()
            .map(|e| match e {
                Effect::Changed(_) => "changed",
                Effect::NotifySignInNeeded => "notify",
                Effect::RetractSignInNeeded => "retract",
                Effect::PauseNote { .. } => "pause",
                Effect::Resumed { .. } => "resumed",
            })
            .collect()
    }

    #[test]
    fn each_state_derives_from_its_inputs() {
        assert_eq!(derive(true, &Link::Connected, Some(&refused())).0, Connected, "an open link wins");
        assert_eq!(derive(false, &Link::NoSignIn, None).0, SignedOut);
        assert_eq!(derive(false, &Link::Unreachable("x".into()), Some(&refused())).0, SignedOut);
        assert_eq!(derive(true, &Link::NoSignIn, None).0, NeedsSignIn, "a sign-in that vanished");
        assert_eq!(derive(true, &Link::SignInStale("pool".into()), None), (NeedsSignIn, Some("pool".into())));
        assert_eq!(derive(true, &Link::Unreachable("timeout".into()), None), (Reconnecting, Some("timeout".into())));
        assert_eq!(derive(true, &Link::Unreachable("x".into()), Some(&piled_up())).0, Reconnecting);
        let (state, why) = derive(true, &Link::Unreachable("x".into()), Some(&refused()));
        assert_eq!(state, NeedsSignIn);
        assert!(why.unwrap().contains("invalid_grant"));
    }

    #[test]
    fn a_channel_never_signed_in_stays_signed_out_and_quiet() {
        let mut m = Machine::new(T0);
        let fx = m.set_link(Link::NoSignIn, Some(&refused()), T0 + 1);
        assert!(fx.is_empty(), "{fx:?}");
        assert_eq!(m.status().state, SignedOut);
        assert!(m.tick(Some(&refused()), T0 + 10 * PAUSE_NOTICE_MS).is_empty());
    }

    #[test]
    fn transitions_record_when_they_happened() {
        let mut m = connected_machine();
        assert_eq!(m.status().state, Connected);
        assert_eq!(m.status().account_email.as_deref(), Some("a@b.c"));
        m.set_link(Link::Unreachable("closed".into()), None, T0 + 5_000);
        assert_eq!((m.status().state, m.status().since_ms), (Reconnecting, T0 + 5_000));
        assert_eq!(m.status().last_ok_ms, Some(T0 + 5_000), "last ok is when it left connected");
        m.set_link(Link::SignInStale("refused".into()), None, T0 + 9_000);
        assert_eq!((m.status().state, m.status().since_ms), (NeedsSignIn, T0 + 9_000));
        m.set_link(Link::Connected, None, T0 + 20_000);
        assert_eq!((m.status().state, m.status().since_ms), (Connected, T0 + 20_000));
        let fx = m.signed_out(T0 + 30_000);
        assert_eq!(m.status().state, SignedOut);
        assert_eq!(m.status().account_email, None);
        assert_eq!(kinds(&fx), ["changed"]);
    }

    #[test]
    fn the_broker_alone_can_move_reconnecting_to_needs_sign_in() {
        let mut m = connected_machine();
        m.set_link(Link::Unreachable("closed".into()), None, T0 + 1);
        let fx = m.tick(Some(&refused()), T0 + 2);
        assert_eq!(m.status().state, NeedsSignIn);
        assert_eq!(kinds(&fx), ["changed", "notify"]);
    }

    #[test]
    fn one_notification_per_needs_sign_in_episode() {
        let mut m = connected_machine();
        let fx = m.set_link(Link::SignInStale("refused".into()), None, T0 + 1);
        assert_eq!(kinds(&fx), ["changed", "notify"]);
        // Flapping inside the episode does not notify again.
        m.set_link(Link::Unreachable("x".into()), None, T0 + 2);
        let fx = m.set_link(Link::SignInStale("refused".into()), None, T0 + 3);
        assert_eq!(kinds(&fx), ["changed"]);
        assert!(!kinds(&m.tick(None, T0 + 10 * PAUSE_NOTICE_MS)).contains(&"notify"));
        // Connected ends it and takes the notification down; the next one notifies.
        let fx = m.set_link(Link::Connected, None, T0 + 20 * PAUSE_NOTICE_MS);
        assert!(kinds(&fx).contains(&"retract"));
        let fx = m.set_link(Link::NoSignIn, None, T0 + 21 * PAUSE_NOTICE_MS);
        assert_eq!(kinds(&fx), ["changed", "notify"]);
    }

    #[test]
    fn signing_out_ends_the_sign_in_episode_without_another_notification() {
        let mut m = connected_machine();
        m.set_link(Link::SignInStale("refused".into()), None, T0 + 1);
        let fx = m.signed_out(T0 + 2);
        assert_eq!(kinds(&fx), ["changed", "retract"]);
    }

    #[test]
    fn the_pause_note_waits_two_minutes_and_goes_out_once() {
        let mut m = connected_machine();
        let start = T0 + 1_000;
        m.set_link(Link::Unreachable("closed".into()), None, start);
        assert!(!kinds(&m.tick(None, start + PAUSE_NOTICE_MS - 1)).contains(&"pause"));
        let fx = m.tick(None, start + PAUSE_NOTICE_MS);
        assert_eq!(fx, [Effect::PauseNote { since_ms: start, reason: "AgentMux can't reach MuxBus" }]);
        assert!(m.tick(None, start + 5 * PAUSE_NOTICE_MS).is_empty(), "once per pause");
        // The pause carries on into needs-sign-in: still the same pause, no second note.
        m.set_link(Link::SignInStale("refused".into()), None, start + 6 * PAUSE_NOTICE_MS);
        assert!(!kinds(&m.tick(None, start + 9 * PAUSE_NOTICE_MS)).contains(&"pause"));
        let fx = m.set_link(Link::Connected, None, start + 10 * PAUSE_NOTICE_MS);
        assert!(fx.contains(&Effect::Resumed { at_ms: start + 10 * PAUSE_NOTICE_MS }));
    }

    #[test]
    fn the_pause_reason_follows_the_state_at_note_time() {
        let mut m = connected_machine();
        m.set_link(Link::SignInStale("refused".into()), None, T0 + 1);
        let fx = m.tick(None, T0 + 1 + PAUSE_NOTICE_MS);
        assert!(fx.contains(&Effect::PauseNote { since_ms: T0 + 1, reason: "MuxBus needs a sign-in" }));
    }

    #[test]
    fn a_blip_under_two_minutes_tells_the_agents_nothing() {
        let mut m = connected_machine();
        m.set_link(Link::Unreachable("closed".into()), None, T0 + 1);
        m.tick(None, T0 + PAUSE_NOTICE_MS - 10);
        let fx = m.set_link(Link::Connected, None, T0 + PAUSE_NOTICE_MS - 5);
        assert_eq!(kinds(&fx), ["changed"], "no resume without a pause note");
        assert!(m.tick(None, T0 + 10 * PAUSE_NOTICE_MS).is_empty());
        assert_eq!(m.paused_since_ms(), None);
    }

    #[test]
    fn a_pause_survives_a_sign_out_and_resumes_on_the_next_connect() {
        let mut m = connected_machine();
        m.set_link(Link::SignInStale("refused".into()), None, T0);
        m.tick(None, T0 + PAUSE_NOTICE_MS);
        m.signed_out(T0 + PAUSE_NOTICE_MS + 1);
        assert!(m.tick(None, T0 + 4 * PAUSE_NOTICE_MS).is_empty(), "signed out: no further note");
        let fx = m.saw_session(None, None, T0 + 5 * PAUSE_NOTICE_MS);
        assert_eq!(m.status().state, Reconnecting, "signing in again is not a lost sign-in");
        assert!(!kinds(&fx).contains(&"notify"));
        let fx = m.set_link(Link::Connected, None, T0 + 5 * PAUSE_NOTICE_MS);
        assert!(kinds(&fx).contains(&"resumed"));
    }

    #[test]
    fn a_short_pause_ended_by_a_sign_out_is_not_dated_from_later() {
        let mut m = connected_machine();
        m.set_link(Link::Unreachable("closed".into()), None, T0);
        m.signed_out(T0 + PAUSE_NOTICE_MS / 2);
        assert_eq!(m.paused_since_ms(), None);
        let later = T0 + 100 * PAUSE_NOTICE_MS;
        m.saw_session(None, None, later);
        assert!(!kinds(&m.tick(None, later + 1)).contains(&"pause"), "no note dated from the old outage");
        let fx = m.tick(None, later + PAUSE_NOTICE_MS);
        assert!(fx.contains(&Effect::PauseNote { since_ms: later, reason: pause_reason(Reconnecting) }));
    }

    #[test]
    fn an_unchanged_report_changes_nothing() {
        let mut m = connected_machine();
        assert!(m.set_link(Link::Connected, None, T0 + 1).is_empty());
        assert!(m.saw_session(Some("a@b.c".into()), None, T0 + 2).is_empty());
    }
}
