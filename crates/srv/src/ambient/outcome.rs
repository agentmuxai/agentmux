// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One outcome per ambient call, logged and counted.
//!
//! The module used to be silent: its failure paths logged at debug, so a rejected,
//! empty or failed title left no trace, and diagnosing the `(none yet)` swarm row
//! took a database copy (docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md
//! section 2.1 defect 6, section 5.8). Every call now ends in exactly one
//! [`Outcome`], written as one structured info line and counted per purpose, and
//! the counts are readable over the `ambient.outcomes` RPC (the Instance panel).
//! The rejected text is logged, truncated, because rejected values are the corpus
//! the title predicate is tuned with.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use super::prompt::KEEP_TOKEN;

/// How an ambient call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The reply passed validation and was used.
    Accepted,
    /// The model abstained with the `KEEP` token: no change, the safe answer.
    Kept,
    /// The model answered `SKIP` in the reply format (`reply`): nothing to write.
    /// A healthy answer, like `Kept`.
    Skipped,
    /// No model call: a check in code already showed there is nothing to write
    /// (for a suggestion, the assistant's last message waits for the user).
    Gated,
    /// The reply was refused; the reason says by which rule.
    Rejected(RejectReason),
    /// There was nothing to send: no conversation in the digest, or no message
    /// and no title to work from. No model call was made.
    EmptyDigest,
    /// A newer request for the same key replaced this one, before or during the
    /// call.
    Superseded,
    /// The CLI could not be run or failed.
    CliFailed,
    /// The CLI ran past its time limit.
    Timeout,
    /// Admitted, then given up before any model call: the block or its CLI path
    /// could not be resolved, or a purpose found nothing to do. Recorded when an
    /// admitted call is dropped without running (`call::Slot`'s `Drop`).
    NotRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Blank after trimming.
    Empty,
    /// Wrong shape: several lines, too long, or no letters.
    Shape,
    /// About the absence of a title (`none`, `(none yet)`, `untitled`, ...).
    AbsencePattern,
    /// A refusal or an assistant-style reply instead of the thing asked for.
    Refusal,
    /// Refused by a purpose-specific rule the generic classifier cannot name.
    Other,
    /// Not in the reply format (`reply`): no `ANSWER:` prefix and not `SKIP`.
    Format,
}

impl Outcome {
    /// The stable label used in the log line and the counters.
    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Accepted => "accepted",
            Outcome::Kept => "kept",
            Outcome::Skipped => "skipped",
            Outcome::Gated => "gated",
            Outcome::Rejected(RejectReason::Format) => "rejected:format",
            Outcome::Rejected(RejectReason::Empty) => "rejected:empty",
            Outcome::Rejected(RejectReason::Shape) => "rejected:shape",
            Outcome::Rejected(RejectReason::AbsencePattern) => "rejected:absence_pattern",
            Outcome::Rejected(RejectReason::Refusal) => "rejected:refusal",
            Outcome::Rejected(RejectReason::Other) => "rejected:other",
            Outcome::EmptyDigest => "empty_digest",
            Outcome::Superseded => "superseded",
            Outcome::CliFailed => "cli_failed",
            Outcome::Timeout => "timeout",
            Outcome::NotRun => "not_run",
        }
    }
}

/// Classify a reply the CLI returned. `accepted` is whether the purpose's own
/// validator took it; when it did not, the reason is named by the generic rules
/// (`validate::rejection_reason`), which cover every title-shaped purpose.
pub fn classify_reply(raw: &str, accepted: bool) -> Outcome {
    if accepted {
        return Outcome::Accepted;
    }
    let trimmed = raw.trim();
    // An abstain, bare or explained ("KEEP — the title still fits"): the reply
    // leads with the token. Counted as kept, never as a refusal, or the Titles row
    // would warn about a healthy pipeline and the logged rejected corpus would
    // fill with abstains (ReAgent P2 on #4243). A real title that starts with the
    // word ("Keep alive pings") is accepted above and never reaches this.
    let first_word = trimmed.split_whitespace().next().unwrap_or("");
    if first_word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .eq_ignore_ascii_case(KEEP_TOKEN)
    {
        return Outcome::Kept;
    }
    Outcome::Rejected(super::validate::rejection_reason(trimmed))
}

/// Classify a failed CLI call. `cancelled` is whether the gateway cancelled it
/// because a newer request superseded it.
pub fn classify_error(error: &str, cancelled: bool) -> Outcome {
    if cancelled {
        return Outcome::Superseded;
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        Outcome::Timeout
    } else {
        Outcome::CliFailed
    }
}

type Counts = BTreeMap<&'static str, BTreeMap<&'static str, u64>>;

fn counts() -> &'static Mutex<Counts> {
    static COUNTS: OnceLock<Mutex<Counts>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Longest text logged with a rejected or kept outcome.
const LOGGED_TEXT_MAX_CHARS: usize = 120;

