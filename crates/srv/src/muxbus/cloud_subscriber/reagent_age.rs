// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How old a signed notice may be when it is delivered, and what happens to a copy of one already delivered.
//!
//! A notice is signed when it is sent and may then wait at the relay for a host that is off. Judging the
//! signature's age at delivery alone would read every notice held past [`REAGENT_SIG_MAX_AGE_SECS`] as forged.
//! The age is therefore judged at the moment the relay accepted the notice (its own record, `created_at`), with
//! a bound at delivery that needs no trust in the relay ([`SIG_MAX_HOLD_SECS`]). A signed message that was already
//! delivered to an agent is not delivered again under another delivery ([`delivered_under_another`]): a sender
//! that retried a call the relay had in fact taken signs once and sends twice, and a replay of a captured notice
//! looks the same. Either way nothing is shown, neither as the notice nor as a forgery.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::super::{reagent_sig_is_fresh, REAGENT_SIG_MAX_AGE_SECS};

/// The longest a signed notice may have been held, from signing to delivery, in seconds: the longest hold a sender
/// uses for a host that is off (6 hours today; a sender's hold must not exceed this) plus the 10 minutes a notice
/// may take to be accepted. A signature older than this is never a notice still inside its own hold.
pub(super) const SIG_MAX_HOLD_SECS: i64 = 6 * 60 * 60 + REAGENT_SIG_MAX_AGE_SECS;

/// How far the relay's clock and the signer's may differ.
const CLOCK_SKEW_SECS: i64 = 60;

/// Delivered signed messages remembered at most.
const MAX_SEEN: usize = 4096;

/// Is a signature made at `ts_secs` recent enough to trust at `now`? `accepted_secs` is when the relay says it
/// accepted the message. A signature fresh at delivery always is (the rule before the relay's time was used).
/// A signature that waited is trusted only when all of these hold: the relay gave a time; it is not in the future
/// and not before the signature was made (beyond clock skew); the relay accepted it within
/// [`REAGENT_SIG_MAX_AGE_SECS`] of signing; and the delivery is within [`SIG_MAX_HOLD_SECS`] of signing. Anything
/// the relay's record does not support falls back to the delivery-time rule alone, never to "accepted".
pub(super) fn age_ok(ts_secs: i64, accepted_secs: Option<i64>, now: i64) -> bool {
    if reagent_sig_is_fresh(ts_secs, now) {
        return true;
    }
    let Some(accepted) = accepted_secs else { return false };
    if ts_secs <= 0 || accepted > now + CLOCK_SKEW_SECS || accepted < ts_secs - CLOCK_SKEW_SECS {
        return false;
    }
    accepted - ts_secs <= REAGENT_SIG_MAX_AGE_SECS && now - ts_secs <= SIG_MAX_HOLD_SECS
}

/// The signed messages already delivered, by (signed message id, agent): the delivery each went out under and
/// when it is forgotten. The sender signs one id per recipient, and the agent is part of the key anyway, so two
/// recipients of one id never meet.
#[derive(Default)]
struct Delivered {
    ids: HashMap<(String, String), (String, i64)>,
}

impl Delivered {
    /// The delivery `msg_id` already went to `agent` as, when that is another delivery than `inj_id`. The same
    /// delivery again (a claim given back and taken again) is not a repeat.
    fn delivered_under_another(&self, msg_id: &str, agent: &str, inj_id: &str, now: i64) -> Option<String> {
        let (delivered_as, until) = self.ids.get(&(msg_id.to_string(), agent.to_string()))?;
        (*until > now && delivered_as != inj_id).then(|| delivered_as.clone())
    }

    /// Remember that `msg_id` went to `agent` as the delivery `inj_id`, for [`SIG_MAX_HOLD_SECS`], the longest a
    /// signature can still pass. Past `MAX_SEEN` entries the one due soonest is dropped.
    fn record(&mut self, msg_id: &str, agent: &str, inj_id: &str, now: i64) {
        if self.ids.len() >= MAX_SEEN {
            self.ids.retain(|_, (_, until)| *until > now);
        }
        let key = (msg_id.to_string(), agent.to_string());
        if !self.ids.contains_key(&key) && self.ids.len() >= MAX_SEEN {
            if let Some(oldest) = self.ids.iter().min_by_key(|(_, (_, until))| *until).map(|(k, _)| k.clone()) {
                self.ids.remove(&oldest);
            }
        }
        // The first delivery keeps its record: a later one never moves it.
        self.ids.entry(key).or_insert_with(|| (inj_id.to_string(), now + SIG_MAX_HOLD_SECS));
    }
}

static DELIVERED: LazyLock<Mutex<Delivered>> = LazyLock::new(Default::default);

/// The delivery the signed message `msg_id` already went to `agent` as, when it is another than `inj_id`.
pub(super) fn delivered_under_another(msg_id: &str, agent: &str, inj_id: &str, now: i64) -> Option<String> {
    DELIVERED.lock().unwrap_or_else(|e| e.into_inner()).delivered_under_another(msg_id, agent, inj_id, now)
}

