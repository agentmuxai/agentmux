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
    "wipe",
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

/// Whether `phrase` occurs in `lower` as whole words. A bare substring match
/// would read "has an AI summary bug" as the refusal "as an ai".
fn has_phrase(lower: &str, phrase: &str) -> bool {
    let mut from = 0;
    while let Some(found) = lower[from..].find(phrase) {
        let start = from + found;
        let end = start + phrase.len();
        let before_ok = lower[..start].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
        let after_ok = lower[end..].chars().next().map_or(true, |c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        // Advance past this match's first character, staying on a char boundary.
        from = start + lower[start..].chars().next().map_or(1, |c| c.len_utf8());
    }
    false
}

fn has_any(lower: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| has_phrase(lower, p))
}

/// Text that is ABOUT the absence of a title rather than a title. Compared after
/// [`absence_form`], so brackets, quotes and punctuation do not matter.
///
/// Exact entries are whole replies only: "none" must not reject "None of the tests
/// pass on Windows". Prefix entries are specific enough that a real title is very
/// unlikely to begin with them. The first real examples were `(none yet)`, which is
/// the title prompt's own placeholder echoed back, and `no goal established yet`;
/// see SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md. The corpus in
/// `title_corpus.json` is the contract, shared with the frontend.
const ABSENCE_EXACT: &[&str] = &[
    "none",
    "n a",
    "na",
    "null",
    "nothing",
    "empty",
    "unknown",
    "untitled",
    "untitled session",
    "untitled conversation",
    "untitled chat",
    "tbd",
    "to be determined",
    "placeholder",
    "pending",
    // The abstain token the title prompts tell the model to use for "no change".
    // It is not a title, so it must never be stored or shown.
    "keep",
    // A model that abstains in its own words instead of the exact token. Exact only:
    // "Keep alive pings" is a real title.
    "keep the current title",
    "keep current title",
    "keep it",
    "no change",
    "no changes",
    "no update",
    "unchanged",
    "not set",
    "not yet",
    "no title",
    "no title yet",
    "title unavailable",
    "no summary",
    "no goal",
    "no task",
    "no activity",
    "no recent activity",
    "nothing yet",
];

/// Deliberately few and specific. A prefix rejects every title that begins with it,
/// so "no title" and "title not" are NOT here: "No title bar on Windows" and "Title
/// not updating in swarm row" are real titles (muxreview on #4185). Anything that is
/// only absence when it is the WHOLE reply belongs in `ABSENCE_EXACT`.
const ABSENCE_PREFIXES: &[&str] = &[
    "none yet",
    "no summary yet",
    "no goal established",
    "no goal yet",
    "no goal set",
    "no task yet",
    "no activity yet",
    "nothing yet",
    "not yet established",
];

/// Lower-case, every non-alphanumeric run collapsed to one space, trimmed. So
/// `(None yet)`, `"none yet."` and `none-yet` all become `none yet`, while
/// `No-op rename threshold` becomes `no op rename threshold`.
fn absence_form(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut gap = false;
    for c in normalized(text).chars() {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// A reply that is entirely a parenthetical or bracketed note. A title is not a
/// note about the title.
///
/// Only ONE balanced pair that wraps the WHOLE text counts. `(WIP) Fix login redirect
/// (again)` and `[Windows] Fix installer crash [x64]` start and end with brackets but
/// are real titles: the first pair closes before the end (muxreview on #4185).
fn is_wrapped_note(text: &str) -> bool {
    let t = text.trim();
    let (open, close) = match (t.chars().next(), t.chars().last()) {
        (Some('('), Some(')')) => ('(', ')'),
        (Some('['), Some(']')) => ('[', ']'),
        _ => return false,
    };
    let last = t.chars().count() - 1;
    let mut depth = 0i32;
    for (i, c) in t.chars().enumerate() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            // The opening bracket closed before the end: not one wrapping pair.
            if depth <= 0 && i != last {
                return false;
            }
        }
    }
    depth == 0
}

