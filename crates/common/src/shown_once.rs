// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A secret a web page shows once: a new OAuth client secret, an app registration's
//! client secret, a key next to a Copy button
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.1).
//!
//! The shape alone would hide too much (a long identifier, a path), so callers use it
//! only where a page shows such a value: a read-only field, a `code`, `pre`, `kbd` or
//! `samp` element, or the row or container of a button named like "Copy". There, a
//! value with the shape is hidden from the agent, which saves it with `Browser`
//! `capture` instead of reading it.

/// What the agent sees in place of the value.
pub const SHOWN_ONCE: &str = "[secret: shown once; use Browser capture to save it]";

const MIN: usize = 20;
const MAX: usize = 512;

/// `8-4-4-4-12` hex, with or without braces: an Azure client or tenant id, which
/// isn't secret.
fn is_guid(s: &str) -> bool {
    let s = s.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(s);
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5 && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("://") || lower.starts_with("www.") || lower.starts_with("mailto:")
}

/// Whether one whitespace-free piece of text has the shape of a one-time secret:
/// 20 to 512 characters, at least two of lowercase, uppercase, digits and symbols,
/// and not a GUID, pure hex (a commit SHA, which agents need) or a URL.
pub fn looks_shown_once(token: &str) -> bool {
    let n = token.chars().count();
    if !(MIN..=MAX).contains(&n) || token.chars().any(char::is_whitespace) {
        return false;
    }
    if is_guid(token) || token.bytes().all(|b| b.is_ascii_hexdigit()) || is_url(token) {
        return false;
    }
    let classes = [
        token.chars().any(|c| c.is_lowercase()),
        token.chars().any(|c| c.is_uppercase()),
        token.chars().any(|c| c.is_ascii_digit()),
        token.chars().any(|c| !c.is_alphanumeric()),
    ];
    classes.iter().filter(|c| **c).count() >= 2
}

/// Whether `text` holds a value with that shape.
pub fn has_shown_once(text: &str) -> bool {
    text.split_whitespace().any(looks_shown_once)
}

/// `text` with every value of that shape replaced by [`SHOWN_ONCE`]; the rest, and
/// its spacing, as it was.
pub fn mask(text: &str) -> String {
    if !has_shown_once(text) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if looks_shown_once(word) {
            out.push_str(SHOWN_ONCE);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_one_time_secrets_have_the_shape() {
        for s in [
            // A GitHub OAuth app client secret is 40 hex: not masked by shape (see below), but
            // a GitHub App's or an Azure client secret is.
            "Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "tok_Example_AbCdEfGhIjKlMnOpQr2",
            "AbCdEfGhIjKlMnOpQrStUvWx",
        ] {
            assert!(looks_shown_once(s), "{s}");
        }
    }

    #[test]
    fn ids_shas_urls_and_words_are_left_alone() {
        for s in [
            "0f4c2a1e-9b7d-4e3a-8c6f-1a2b3c4d5e6f",
            "{0F4C2A1E-9B7D-4E3A-8C6F-1A2B3C4D5E6F}",
            "c58ebaa8e2f1b0d9c7a6e5f4d3c2b1a098765432",
            "https://github.com/settings/applications/new",
            "www.example.com/a/very/long/path/here",
            "internationalization",
            "short-Secret-1",
            "this is twenty-plus chars but has spaces",
        ] {
            assert!(!looks_shown_once(s), "{s}");
        }
        assert!(!looks_shown_once(&"aB3".repeat(200)), "longer than 512");
    }

    #[test]
    fn mask_replaces_only_the_secret_and_keeps_the_rest() {
        let text = "Client secret:  Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123 (copy it now)";
        assert_eq!(mask(text), format!("Client secret:  {SHOWN_ONCE} (copy it now)"));
        assert_eq!(mask("nothing secret here"), "nothing secret here");
        assert!(has_shown_once(text) && !has_shown_once("app id 0f4c2a1e-9b7d-4e3a-8c6f-1a2b3c4d5e6f"));
    }
}
