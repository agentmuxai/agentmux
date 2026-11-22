// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a text posted for others to read (a bulletin or a channel post) must not hold besides credential words
//! (`sanitize::post_refusal_reason`): a value AgentMux knows to be a secret, and a credential word with a value
//! after it. The refusals name the rule, never the text that matched.

/// The rule a post is refused under when it holds a value in the registry the transcripts withhold from
/// ([`crate::backend::transcript::withhold`]): the registry's own matcher, in every spelling it knows.
pub const KNOWN_SECRET: &str = "a known secret value";

/// The rule a post is refused under when a credential word has a value-shaped string after it.
pub const WORD_THEN_VALUE: &str = "a credential word followed by a value";

/// The nouns that name a credential, as a whole word or as a part of a joined name (`access_token`, `client-secret`,
/// `apiKey`).
const CREDENTIAL_NOUNS: &[&str] = &["token", "tokens", "secret", "secrets", "key", "keys", "password", "passwords", "credential", "credentials", "pat", "apikey"];

/// Starts of the values the common providers issue.
const VALUE_PREFIXES: &[&str] = &["ghp_", "gho_", "ghs_", "ghu_", "ghr_", "github_pat_", "xox", "sk-", "sk_", "akia", "asia", "aiza", "ya29.", "eyj", "glpat-", "npm_", "pypi-"];

/// How many words before a value a credential noun may stand.
const REACH: usize = 3;

/// Whether `word` looks like an issued value rather than a word: a provider's prefix, or a long run of letters and
/// digits (with the signs such values use) that holds both.
fn value_shaped(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    if word.len() >= 12 && VALUE_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    let run = word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.'));
    run && word.len() >= 20 && word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(|c| c.is_ascii_alphabetic())
}

/// The parts of a joined name: split at underscores, hyphens and a lower-case letter followed by a capital.
fn parts(word: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut part = String::new();
    let mut prev_lower = false;
    for c in word.chars() {
        if c == '_' || c == '-' {
            if !part.is_empty() {
                out.push(std::mem::take(&mut part));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !part.is_empty() {
            out.push(std::mem::take(&mut part));
        }
        prev_lower = c.is_ascii_lowercase();
        part.push(c.to_ascii_lowercase());
    }
    if !part.is_empty() {
        out.push(part);
    }
    out
}

/// Whether `word` names a credential, as a word or as a part of a joined name.
fn names_a_credential(word: &str) -> bool {
    parts(word).iter().any(|p| CREDENTIAL_NOUNS.contains(&p.as_str()))
}

/// The rule `text` is refused under, or `None`: it holds a value AgentMux knows to be a secret (in any of its
/// spellings), or a credential word with a value-shaped string within a few words after it.
pub fn refusal_rule(text: &str) -> Option<&'static str> {
    if crate::backend::transcript::withhold::holds(text) {
        return Some(KNOWN_SECRET);
    }
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '=' | ':' | '"' | '\'' | ',' | ';' | '{' | '}' | '(' | ')' | '[' | ']' | '`' | '<' | '>'))
        .filter(|w| !w.is_empty())
        .collect();
    for (i, word) in words.iter().enumerate() {
        if value_shaped(word) && words[i.saturating_sub(REACH)..i].iter().any(|before| names_a_credential(before)) {
            return Some(WORD_THEN_VALUE);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::transcript::withhold;

    #[test]
    fn a_value_the_registry_holds_is_refused_in_every_spelling_and_never_named() {
        let canary = "canary-Value_7731xQ-post";
        withhold::register(canary);
        for form in crate::backend::vault::scrub::forms(canary) {
            let text = format!("please note that {form} is what you need");
            assert_eq!(refusal_rule(&text), Some(KNOWN_SECRET), "{form}");
        }
        assert!(!KNOWN_SECRET.contains("canary") && !WORD_THEN_VALUE.contains("canary"));
        assert_eq!(refusal_rule("a value that is not registered, and ordinary text"), None);
    }

    #[test]
    fn joined_credential_names_are_refused_when_a_value_follows() {
        // Built from parts: the text is a test value, and no line of source should look like an issued one.
        let value = ["ghp", "_aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5"].concat();
        let slack = ["xo", "xb-1234567890-abcdefghijkl"].concat();
        for text in [
            format!("access_token={value}"),
            format!("refresh_token: {value}"),
            format!("api-key {value}"),
            format!("client_secret = \"{value}\""),
            format!("apiKey: {value}"),
            format!("AccessToken {value}"),
            format!("a slack token {slack}"),
            format!("the discord token is {value}"),
        ] {
            assert_eq!(refusal_rule(&text), Some(WORD_THEN_VALUE), "{text}");
        }
    }

    #[test]
    fn ordinary_text_about_usage_units_and_names_without_a_value_is_let_through() {
        for text in [
            "Record ways to save tokens: a kind named security is for gaps.",
            "the access_token field is renamed to accessToken in the new release",
            "rotate the api-key every quarter; the key name stays the same",
            "It cost 40k tokens and seven turns on 2026-11-21T19:59:07Z",
            "the secret store is in the private repo (a5af/shared-infrastructure)",
        ] {
            assert_eq!(refusal_rule(text), None, "{text}");
        }
    }
}