/// A reply that LEADS with the abstain token in capitals, "KEEP — the title still
/// fits": the model said "no change" and explained itself. Case-sensitive on purpose:
/// "Keep alive pings" and "Keep the swarm summary fresh" are real titles, while an
/// upper-case KEEP is the token the prompt asked for.
fn leads_with_abstain_token(text: &str) -> bool {
    let first = text.trim().split_whitespace().next().unwrap_or("");
    first.trim_matches(|c: char| !c.is_alphanumeric()) == crate::ambient::prompt::KEEP_TOKEN
}

fn is_absence(text: &str) -> bool {
    if is_wrapped_note(text) || leads_with_abstain_token(text) {
        return true;
    }
    let form = absence_form(text);
    ABSENCE_EXACT.contains(&form.as_str())
        || ABSENCE_PREFIXES
            .iter()
            .any(|p| form == *p || form.strip_prefix(*p).is_some_and(|rest| rest.starts_with(' ')))
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
    if is_absence(text) {
        return None;
    }
    let lower = normalized(text);
    if has_any(&lower, REFUSAL_PHRASES) {
        return None;
    }
    Some(text.to_string())
}

/// Which rule refuses `raw`, for the outcome log (`ambient::outcome`). Uses the
/// stored-title bounds for "too long", since the caller's own word target is not
/// known here; a reply refused only by a tighter purpose limit reads `Other`.
pub fn rejection_reason(raw: &str) -> super::outcome::RejectReason {
    use super::outcome::RejectReason;
    let text = raw.trim();
    if text.is_empty() {
        return RejectReason::Empty;
    }
    if text.contains('\n')
        || !text.chars().any(|c| c.is_alphabetic())
        || text.chars().count() > STORED_TITLE.max_chars
        || text.split_whitespace().count() > STORED_TITLE.max_words
    {
        return RejectReason::Shape;
    }
    if is_absence(text) {
        return RejectReason::AbsencePattern;
    }
    if has_any(&normalized(text), REFUSAL_PHRASES) {
        return RejectReason::Refusal;
    }
    RejectReason::Other
}

/// Bounds for judging a title that is ALREADY stored. Generous: a title written
/// under an older word target or by an older build may be longer than today's.
pub const STORED_TITLE: Limits = Limits { max_words: 28, max_chars: 200 };

