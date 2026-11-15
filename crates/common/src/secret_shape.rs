// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The shape of a secret in text a page held, for the places that hand page text to an agent (#47628, #47656).
//!
//! The Browser reads (`agentmux-cef`'s `hidden_secrets`) withhold hidden text that has this shape, and so does
//! every string that comes back with an answer: a click's name in the waiting notice, a line of the pane's
//! Log, the state an action reports. One rule, here, so they agree.
use crate::shown_once::has_shown_once;

/// What an agent reads in place of withheld text.
pub const WITHHELD: &str = "[hidden text withheld: it may be a secret; use Browser capture to save it]";

/// What an agent reads in place of a secret the page shows in plain sight, in a row named like one: it is
/// there, and `Browser` `capture` saves it, but its text stays out of the agent's context.
pub const UNREAD: &str = "[unread secret: use Browser capture to save it]";

/// Prefixes of keys, tokens and signing secrets that are short enough to miss the one-time shape.
const KEY_PREFIXES: &[&str] = &["sk_", "rk_", "pk_", "whsec_", "ghp_", "gho_", "ghs_", "github_pat_", "xoxb-", "xoxp-", "xoxa-", "eyj"];

/// A cloud access key id: four capital letters and sixteen capitals or digits, as one word.
fn is_access_key_id(t: &str) -> bool {
    t.len() == 20 && (t.starts_with("AKIA") || t.starts_with("ASIA")) && t.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Whether `text` holds something shaped like a secret: what a page shows once, a key with a known prefix, an
/// access key id or a PEM header.
pub fn has_key_shape(text: &str) -> bool {
    has_shown_once(text)
        || text.to_ascii_lowercase().contains("-----begin")
        || text.split_whitespace().any(|t| {
            // A word in quotes or beside a comma is still the word.
            let t = t.trim_matches(|c: char| !c.is_alphanumeric());
            let lower = t.to_ascii_lowercase();
            is_access_key_id(t) || (t.len() >= 12 && KEY_PREFIXES.iter().any(|p| lower.starts_with(p)))
        })
}

/// One long word with a digit and a letter in it, which words are not: an odd-shaped value (an id, a key a
/// page makes up).
pub fn token_like(text: &str) -> bool {
    let t = text.trim();
    t.chars().count() >= 12
        && !t.chars().any(char::is_whitespace)
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().any(char::is_alphabetic)
        && t.chars().all(|c| c.is_alphanumeric() || "-_=+/.:".contains(c))
}

/// `text` with every word that has a secret's shape replaced by [`WITHHELD`], and the rest as it was (line
/// breaks and spacing too); the text itself when no word has one.
pub fn scrub(text: &str) -> String {
    if !has_key_shape(text) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut flush = |out: &mut String, word: &mut String| {
        out.push_str(if has_key_shape(word) { WITHHELD } else { word });
        word.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut out, &mut word);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut out, &mut word);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_word_is_replaced_and_the_spacing_around_it_is_kept() {
        assert_eq!(scrub("Copy sk_live_abcdefgh12 now"), format!("Copy {WITHHELD} now"));
        assert_eq!(scrub("a\nghp_abcdefghij12\n\nb"), format!("a\n{WITHHELD}\n\nb"));
        assert_eq!(scrub("Account settings"), "Account settings");
        assert_eq!(scrub("Click \"Copy sk_live_abcdefgh12\" on https://a.test"), format!("Click \"Copy {WITHHELD} on https://a.test"));
    }

    #[test]
    fn ordinary_words_and_odd_tokens_are_told_apart() {
        for word in ["Asia/Kolkata", "Asiatic", "keyboard", "2026-11-14"] {
            assert!(!has_key_shape(word), "{word}");
        }
        assert!(token_like("q7Zp-93xaLm2Rt") && !token_like("At least eight characters") && !token_like("Reveal"));
    }
}
