// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Output validation for ambient model calls. `sanitize::sanitize_ambient_text`
//! strips formatting; this decides whether what's left is usable at all. A model
//! that was handed too little context tends to answer *as an assistant* ("I
//! don't have access to...", "If you'd like me to...") instead of with the
//! requested text, and that must never reach a pane title, a name, a narration
//! line or a ghost-text composer.
//!
//! Every ambient call's reply goes through [`accept_line`] (via `call::Slot::run`),
//! so a refusal, a paragraph or a placeholder is dropped the same way everywhere.
//! A next-prompt suggestion additionally goes through [`accept_next_prompt`],
//! because it is something the user may send.

/// Size bounds for a one-line reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_words: usize,
    pub max_chars: usize,
}

/// A pane/session title asked for `word_target` words. The slack covers a model
/// that runs a little over; a paragraph is still rejected.
pub fn title_limits(word_target: u32) -> Limits {
    Limits { max_words: word_target as usize + 8, max_chars: 200 }
}

/// A subagent or workflow-batch name (asked for ~5 words).
pub const NAME: Limits = Limits { max_words: 12, max_chars: 100 };
/// A definition's conversation preview (asked for 12 words or fewer).
pub const PREVIEW: Limits = Limits { max_words: 24, max_chars: 240 };
/// A one-sentence narration (asked for under 20 words).
pub const NARRATION: Limits = Limits { max_words: 40, max_chars: 300 };
/// A next-prompt suggestion is one short imperative; anything longer is an
/// explanation, not a command.
pub const NEXT_PROMPT: Limits = Limits { max_words: 28, max_chars: 160 };

/// First-person refusals and offers: a model talking to the reader instead of
/// producing the text. Specific enough not to fire on an ordinary title.
const REFUSAL_PHRASES: &[&str] = &[
    "i don't have",
    "i do not have",
    "i don't know",
    "i cannot",
    "i can't",
    "i'm unable",
    "i am unable",
    "i'm not able",
    "i would need",
    "i'd need",
    "i need to see",
    "i need more",
    "if you'd like",
    "if you would like",
    "if you want me",
    "without knowing",
    "not enough context",
    "not enough information",
    "as an ai",
];

/// More ways a next-prompt reply can be about the task instead of the next
/// step. Too broad for titles ("Fix insufficient permissions error"), so only
/// the next-prompt check uses them.
const NEXT_PROMPT_META_PHRASES: &[&str] = &[
    "without more",
    "no recent activity",
    "insufficient",
    "the user is",
    "the user has",
    "next instruction",
    "next step would",
    "recent activity",
    "cost summary",
    "nothing plausible",
    "no plausible",
    "empty string",
];

/// Things a ghost suggestion must not propose on its own initiative: one Tab and
/// Enter away from the user sending them without having thought of them.
const RISKY_PHRASES: &[&str] = &[
    "rm -rf",
    "--force",
    "force push",
    "force-push",
    "reset --hard",
    "drop table",
    "drop database",
    "delete the database",
    "delete all",
    "wipe ",
    "password",
    "secret",
    "credential",
    "api key",
    "api_key",
    "token",
];

/// Lower-cased text with typographic apostrophes straightened, for phrase
/// matching only (the text handed back is untouched).
fn normalized(text: &str) -> String {
    text.replace(['\u{2019}', '\u{2018}'], "'").to_lowercase()
}

fn is_placeholder(lower: &str) -> bool {
    let bare = lower.trim_matches(|c: char| !c.is_alphanumeric());
    matches!(bare, "none" | "n/a" | "na" | "nothing" | "empty" | "null")
}

/// Returns the line to use, or `None` if the reply is not usable text: empty,
/// multi-line, over the limits, a placeholder, or a model refusal. This is the
/// check every ambient reply gets.
pub fn accept_line(raw: &str, limits: &Limits) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    if text.chars().count() > limits.max_chars
        || text.split_whitespace().count() > limits.max_words
    {
        return None;
    }
    if !text.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let lower = normalized(text);
    if is_placeholder(&lower) {
        return None;
    }
    if REFUSAL_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }
    Some(text.to_string())
}

