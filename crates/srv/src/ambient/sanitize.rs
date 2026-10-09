// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Strips the formatting a model wraps around its whole reply. Whether what is
//! left is usable is the reply format's question (`reply`), then the purpose's own
//! check (`validate`).

/// Removes, repeatedly, then trims: a code fence wrapping the whole text
/// (optionally with a language tag), one matching pair of quotes wrapping it, and
/// a conversational opener ("Yeah, let's…"). A reply left with nothing but
/// backticks (an empty fence) becomes "", which callers already treat as no
/// reply.
///
/// Run on the whole reply by the CLI, and again on the text after `ANSWER:` by
/// `reply::parse`, which the first pass can't reach past the prefix.
pub fn sanitize_ambient_text(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for _ in 0..4 {
        let before = s.clone();
        if let Some(inner) = strip_wrapping_fence(&s) {
            s = inner;
        }
        if let Some(inner) = strip_wrapping_quotes(&s) {
            s = inner;
        }
        if let Some(rest) = strip_conversational_preamble(&s) {
            s = rest;
        }
        if s == before {
            break;
        }
    }
    if !s.is_empty() && s.chars().all(|c| c == '`') {
        s.clear();
    }
    s
}

/// Strip a wrapping code fence (triple-backtick, optionally with a language
/// tag on the opening line, or a single inline backtick) if the *entire*
/// string is wrapped — a fence-like substring embedded mid-sentence is left
/// alone. Returns the un-fenced inner text, trimmed.
fn strip_wrapping_fence(s: &str) -> Option<String> {
    let s = s.trim();
    for fence in ["```", "`"] {
        if s.len() >= fence.len() * 2 && s.starts_with(fence) && s.ends_with(fence) {
            let mut inner = &s[fence.len()..s.len() - fence.len()];
            if fence == "```" {
                if let Some(nl) = inner.find('\n') {
                    inner = &inner[nl + 1..];
                }
            }
            return Some(inner.trim().to_string());
        }
    }
    None
}

/// Strip ONE pair of quotes that wraps the whole string. A quote at only one end,
/// or a pair that closes before the end (`"a" and "b"`), is part of the text.
fn strip_wrapping_quotes(s: &str) -> Option<String> {
    const PAIRS: &[(char, char)] = &[('"', '"'), ('\'', '\''), ('\u{201C}', '\u{201D}'), ('\u{2018}', '\u{2019}')];
    let s = s.trim();
    for &(open, close) in PAIRS {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.strip_suffix(close)) {
            if !inner.contains(open) && !inner.contains(close) {
                return Some(inner.trim().to_string());
            }
        }
    }
    None
}

/// Conversational-filler openers a model reaches for despite being told to
/// respond with a bare, direct instruction — e.g. "Yeah, let's fix the
/// bug" instead of "Fix the bug". Matched case-insensitively against the
/// very start of the string only (a "let's" appearing mid-sentence is left
/// alone — this strips openers, not arbitrary word choice). Longer/more
/// specific entries first so e.g. "let's go ahead and " matches whole
/// rather than the shorter "let's " eating only part of it.
const CONVERSATIONAL_PREAMBLES: &[&str] = &[
    "let's go ahead and ",
    "lets go ahead and ",
    "yeah, let's ",
    "yeah let's ",
    "yeah, lets ",
    "yeah lets ",
    "sure, let's ",
    "sure let's ",
    "ok, let's ",
    "okay, let's ",
    "alright, let's ",
    "go ahead and ",
    "let's ",
    "lets ",
    "sure, i'll ",
    "sure, i will ",
    "sure, ",
    "yeah, ",
    "yeah ",
    "ok, ",
    "okay, ",
    "alright, ",
    "i'll ",
    "i will ",
    "i should ",
    "we should ",
    "next up, ",
    "next, ",
];

/// Strip one leading conversational-filler opener (see
/// `CONVERSATIONAL_PREAMBLES`) and re-capitalize the new first letter, so
/// "Yeah, let's debug the bug" becomes "Debug the bug" — matching the
/// direct-instruction register every ambient-call prompt asks for. Returns
/// `None` if no known opener matches (left alone rather than guessed at —
/// this list is deliberately not exhaustive NLP-style filler detection,
/// just the handful of openers a model actually reaches for here).
fn strip_conversational_preamble(s: &str) -> Option<String> {
    for prefix in CONVERSATIONAL_PREAMBLES {
        if let Some(byte_len) = case_insensitive_prefix_byte_len(s, prefix) {
            let rest = &s[byte_len..];
            let mut chars = rest.chars();
            return Some(match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            });
        }
    }
    None
}

/// Byte length in `s` of a prefix that case-insensitively matches `prefix`
/// (always plain ASCII lowercase), or `None` if `s` doesn't start with it.
/// Walks `s`'s own char boundaries rather than slicing by an offset computed
/// against a separately-lowercased copy of `s` — a char's lowercase mapping
/// can change UTF-8 byte length (e.g. the Kelvin sign 'K' -> 'k') or even
/// character count (e.g. Turkish 'İ' -> "i̇"), which would otherwise misalign
/// the offset or land it off a char boundary and panic on slice.
fn case_insensitive_prefix_byte_len(s: &str, prefix: &str) -> Option<usize> {
    let mut prefix_chars = prefix.chars().peekable();
    for (byte_idx, c) in s.char_indices() {
        if prefix_chars.peek().is_none() {
            return Some(byte_idx);
        }
        for lc in c.to_lowercase() {
            if prefix_chars.next_if_eq(&lc).is_none() {
                return None;
            }
        }
    }
    if prefix_chars.peek().is_none() {
        Some(s.len())
    } else {
        None
    }
}

