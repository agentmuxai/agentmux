// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The reply format an ambient call asks for, and the one reader of it.
//!
//! The model answers with exactly one of:
//!
//! ```text
//! ANSWER: <the text>
//! SKIP
//! ```
//!
//! A reply is used only when it has that form. Asked to "output nothing", Haiku
//! often writes a sentence about doing so instead ("Output nothing at all - the
//! assistant's message ends by asking for a decision"), and no list of phrases
//! can name every wording of that. Without the `ANSWER:` prefix, any of them is
//! simply not an answer. docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md
//! section 6.1.

/// Appended to a prompt that uses this format.
pub const FORMAT_RULES: &str = "Reply with exactly one line, in one of two forms: ANSWER: followed by the text, \
or the single word SKIP when the text should not be written. Nothing before or after it.";

const ANSWER_PREFIX: &str = "ANSWER:";
const SKIP: &str = "SKIP";

/// What a call's judge decided about a reply. Every ambient call ends in one,
/// and its outcome is recorded from it (`call::Slot::run`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The text to use.
    Use(String),
    /// The model decided there is nothing to write (`SKIP`): a healthy answer.
    Skip,
    /// Not usable, and why.
    Reject(super::outcome::RejectReason),
}

/// Judge a one-line reply in this format: `ANSWER:` text that `check` takes is
/// used, `SKIP` is a skip, and anything else is refused, by `check` (`Other`) or
/// for not being in the format (`Format`, or `Empty` for no reply at all).
pub fn judge_line(raw: &str, check: impl Fn(&str) -> Option<String>) -> Verdict {
    use super::outcome::RejectReason;
    match parse(raw) {
        Parsed::Answer(answer) => match check(&answer) {
            Some(text) => Verdict::Use(text),
            None => Verdict::Reject(RejectReason::Other),
        },
        Parsed::Skip => Verdict::Skip,
        Parsed::Malformed if raw.trim().is_empty() => Verdict::Reject(RejectReason::Empty),
        Parsed::Malformed => Verdict::Reject(RejectReason::Format),
    }
}

/// What a reply in this format says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// `ANSWER: <text>`, with the text trimmed and non-empty.
    Answer(String),
    /// `SKIP`: the model decided there is nothing to write. A healthy result.
    Skip,
    /// Anything else, including an empty reply.
    Malformed,
}