/// Returns the suggestion to show, or `None` if the text is not a usable next
/// prompt: anything [`accept_line`] rejects, a question, talk about the task, or
/// a risky command.
pub fn accept_next_prompt(raw: &str) -> Option<String> {
    let text = accept_line(raw, &NEXT_PROMPT)?;
    // A question is the model asking, not the user telling.
    if text.ends_with('?') {
        return None;
    }
    let lower = normalized(&text);
    if NEXT_PROMPT_META_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }
    if RISKY_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_imperative_passes_through() {
        assert_eq!(accept_next_prompt("Run the tests"), Some("Run the tests".into()));
        assert_eq!(
            accept_next_prompt("  Debug the blank preview bug next  "),
            Some("Debug the blank preview bug next".into())
        );
    }

    #[test]
    fn the_reported_refusal_is_rejected() {
        let reported = "I don't have access to the details of the recent activity beyond the cost summary. \
            Without knowing what work was done or what the current state of the project is, I cannot \
            plausibly predict the next instruction. If you'd like me to predict your next step, I'd \
            need to see the actual recent work or conversation history.";
        assert_eq!(accept_next_prompt(reported), None);
        assert_eq!(accept_line(reported, &title_limits(7)), None);
    }

    #[test]
    fn short_refusals_and_meta_talk_are_rejected() {
        for s in [
            "I cannot predict the next step",
            "I don't have enough information",
            "I don\u{2019}t have enough information",
            "If you'd like me to continue, say so",
            "Not enough context to decide",
            "No plausible next instruction",
            "The user is likely to ask for tests",
            "Based on the recent activity, run the tests",
        ] {
            assert_eq!(accept_next_prompt(s), None, "{s}");
        }
    }

    #[test]
    fn empty_and_placeholder_text_is_rejected() {
        for s in ["", "   ", "...", "-", "None", "N/A", "none.", "(empty)", "null"] {
            assert_eq!(accept_next_prompt(s), None, "{s:?}");
            assert_eq!(accept_line(s, &NAME), None, "{s:?}");
        }
    }

    #[test]
    fn questions_are_rejected_as_next_prompts_but_not_as_titles() {
        assert_eq!(accept_next_prompt("Should I run the tests?"), None);
        assert_eq!(accept_next_prompt("What should happen next?"), None);
        assert_eq!(
            accept_line("Why does login fail?", &title_limits(7)),
            Some("Why does login fail?".into())
        );
    }

    #[test]
    fn multi_line_and_over_long_text_is_rejected() {
        assert_eq!(accept_next_prompt("Run the tests\nthen push"), None);
        let long = "Fix ".to_string() + &"the thing and ".repeat(20);
        assert_eq!(accept_next_prompt(&long), None);
        let many_words = vec!["go"; 30].join(" ");
        assert_eq!(accept_next_prompt(&many_words), None);
    }

    #[test]
    fn risky_commands_are_rejected() {
        for s in [
            "Run rm -rf on the build folder",
            "Force push the branch",
            "Push with --force",
            "git reset --hard origin/main",
            "Print the API token",
            "Drop table users",
        ] {
            assert_eq!(accept_next_prompt(s), None, "{s}");
        }
    }

    #[test]
    fn ordinary_commands_pass_and_over_blocking_is_deliberate() {
        // "secret" also blocks "secretary": a missed ghost text costs nothing, so
        // the list errs toward blocking. Pinned so the trade-off is deliberate.
        assert_eq!(accept_next_prompt("Email the secretary"), None);
        assert_eq!(
            accept_next_prompt("Merge the PR once CI is green"),
            Some("Merge the PR once CI is green".into())
        );
    }

    #[test]
    fn the_text_handed_back_is_not_rewritten() {
        // Apostrophes are straightened for matching only.
        assert_eq!(
            accept_line("Fix the user\u{2019}s login", &NAME),
            Some("Fix the user\u{2019}s login".into())
        );
    }

    #[test]
    fn titles_and_names_may_use_words_only_the_next_prompt_check_blocks() {
        assert_eq!(
            accept_line("Fix insufficient permissions error", &title_limits(7)),
            Some("Fix insufficient permissions error".into())
        );
        assert_eq!(accept_next_prompt("Fix insufficient permissions error"), None);
        assert_eq!(
            accept_line("Rotate the API token flow", &NAME),
            Some("Rotate the API token flow".into())
        );
    }

    #[test]
    fn titles_and_names_reject_refusals_paragraphs_and_overruns() {
        assert_eq!(accept_line("I cannot determine a title without more context", &title_limits(7)), None);
        assert_eq!(accept_line("Task\nName: login", &NAME), None);
        let long = vec!["word"; 40].join(" ");
        assert_eq!(accept_line(&long, &NAME), None);
        assert_eq!(accept_line(&long, &title_limits(7)), None);
        assert!(accept_line("Fix login blank screen bug", &NAME).is_some());
    }

    #[test]
    fn narration_allows_a_sentence_but_not_a_paragraph() {
        assert!(accept_line("I started the dev server in the background.", &NARRATION).is_some());
        let paragraph = vec!["word"; 60].join(" ");
        assert_eq!(accept_line(&paragraph, &NARRATION), None);
    }

    #[test]
    fn title_limits_scale_with_the_requested_word_count() {
        assert_eq!(title_limits(7).max_words, 15);
        assert_eq!(title_limits(20).max_words, 28);
    }
}