#[cfg(test)]
mod sanitize_ambient_text_tests {
    use super::*;

    #[test]
    fn passes_plain_text_through_unchanged() {
        assert_eq!(sanitize_ambient_text("Run the tests"), "Run the tests");
        assert_eq!(sanitize_ambient_text("ANSWER: Run the tests"), "ANSWER: Run the tests");
    }

    #[test]
    fn strips_a_triple_backtick_fence() {
        assert_eq!(sanitize_ambient_text("```\nRun the tests\n```"), "Run the tests");
    }

    #[test]
    fn strips_a_fence_with_a_language_tag() {
        assert_eq!(sanitize_ambient_text("```text\nRun the tests\n```"), "Run the tests");
    }

    #[test]
    fn empty_fence_collapses_to_empty_string() {
        assert_eq!(sanitize_ambient_text("```\n```"), "");
        assert_eq!(sanitize_ambient_text("``````"), "");
    }

    #[test]
    fn strips_wrapping_single_backticks() {
        assert_eq!(sanitize_ambient_text("`Run the tests`"), "Run the tests");
    }

    #[test]
    fn strips_one_wrapping_pair_of_quotes() {
        assert_eq!(sanitize_ambient_text("\"Run the tests\""), "Run the tests");
        assert_eq!(sanitize_ambient_text("'Run the tests'"), "Run the tests");
        assert_eq!(sanitize_ambient_text("\u{201C}Run the tests\u{201D}"), "Run the tests");
    }

    /// Quotes were trimmed from each end independently, so a reply ending in a
    /// quoted name lost its closing quote.
    #[test]
    fn a_quote_at_only_one_end_is_text() {
        assert_eq!(sanitize_ambient_text("Rename it to \"foo\""), "Rename it to \"foo\"");
        assert_eq!(sanitize_ambient_text("\"foo\" is the new name"), "\"foo\" is the new name");
        assert_eq!(sanitize_ambient_text("\"foo\" and \"bar\""), "\"foo\" and \"bar\"");
    }

    #[test]
    fn strips_nested_fence_and_quotes() {
        assert_eq!(sanitize_ambient_text("```\n\"Run the tests\"\n```"), "Run the tests");
    }

    #[test]
    fn leaves_an_embedded_fence_like_substring_alone() {
        let s = "Run `npm test` next";
        assert_eq!(sanitize_ambient_text(s), s);
    }

    #[test]
    fn lone_backticks_with_no_content_collapse_to_empty() {
        assert_eq!(sanitize_ambient_text("```"), "");
    }

    #[test]
    fn strips_yeah_lets_preamble() {
        assert_eq!(
            sanitize_ambient_text("Yeah, let's debug the blank preview bug next"),
            "Debug the blank preview bug next"
        );
        assert_eq!(sanitize_ambient_text("Yeah let's fix the login bug"), "Fix the login bug");
    }

    #[test]
    fn strips_go_ahead_and_preamble() {
        assert_eq!(
            sanitize_ambient_text("Go ahead and add tests for the parser"),
            "Add tests for the parser"
        );
    }

    #[test]
    fn strips_sure_ok_alright_preamble() {
        assert_eq!(sanitize_ambient_text("Sure, fix the typo"), "Fix the typo");
        assert_eq!(sanitize_ambient_text("OK, run the tests"), "Run the tests");
        assert_eq!(sanitize_ambient_text("Alright, let's ship it"), "Ship it");
    }

    #[test]
    fn strips_preamble_from_inside_quotes_and_fences() {
        assert_eq!(sanitize_ambient_text("\"Yeah, let's fix the bug\""), "Fix the bug");
        assert_eq!(sanitize_ambient_text("```\nYeah, let's fix the bug\n```"), "Fix the bug");
    }

    #[test]
    fn leaves_a_mid_sentence_lets_alone() {
        let s = "Check whether the retry logic still lets errors through";
        assert_eq!(sanitize_ambient_text(s), s);
    }

    #[test]
    fn a_filler_word_with_no_trailing_content_is_left_alone() {
        // "Yeah," alone (no instruction after it) doesn't match any
        // CONVERSATIONAL_PREAMBLES entry — they all require trailing
        // content after the opener, matching the "strip openers, not
        // guess at degenerate whole-string filler" scope this function
        // documents.
        assert_eq!(sanitize_ambient_text("Yeah,"), "Yeah,");
    }

    #[test]
    fn preamble_strip_handles_byte_length_changing_case_folding() {
        // U+212A KELVIN SIGN lowercases to ASCII 'k' (3 bytes -> 1 byte). If the
        // preamble strip computed its slice offset from a separately-lowercased
        // copy of the string instead of walking the original string's own char
        // boundaries, this would misalign the slice (or panic on a non-char-
        // boundary index) instead of matching "ok, " normally.
        let s = "O\u{212A}, run the tests";
        assert_eq!(sanitize_ambient_text(s), "Run the tests");
    }

    #[test]
    fn preamble_strip_handles_char_count_changing_case_folding() {
        // U+0130 LATIN CAPITAL LETTER I WITH DOT ABOVE lowercases to "i̇" (2
        // chars) under Rust's default Unicode case folding. This does not
        // match any configured preamble, but must not panic or corrupt the
        // string while failing to match.
        let s = "\u{0130} think we should refactor this";
        assert_eq!(sanitize_ambient_text(s), s);
    }
}
