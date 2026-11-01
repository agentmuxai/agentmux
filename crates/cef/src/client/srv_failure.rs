// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A call from the host to srv that did not get through, for the person to read.
//!
//! `helpers::post_as_host` fails with the operating system's own words
//! ("read from srv: A connection attempt failed because the connected party did
//! not properly respond … (os error 10060)"), which a banner used to show as it
//! was. This tells a failure to reach srv from srv's own refusal, retries the
//! first for a call that is safe to repeat, and says in plain words what failed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// How long each try of a repeatable call waits for srv.
pub(crate) const TRY_WITHIN: Duration = Duration::from_secs(8);
/// The pauses between the tries of a repeatable call: three tries in all.
pub(crate) const RETRY_PAUSES: [Duration; 2] = [Duration::from_millis(400), Duration::from_millis(1500)];

/// How far a failed call got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// It never reached srv (no connection), so nothing was changed.
    NotSent,
    /// The request went out and srv did not answer in time or the connection
    /// broke: srv may or may not have acted on it.
    NoAnswer,
}

/// How far `error`, from `post_as_host`, got; None when srv answered (a refusal
/// is srv's own reason and is shown as it is).
pub(crate) fn reach_of(error: &str) -> Option<Reach> {
    if error.starts_with("connect to srv:") || error == "no srv endpoint" {
        Some(Reach::NotSent)
    } else if error.starts_with("write to srv:") || error.starts_with("read from srv:") {
        Some(Reach::NoAnswer)
    } else {
        None
    }
}

/// What to tell the person when `what` (a phrase such as "turn on auto-approve")
/// could not be done because srv could not be reached. `repeatable`: asking
/// again does no harm if it was done (setting a state), which the text then says;
/// an answer to a request is refused the second time, so it says to look first.
pub(crate) fn unreachable_text(what: &str, reach: Reach, repeatable: bool) -> String {
    match (reach, repeatable) {
        (Reach::NotSent, _) => format!("Couldn't reach AgentMux to {what}. Nothing was changed. Try again in a moment."),
        (Reach::NoAnswer, true) => format!(
            "AgentMux didn't answer in time when asked to {what}, so it isn't certain it was done. Check this pane; if it isn't, try again: repeating it is safe."
        ),
        (Reach::NoAnswer, false) => format!(
            "AgentMux didn't answer in time when asked to {what}, so it isn't certain it was done. Check this pane before trying again."
        ),
    }
}

/// What the older of two calls on the same thing is told when the person has
/// since asked again: the newer request decides, so the older is not repeated.
pub(crate) const SUPERSEDED: &str = "A newer request replaced this one.";

static LATEST: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// The place of one call among the calls on the same thing (`key`, e.g. a
/// pane's auto-approve): a call that is repeated must not undo a newer one the
/// person made while it waited (a retried "turn on" after a "stop").
pub(crate) struct Ticket {
    key: String,
    n: u64,
}

impl Ticket {
    pub(crate) fn take(key: String) -> Ticket {
        let mut all = LATEST.lock().unwrap_or_else(|e| e.into_inner());
        let n = all.get_or_insert_with(HashMap::new).entry(key.clone()).and_modify(|n| *n += 1).or_insert(1);
        Ticket { key, n: *n }
    }

    /// Whether no newer call on the same thing has been taken since.
    pub(crate) fn is_latest(&self) -> bool {
        let all = LATEST.lock().unwrap_or_else(|e| e.into_inner());
        all.as_ref().and_then(|m| m.get(&self.key)).is_none_or(|n| *n == self.n)
    }
}

/// Runs `call` and, while it fails to reach srv, again after each pause in
/// `backoff` (so `backoff.len() + 1` tries), unless `wanted` says a newer call
/// has taken its place ([`SUPERSEDED`]). Only for a call that is safe to repeat.
/// A refusal from srv is returned at once. If an earlier try may have gone out,
/// the error stays an uncertain one even when the last try never connected.
pub(crate) fn retrying<T>(
    backoff: &[Duration],
    wanted: impl Fn() -> bool,
    mut call: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let mut pauses = backoff.iter();
    let mut may_have_gone_out = false;
    loop {
        match call() {
            Err(e) if reach_of(&e).is_some() => {
                may_have_gone_out |= reach_of(&e) == Some(Reach::NoAnswer);
                let Some(pause) = pauses.next() else {
                    return Err(if may_have_gone_out && reach_of(&e) == Some(Reach::NotSent) {
                        format!("{UNCERTAIN}an earlier try may have gone through ({e})")
                    } else {
                        e
                    });
                };
                std::thread::sleep(*pause);
                if !wanted() {
                    return Err(SUPERSEDED.to_string());
                }
            }
            other => return other,
        }
    }
}

/// Starts an error `reach_of` reads as [`Reach::NoAnswer`].
const UNCERTAIN: &str = "read from srv: ";

