// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Output validation for ambient model calls whose text is shown to the user as
//! something they might send. `sanitize_ambient_text` (session.rs) strips
//! formatting; this decides whether what's left is usable at all. A model that
//! was handed too little context tends to answer *as an assistant* ("I don't
//! have access to...", "If you'd like me to...") instead of with the requested
//! text, and that must never reach a ghost-text composer.

/// A next-prompt suggestion is one short imperative; anything longer is an
/// explanation, not a command.
const NEXT_PROMPT_MAX_CHARS: usize = 160;
const NEXT_PROMPT_MAX_WORDS: usize = 28;

/// Phrases a model uses when it is talking *about* the task instead of doing it.
const META_PHRASES: &[&str] = &[
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
    "without more",
    "no recent activity",
    "not enough context",
    "not enough information",
    "insufficient",
    "as an ai",
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

/// Returns the suggestion to show, or `None` if the text is not a usable next
/// prompt: empty, multi-line, a question, over-long, a refusal or other talk
/// about the task, or a risky command.
pub fn accept_next_prompt(raw: &str) -> Option<String> {
    // The CLI can hand back typographic apostrophes; the phrase lists use ASCII.
    let text = raw.trim().replace(['\u{2019}', '\u{2018}'], "'");
    let text = text.as_str();
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    if text.chars().count() > NEXT_PROMPT_MAX_CHARS
        || text.split_whitespace().count() > NEXT_PROMPT_MAX_WORDS
    {
        return None;
    }
    // A question is the model asking, not the user telling.
    if text.ends_with('?') {
        return None;
    }
    if !text.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let lower = text.to_lowercase();
    let bare = lower.trim_matches(|c: char| !c.is_alphanumeric());
    if matches!(bare, "none" | "n/a" | "na" | "nothing" | "empty" | "null") {
        return None;
    }
    if META_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }
    if RISKY_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }
    Some(text.to_string())
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
        }
    }

    #[test]
    fn questions_are_rejected() {
        assert_eq!(accept_next_prompt("Should I run the tests?"), None);
        assert_eq!(accept_next_prompt("What should happen next?"), None);
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
}