fn truncated(text: &str) -> String {
    let mut out: String = text.chars().take(LOGGED_TEXT_MAX_CHARS).collect();
    if text.chars().count() > LOGGED_TEXT_MAX_CHARS {
        out.push('…');
    }
    out.replace('\n', " ⏎ ")
}

/// How long a call waited and ran, for the outcome line. Both in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timing {
    /// Waiting for a concurrency permit, from admission.
    pub queued_ms: u64,
    /// The CLI call itself, spawn to exit (or cancellation, or the time limit).
    pub run_ms: u64,
}

/// Record one outcome without timings: count it, and log it at info with the
/// purpose and entity. See [`record_timed`].
pub fn record(purpose: &'static str, entity: &str, outcome: Outcome, text: Option<&str>) {
    record_timed(purpose, entity, outcome, text, None);
}

/// Record one outcome: count it, and log it at info with the purpose, the entity
/// and, when the call ran, how long it queued and ran. `text` is logged
/// (truncated) for a rejected reply, which is the corpus the validators are tuned
/// with, and for an accepted one, which is what the user is shown: without it, a
/// bad answer that passed could not be found afterwards.
pub fn record_timed(purpose: &'static str, entity: &str, outcome: Outcome, text: Option<&str>, timing: Option<Timing>) {
    if let Ok(mut map) = counts().lock() {
        *map.entry(purpose)
            .or_default()
            .entry(outcome.label())
            .or_default() += 1;
    }
    let text = match outcome {
        Outcome::Rejected(_) | Outcome::Accepted => text.map(truncated),
        _ => None,
    };
    let queued_ms = timing.map(|t| t.queued_ms);
    let run_ms = timing.map(|t| t.run_ms);
    tracing::info!(
        purpose,
        entity,
        outcome = outcome.label(),
        text = text.as_deref(),
        queued_ms,
        run_ms,
        "ambient outcome"
    );
}

/// Every count since this srv started, by purpose then outcome label.
pub fn snapshot() -> BTreeMap<String, BTreeMap<String, u64>> {
    let Ok(map) = counts().lock() else {
        return BTreeMap::new();
    };
    map.iter()
        .map(|(purpose, by)| {
            (
                purpose.to_string(),
                by.iter()
                    .map(|(label, n)| (label.to_string(), *n))
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_classified_by_the_rule_that_refused_it() {
        assert_eq!(
            classify_reply("Fix the login race", true),
            Outcome::Accepted
        );
        assert_eq!(classify_reply("KEEP", false), Outcome::Kept);
        assert_eq!(classify_reply("  keep. ", false), Outcome::Kept);
        // An explained abstain is still an abstain, not a refusal.
        assert_eq!(
            classify_reply("KEEP — the title still fits", false),
            Outcome::Kept
        );
        assert_eq!(
            classify_reply("Keep the current title", false),
            Outcome::Kept
        );
        // A real title starting with the word was accepted, so it never gets here.
        assert_eq!(classify_reply("Keep alive pings", true), Outcome::Accepted);
        assert_eq!(
            classify_reply("   ", false),
            Outcome::Rejected(RejectReason::Empty)
        );
        assert_eq!(
            classify_reply("(none yet)", false),
            Outcome::Rejected(RejectReason::AbsencePattern)
        );
        assert_eq!(
            classify_reply("Line one\nline two", false),
            Outcome::Rejected(RejectReason::Shape)
        );
        assert_eq!(
            classify_reply("12345", false),
            Outcome::Rejected(RejectReason::Shape)
        );
        assert_eq!(
            classify_reply("I'm sorry, but I can't help with that", false),
            Outcome::Rejected(RejectReason::Refusal)
        );
    }

    #[test]
    fn a_failure_is_superseded_timed_out_or_a_cli_failure() {
        assert_eq!(classify_error("cancelled", true), Outcome::Superseded);
        assert_eq!(
            classify_error("haiku call timed out after 15s", false),
            Outcome::Timeout
        );
        assert_eq!(
            classify_error("spawn failed: not found", false),
            Outcome::CliFailed
        );
    }

    #[test]
    fn outcomes_are_counted_per_purpose() {
        // A purpose no other test uses, so parallel tests cannot disturb it.
        let p = "test_purpose_outcome_counts";
        record(p, "b1", Outcome::Accepted, None);
        record(p, "b1", Outcome::Accepted, None);
        record(
            p,
            "b2",
            Outcome::Rejected(RejectReason::AbsencePattern),
            Some("(none yet)"),
        );
        let snap = snapshot();
        let by = &snap[p];
        assert_eq!(by["accepted"], 2);
        assert_eq!(by["rejected:absence_pattern"], 1);
    }

    #[test]
    fn logged_text_is_bounded_and_on_one_line() {
        let long = "x".repeat(500);
        assert_eq!(truncated(&long).chars().count(), LOGGED_TEXT_MAX_CHARS + 1);
        assert!(!truncated("a\nb").contains('\n'));
    }
}