/// `error` for the person, for a call that is safe to repeat: plain words when
/// srv could not be reached, else as it is.
pub(crate) fn plain(what: &str, error: String) -> String {
    match reach_of(&error) {
        Some(reach) => unreachable_text(what, reach, true),
        None => error,
    }
}

/// [`plain`] for a call that is not safe to repeat: no promise that asking again is harmless.
pub(crate) fn plain_once(what: &str, error: String) -> String {
    match reach_of(&error) {
        Some(reach) => unreachable_text(what, reach, false),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: &str = "read from srv: A connection attempt failed because the connected party did not properly respond after a period of time, or established connection failed because connected host has failed to respond. (os error 10060)";

    #[test]
    fn a_failure_to_reach_srv_is_told_from_srvs_own_refusal() {
        assert_eq!(reach_of("connect to srv: refused (os error 10061)"), Some(Reach::NotSent));
        assert_eq!(reach_of("no srv endpoint"), Some(Reach::NotSent));
        assert_eq!(reach_of(TIMEOUT), Some(Reach::NoAnswer));
        assert_eq!(reach_of("write to srv: broken pipe"), Some(Reach::NoAnswer));
        assert_eq!(reach_of("HTTP/1.1 409 Conflict {\"error\":\"the page moved\"}"), None);
        assert_eq!(reach_of("the page moved"), None);
    }

    #[test]
    fn the_person_never_reads_the_operating_systems_words() {
        let said = plain("turn on auto-approve", TIMEOUT.to_string());
        assert!(!said.contains("os error") && !said.contains("connection attempt") && !said.contains("read from srv"), "{said}");
        assert!(said.contains("turn on auto-approve"), "{said}");
        // It says whether it can be repeated and whether anything was turned on.
        assert!(said.contains("try again") && said.contains("safe"), "{said}");
        let never_sent = plain("turn on auto-approve", "connect to srv: refused (os error 10061)".to_string());
        assert!(never_sent.contains("Nothing was changed"), "{never_sent}");
        assert!(!never_sent.contains("os error"), "{never_sent}");
        // srv's own reason is shown as it is.
        assert_eq!(plain("turn on auto-approve", "the page moved".to_string()), "the page moved");
    }

    #[test]
    fn an_answer_that_may_have_arrived_is_not_called_safe_to_repeat() {
        let said = plain_once("send your answer", TIMEOUT.to_string());
        assert!(said.contains("isn't certain") && said.contains("Check this pane before trying again"), "{said}");
        assert!(!said.contains("safe") && !said.contains("os error"), "{said}");
        // Never sent: nothing was changed, so trying again is plain.
        let never = plain_once("send your answer", "connect to srv: refused (os error 10061)".to_string());
        assert!(never.contains("Nothing was changed"), "{never}");
    }

    #[test]
    fn a_failure_to_reach_srv_is_tried_again_and_a_refusal_is_not() {
        let none = [Duration::ZERO, Duration::ZERO];
        let mut calls = 0;
        let ok = retrying(&none, || true, || {
            calls += 1;
            if calls < 3 { Err(TIMEOUT.to_string()) } else { Ok("done") }
        });
        assert_eq!((ok, calls), (Ok("done"), 3));

        let mut calls = 0;
        let gave_up: Result<(), String> = retrying(&none, || true, || {
            calls += 1;
            Err(TIMEOUT.to_string())
        });
        assert_eq!((gave_up.is_err(), calls), (true, 3), "one try and one per pause");

        let mut calls = 0;
        let refused: Result<(), String> = retrying(&none, || true, || {
            calls += 1;
            Err("the page moved".to_string())
        });
        assert_eq!((refused, calls), (Err("the page moved".to_string()), 1));
    }

    #[test]
    fn a_repeated_call_does_not_undo_a_newer_one() {
        let none = [Duration::ZERO, Duration::ZERO];
        let first = Ticket::take("test-pane-newer".into());
        let mut calls = 0;
        let out: Result<(), String> = retrying(&none, || first.is_latest(), || {
            calls += 1;
            // The person asks the opposite while this call waits for srv.
            let _stop = Ticket::take("test-pane-newer".into());
            Err(TIMEOUT.to_string())
        });
        assert_eq!((out, calls), (Err(SUPERSEDED.to_string()), 1), "no second try after a newer call");
        // Another pane's calls are not affected.
        let other = Ticket::take("test-pane-other".into());
        assert!(other.is_latest());
    }

    #[test]
    fn an_earlier_try_that_may_have_gone_out_keeps_the_outcome_uncertain() {
        let none = [Duration::ZERO, Duration::ZERO];
        let mut calls = 0;
        let out: Result<(), String> = retrying(&none, || true, || {
            calls += 1;
            Err(if calls == 1 { TIMEOUT } else { "connect to srv: refused (os error 10061)" }.to_string())
        });
        let e = out.unwrap_err();
        assert_eq!(reach_of(&e), Some(Reach::NoAnswer), "{e}");
        assert!(!plain("turn on auto-approve", e).contains("Nothing was changed"));
    }
}