/// The signed message `msg_id` has been delivered to `agent` as `inj_id`.
pub(super) fn record_delivered(msg_id: &str, agent: &str, inj_id: &str, now: i64) {
    DELIVERED.lock().unwrap_or_else(|e| e.into_inner()).record(msg_id, agent, inj_id, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: i64 = 1_000_000;

    #[test]
    fn a_signature_fresh_at_delivery_passes_whatever_the_relay_says() {
        assert!(age_ok(TS, None, TS));
        assert!(age_ok(TS, Some(0), TS + REAGENT_SIG_MAX_AGE_SECS));
        assert!(!age_ok(TS, None, TS + REAGENT_SIG_MAX_AGE_SECS + 1), "late, and nothing from the relay");
    }

    #[test]
    fn a_notice_signed_and_accepted_together_and_delivered_20_minutes_later_passes() {
        // The case behind the stop of 16:46: signed 16:46, accepted 16:46, delivered 17:06.
        assert!(age_ok(TS, Some(TS + 2), TS + 20 * 60));
        assert!(age_ok(TS, Some(TS), TS + SIG_MAX_HOLD_SECS), "up to the longest hold");
        assert!(!age_ok(TS, Some(TS), TS + SIG_MAX_HOLD_SECS + 1), "past the longest hold, whatever the relay says");
    }

    #[test]
    fn a_notice_the_relay_accepted_late_is_not_trusted() {
        assert!(age_ok(TS, Some(TS + REAGENT_SIG_MAX_AGE_SECS), TS + 3600));
        assert!(!age_ok(TS, Some(TS + REAGENT_SIG_MAX_AGE_SECS + 1), TS + 3600), "signed long before it was accepted");
    }

    #[test]
    fn a_relay_time_that_cannot_be_true_falls_back_to_the_delivery_time_rule() {
        let now = TS + 3600;
        assert!(!age_ok(TS, Some(now + CLOCK_SKEW_SECS + 1), now), "accepted in the future");
        assert!(!age_ok(TS, Some(TS - CLOCK_SKEW_SECS - 1), now), "accepted before it was signed");
        assert!(age_ok(TS, Some(TS - CLOCK_SKEW_SECS), now), "within skew of the signing time");
        assert!(!age_ok(TS, None, now), "no time at all (an older relay)");
        assert!(!age_ok(0, Some(0), 3600) && !age_ok(-5, Some(-5), 3600), "no signing time is never fresh");
    }

    #[test]
    fn a_delivered_message_is_a_repeat_only_under_another_delivery() {
        let mut d = Delivered::default();
        assert_eq!(d.delivered_under_another("m1", "agentx", "inj-1", 100), None, "never delivered");
        d.record("m1", "agentx", "inj-1", 100);
        assert_eq!(d.delivered_under_another("m1", "agentx", "inj-1", 101), None, "the same delivery claimed again");
        assert_eq!(d.delivered_under_another("m1", "agentx", "inj-2", 102).as_deref(), Some("inj-1"), "another delivery of it");
    }

    #[test]
    fn one_signed_id_for_two_recipients_never_meets() {
        let mut d = Delivered::default();
        d.record("m1", "author", "inj-1", 100);
        assert_eq!(d.delivered_under_another("m1", "leader", "inj-2", 101), None, "another agent is not a repeat");
        d.record("m1", "leader", "inj-2", 101);
        assert_eq!(d.delivered_under_another("m1", "author", "inj-1", 102), None);
        assert_eq!(d.delivered_under_another("m1", "leader", "inj-2", 102), None);
        assert_eq!(d.delivered_under_another("m1", "leader", "inj-3", 102).as_deref(), Some("inj-2"));
    }

    #[test]
    fn a_record_is_kept_for_the_longest_hold_and_the_first_delivery_keeps_it() {
        let mut d = Delivered::default();
        d.record("m1", "agentx", "inj-1", 100);
        d.record("m1", "agentx", "inj-2", 5000);
        assert!(d.delivered_under_another("m1", "agentx", "inj-2", 100 + SIG_MAX_HOLD_SECS - 1).is_some(), "remembered at the end of the bound");
        assert_eq!(d.delivered_under_another("m1", "agentx", "inj-2", 100 + SIG_MAX_HOLD_SECS), None, "forgotten once no signature can pass");
    }

    #[test]
    fn the_records_stay_bounded_and_drop_the_one_due_soonest() {
        let mut d = Delivered::default();
        for i in 0..MAX_SEEN as i64 {
            d.record(&format!("m{i}"), "agentx", "inj", 1_000 + i);
        }
        assert_eq!(d.ids.len(), MAX_SEEN);
        d.record("one-more", "agentx", "inj", 1_000 + MAX_SEEN as i64);
        assert_eq!(d.ids.len(), MAX_SEEN);
        assert!(!d.ids.contains_key(&("m0".to_string(), "agentx".to_string())), "the oldest entry went");
        assert!(d.ids.contains_key(&("one-more".to_string(), "agentx".to_string())));
    }
}