/// Read a reply. Case is ignored for the prefix and for `SKIP`, as is trailing
/// punctuation on `SKIP`. The text after `ANSWER:` must be one line, and is
/// cleaned by `sanitize_ambient_text` (wrapping quotes and fences, a filler
/// opener): the CLI sanitizes the whole reply, whose start is now the prefix, so
/// that pass no longer reaches the answer itself.
pub fn parse(raw: &str) -> Parsed {
    let text = raw.trim();
    if text.is_empty() {
        return Parsed::Malformed;
    }
    let bare = text.trim_matches(|c: char| !c.is_alphanumeric());
    if bare.eq_ignore_ascii_case(SKIP) {
        return Parsed::Skip;
    }
    let Some(head) = text.get(..ANSWER_PREFIX.len()) else {
        return Parsed::Malformed;
    };
    if !head.eq_ignore_ascii_case(ANSWER_PREFIX) {
        return Parsed::Malformed;
    }
    let answer = super::sanitize::sanitize_ambient_text(text[ANSWER_PREFIX.len()..].trim());
    if answer.is_empty() || answer.contains('\n') {
        return Parsed::Malformed;
    }
    // `ANSWER: SKIP` mixes the two forms; the model meant the skip, and the word
    // must never be stored as a title, a name or a suggestion.
    if answer.trim_matches(|c: char| !c.is_alphanumeric()).eq_ignore_ascii_case(SKIP) {
        return Parsed::Skip;
    }
    Parsed::Answer(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_that_is_only_skip_is_a_skip() {
        for raw in ["ANSWER: SKIP", "answer: skip.", "ANSWER: \"SKIP\""] {
            assert_eq!(parse(raw), Parsed::Skip, "{raw:?}");
        }
        assert_eq!(parse("ANSWER: Skip the flaky test"), Parsed::Answer("Skip the flaky test".into()));
    }

    #[test]
    fn an_answer_is_its_text() {
        assert_eq!(parse("ANSWER: Run the tests"), Parsed::Answer("Run the tests".into()));
        assert_eq!(parse("  answer:   Add a regression test  "), Parsed::Answer("Add a regression test".into()));
        assert_eq!(parse("Answer:Fix the login race"), Parsed::Answer("Fix the login race".into()));
    }

    #[test]
    fn skip_is_skip_however_it_is_dressed() {
        for s in ["SKIP", "skip", " Skip. ", "`SKIP`", "\"SKIP\""] {
            assert_eq!(parse(s), Parsed::Skip, "{s:?}");
        }
    }

    /// Every way the model described abstaining in the 2026-10-04..08 logs, and the
    /// one that reached the composer. None has the prefix, so none is an answer.
    #[test]
    fn prose_about_saying_nothing_is_not_an_answer() {
        for s in [
            "Output nothing at all - the assistant's message ends by asking for a decision and waiting for user input.",
            "(No output - the assistant's last message asks the user a question and waits for a decision)",
            "[Output nothing - the assistant's last message explicitly waits for user decisions on three matters.]",
            "(no output)",
            "(nothing)",
            "I cannot predict a next instruction because the assistant's last message waits for your decision.",
            "Run the tests",
            "",
            "   ",
            "SKIP this one, the assistant asked a question",
        ] {
            assert_eq!(parse(s), Parsed::Malformed, "{s:?}");
        }
    }

    /// The answer gets the clean-up the whole reply used to get, which can no
    /// longer reach it past the prefix.
    #[test]
    fn the_answer_text_is_cleaned_of_quotes_fences_and_filler() {
        assert_eq!(parse("ANSWER: \"Run the tests\""), Parsed::Answer("Run the tests".into()));
        assert_eq!(parse("ANSWER: `Run the tests`"), Parsed::Answer("Run the tests".into()));
        assert_eq!(
            parse("ANSWER: Yeah, let's debug the blank preview bug"),
            Parsed::Answer("Debug the blank preview bug".into())
        );
        assert_eq!(parse("ANSWER: \"\""), Parsed::Malformed);
    }

    #[test]
    fn an_answer_must_have_text_on_one_line() {
        assert_eq!(parse("ANSWER:"), Parsed::Malformed);
        assert_eq!(parse("ANSWER:   "), Parsed::Malformed);
        assert_eq!(parse("ANSWER: Run the tests\nthen push"), Parsed::Malformed);
    }

    #[test]
    fn one_wrapping_pair_of_quotes_is_dropped_and_inner_quotes_are_kept() {
        assert_eq!(parse("ANSWER: \"Run the tests\""), Parsed::Answer("Run the tests".into()));
        assert_eq!(parse("ANSWER: `Fix the build`"), Parsed::Answer("Fix the build".into()));
        assert_eq!(parse("ANSWER: Rename it to \"foo\""), Parsed::Answer("Rename it to \"foo\"".into()));
        assert_eq!(
            parse("ANSWER: \"foo\" and \"bar\" both break"),
            Parsed::Answer("\"foo\" and \"bar\" both break".into())
        );
    }

    #[test]
    fn a_reply_is_judged_by_its_form_then_by_the_check() {
        use crate::ambient::outcome::RejectReason;
        let check = |t: &str| (!t.contains("rm -rf")).then(|| t.to_string());
        assert_eq!(judge_line("ANSWER: Run the tests", check), Verdict::Use("Run the tests".into()));
        assert_eq!(judge_line("SKIP", check), Verdict::Skip);
        assert_eq!(judge_line("ANSWER: rm -rf build", check), Verdict::Reject(RejectReason::Other));
        assert_eq!(
            judge_line(
                "Output nothing at all - the assistant's message ends by asking for a decision and waiting for user input.",
                check
            ),
            Verdict::Reject(RejectReason::Format)
        );
        assert_eq!(judge_line("  ", check), Verdict::Reject(RejectReason::Empty));
    }

    #[test]
    fn the_format_rules_name_both_forms() {
        assert!(FORMAT_RULES.contains("ANSWER:"));
        assert!(FORMAT_RULES.contains("SKIP"));
    }
}