/// Is this stored value a real title? The one predicate behind accepting a reply,
/// feeding a stored title back into a prompt, and (in the frontend's port of it)
/// displaying one. A value already in a database from before the check existed,
/// such as `(none yet)`, is judged here too, so it is never fed back to the model
/// and never shown.
pub fn is_usable_title(stored: &str) -> bool {
    accept_line(stored, &STORED_TITLE).is_some()
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
    if has_any(&lower, NEXT_PROMPT_META_PHRASES) {
        return None;
    }
    if has_any(&lower, RISKY_PHRASES) {
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

    /// The shared corpus (see `title_corpus.json`): what is about the absence of a
    /// title is rejected, and a real title that merely contains such a word is not.
    #[test]
    fn the_shared_corpus_is_honoured() {
        #[derive(serde::Deserialize)]
        struct Corpus {
            rejected: Vec<String>,
            accepted: Vec<String>,
        }
        let corpus: Corpus = serde_json::from_str(include_str!("title_corpus.json")).expect("corpus parses");
        assert!(corpus.rejected.len() >= 30 && corpus.accepted.len() >= 10, "the corpus must not be hollowed out");
        for s in &corpus.rejected {
            assert_eq!(accept_line(s, &title_limits(7)), None, "must reject {s:?}");
            assert!(!is_usable_title(s), "a stored {s:?} is not a usable title");
        }
        for s in &corpus.accepted {
            assert!(accept_line(s, &title_limits(7)).is_some(), "must accept {s:?}");
            assert!(is_usable_title(s), "a stored {s:?} is a usable title");
        }
    }

    /// The two values actually found in the owner's database on 2026-10-02.
    #[test]
    fn the_values_found_in_the_wild_are_not_titles() {
        assert!(!is_usable_title("(none yet)"));
        assert!(!is_usable_title("no goal established yet"));
        assert!(is_usable_title("Develop hardening spec for swarm ambient summary quality"));
    }

    // muxreview on #4185: both of these were false rejects in the first version.
    #[test]
    fn titles_that_merely_start_with_a_risky_prefix_or_end_in_brackets_are_accepted() {
        for s in [
            "No title bar on Windows",
            "Title not updating in swarm row",
            "(WIP) Fix login redirect (again)",
            "[Windows] Fix installer crash [x64]",
            "(Draft) Review the plan",
        ] {
            assert!(accept_line(s, &title_limits(7)).is_some(), "{s:?}");
            assert!(is_usable_title(s), "{s:?}");
        }
    }

    #[test]
    fn only_one_balanced_pair_wrapping_the_whole_text_is_a_note() {
        for note in ["(none yet)", "[no summary]", "(waiting for the user's first message)", "((none yet))", "([x] pending)"] {
            assert!(is_wrapped_note(note), "{note:?}");
        }
        for title in ["(a) b (c)", "[a] b [c]", "(a) (b)", "(a", "a)", "plain"] {
            assert!(!is_wrapped_note(title), "{title:?}");
        }
    }

    // The real model returned the exact token in every abstain case when tried live, but
    // a reply that abstains in its own words must not overwrite a good title either.
    #[test]
    fn an_abstain_in_the_models_own_words_is_not_a_title() {
        for s in [
            "KEEP — the title still fits",
            "KEEP: unchanged",
            "KEEP the current title",
            "Keep current title",
            "keep the current title",
            "No change",
            "Unchanged",
            "No update",
        ] {
            assert_eq!(accept_line(s, &title_limits(7)), None, "{s:?}");
        }
        for s in ["Keep alive pings", "Keep the swarm summary fresh", "Unchanged files report fix"] {
            assert!(accept_line(s, &title_limits(7)).is_some(), "{s:?}");
        }
    }

    #[test]
    fn the_abstain_token_is_never_a_title() {
        for s in ["KEEP", "keep", "Keep.", "(KEEP)", " KEEP "] {
            assert_eq!(accept_line(s, &title_limits(7)), None, "{s:?}");
        }
    }

    #[test]
    fn whole_word_matching_keeps_real_titles_that_start_with_an_absence_word() {
        for s in ["None of the tests pass", "Untitled-tab bug", "Nothing else matters here fix", "Keep alive pings"] {
            assert!(accept_line(s, &title_limits(7)).is_some(), "{s:?}");
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
            "Wipe the disk",
        ] {
            assert_eq!(accept_next_prompt(s), None, "{s}");
        }
    }

    #[test]
    fn ordinary_commands_pass_and_over_blocking_is_deliberate() {
        // Plurals ("tokens", "secrets") are separate words and are NOT matched by
        // "token"/"secret": add them to the list if that ever matters.
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
    fn phrases_match_whole_words_only() {
        // "has an AI" contains the substring "as an ai".
        for t in ["Fix pane has an AI summary bug", "Agent was an AI wrapper", "Rewire the bias in token counts"] {
            assert_eq!(accept_line(t, &title_limits(7)).as_deref(), Some(t), "{t}");
        }
        assert_eq!(accept_line("As an AI, I have no title", &NAME), None);
        assert_eq!(accept_line("Sorry, as an AI model I can't say", &NAME), None);
        assert!(has_phrase("sorry. i cannot do that", "i cannot"));
        assert!(!has_phrase("lexi cannot", "i cannot"));
    }

    #[test]
    fn risky_words_inside_other_words_no_longer_block_a_next_prompt() {
        // "secret" used to block "secretary"; whole-word matching fixes the false
        // positive while "secrets" and "tokens" stay out of reach by design.
        assert_eq!(accept_next_prompt("Email the secretary"), Some("Email the secretary".into()));
        assert_eq!(accept_next_prompt("Print the secret"), None);
    }

    #[test]
    fn title_limits_scale_with_the_requested_word_count() {
        assert_eq!(title_limits(7).max_words, 15);
        assert_eq!(title_limits(20).max_words, 28);
    }
}
