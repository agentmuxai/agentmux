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
/// punctuation on `SKIP` and a reply wrapped in quotes or backticks (the
/// sanitizer already removed those). The text after `ANSWER:` must be one line.
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
    let answer = text[ANSWER_PREFIX.len()..].trim();
    if answer.is_empty() || answer.contains('\n') {
        return Parsed::Malformed;
    }
    Parsed::Answer(answer.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn an_answer_must_have_text_on_one_line() {
        assert_eq!(parse("ANSWER:"), Parsed::Malformed);
        assert_eq!(parse("ANSWER:   "), Parsed::Malformed);
        assert_eq!(parse("ANSWER: Run the tests\nthen push"), Parsed::Malformed);
    }

    #[test]
    fn the_format_rules_name_both_forms() {
        assert!(FORMAT_RULES.contains("ANSWER:"));
        assert!(FORMAT_RULES.contains("SKIP"));
    }
}
