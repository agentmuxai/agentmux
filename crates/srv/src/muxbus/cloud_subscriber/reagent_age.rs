// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How old a signed notice may be when it is delivered, and the replay check that goes with that.
//!
//! A notice is signed when it is sent and may then wait at the relay for a host that is off. Judging the
//! signature's age at delivery alone would read every notice held past [`REAGENT_SIG_MAX_AGE_SECS`] as forged.
//! The age is therefore judged at the moment the relay accepted the notice (its own record, `created_at`), with
//! two bounds that need no trust in the relay: the notice must be inside the longest hold a sender uses, and a
//! signed message id is accepted once ([`first_use`]).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::super::{reagent_sig_is_fresh, REAGENT_SIG_MAX_AGE_SECS};

/// The longest a signed notice may have been held, from signing to delivery, in seconds: the longest hold a sender
/// uses for a host that is off (6 hours today; a sender's hold must not exceed this) plus the 10 minutes a notice
/// may take to be accepted. A signature older than this is never a notice still inside its own hold.
pub(super) const SIG_MAX_HOLD_SECS: i64 = 6 * 60 * 60 + REAGENT_SIG_MAX_AGE_SECS;

/// How far the relay's clock and the signer's may differ.
const CLOCK_SKEW_SECS: i64 = 60;

/// Signed message ids remembered at most.
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

/// The signed message ids already accepted, each with the delivery it was accepted for and when it is forgotten.
#[derive(Default)]
struct Seen {
    ids: HashMap<String, (String, i64)>,
}

impl Seen {
    /// Record `msg_id` as accepted for the delivery `inj_id`. The same delivery again (a claim retried) is not a
    /// repeat; the same message id under another delivery is, and is refused. Entries are kept for
    /// [`SIG_MAX_HOLD_SECS`], the longest a signature can still pass; past `MAX_SEEN` the one due soonest is dropped.
    fn first_use(&mut self, msg_id: &str, inj_id: &str, now: i64) -> bool {
        if self.ids.len() >= MAX_SEEN {
            self.ids.retain(|_, (_, until)| *until > now);
        }
        if let Some((seen_for, until)) = self.ids.get(msg_id) {
            if *until > now {
                return seen_for == inj_id;
            }
        }
        if self.ids.len() >= MAX_SEEN {
            if let Some(oldest) = self.ids.iter().min_by_key(|(_, (_, until))| *until).map(|(k, _)| k.clone()) {
                self.ids.remove(&oldest);
            }
        }
        self.ids.insert(msg_id.to_string(), (inj_id.to_string(), now + SIG_MAX_HOLD_SECS));
        true
    }
}

static SEEN: LazyLock<Mutex<Seen>> = LazyLock::new(Default::default);

/// Accept the signed message id `msg_id` for the delivery `inj_id` once (see [`Seen::first_use`]).
pub(super) fn first_use(msg_id: &str, inj_id: &str, now: i64) -> bool {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).first_use(msg_id, inj_id, now)
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
    fn a_signed_message_id_is_accepted_once_per_delivery() {
        let mut seen = Seen::default();
        assert!(seen.first_use("m1", "inj-1", 100));
        assert!(seen.first_use("m1", "inj-1", 101), "the same delivery claimed again");
        assert!(!seen.first_use("m1", "inj-2", 102), "the same signed message under another delivery is a replay");
        assert!(seen.first_use("m2", "inj-2", 102));
        assert!(!seen.first_use("m1", "inj-2", 100 + SIG_MAX_HOLD_SECS - 1), "still remembered at the end of the bound");
        assert!(seen.first_use("m1", "inj-3", 100 + SIG_MAX_HOLD_SECS + 1), "forgotten once no signature can pass");
    }

    #[test]
    fn the_seen_ids_stay_bounded_and_drop_the_one_due_soonest() {
        let mut seen = Seen::default();
        for i in 0..MAX_SEEN as i64 {
            assert!(seen.first_use(&format!("m{i}"), "inj", 1_000 + i));
        }
        assert_eq!(seen.ids.len(), MAX_SEEN);
        assert!(seen.first_use("one-more", "inj", 1_000 + MAX_SEEN as i64));
        assert_eq!(seen.ids.len(), MAX_SEEN);
        assert!(!seen.ids.contains_key("m0"), "the oldest entry went");
        assert!(seen.ids.contains_key("m1") && seen.ids.contains_key("one-more"));
    }
}
