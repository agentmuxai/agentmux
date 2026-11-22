// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a text posted for others to read (a bulletin or a channel post) must not hold besides credential words
//! (`sanitize::post_refusal_reason`): a value AgentMux knows to be a secret, and a credential word beside a value. The refusals name the rule, never the text that matched.

/// The rule a post is refused under when it holds a value in the registry the transcripts withhold from
/// ([`crate::backend::transcript::withhold`]): the registry's own matcher, in every spelling it knows.
pub const KNOWN_SECRET: &str = "a known secret value";

/// The rule a post is refused under when a credential word and a value-shaped string stand within a few words of each other.
pub const WORD_THEN_VALUE: &str = "a credential word followed by a value";

/// What a refusal adds for the rule it names, so the poster knows how to post the text anyway when it is not a secret.
pub fn help(rule: &str) -> &'static str {
    match rule {
        KNOWN_SECRET => " Leave the value out of the text (name what it is, do not paste it).",
        WORD_THEN_VALUE => " If it is not a secret, rephrase it: leave out the long value-like string (say what it is called instead of pasting it), or drop the word that names a credential.",
        _ => "",
    }
}

/// Refusals by rule since AgentMux started, so a rule that refuses ordinary text can be seen in the counts.
static REFUSALS: std::sync::Mutex<Vec<(&'static str, u64)>> = std::sync::Mutex::new(Vec::new());

/// Counts one refusal under `rule`, and logs it (the rule only, never the text).
pub fn count_refusal(rule: &'static str) {
    let mut counts = REFUSALS.lock().unwrap_or_else(|e| e.into_inner());
    match counts.iter_mut().find(|(r, _)| *r == rule) {
        Some((_, n)) => *n += 1,
        None => counts.push((rule, 1)),
    }
    tracing::info!(rule, "a post was refused");
}

/// The refusals counted so far, by rule.
pub fn refusal_counts() -> Vec<(&'static str, u64)> {
    REFUSALS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The nouns that name a credential, as a whole word or as a part of a joined name (`access_token`, `client-secret`,
/// `apiKey`).
const CREDENTIAL_NOUNS: &[&str] = &["token", "tokens", "secret", "secrets", "key", "keys", "password", "passwords", "credential", "credentials", "pat", "apikey", "passphrase", "passphrases", "passwd", "pwd", "bearer"];

/// Starts of the values the common providers issue.
const VALUE_PREFIXES: &[&str] = &["ghp_", "gho_", "ghs_", "ghu_", "ghr_", "github_pat_", "xox", "sk-", "sk_", "akia", "asia", "aiza", "ya29.", "eyj", "glpat-", "npm_", "pypi-"];

/// How many words before or after a value a credential noun may stand.
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

/// A letter a look-alike character stands for, with its case: the Cyrillic and Greek letters that copy Latin ones.
pub(super) fn letter_lookalike(c: char) -> char {
    match c {
        'а' => 'a', 'е' | 'є' => 'e', 'о' | 'ο' => 'o', 'р' | 'ρ' => 'p', 'с' | 'ϲ' => 'c', 'у' => 'y', 'х' | 'χ' => 'x',
        'і' | 'ї' | 'ι' => 'i', 'ѕ' => 's', 'ј' => 'j', 'ԁ' => 'd', 'α' => 'a', 'ν' => 'v', 'τ' => 't', 'κ' => 'k',
        'А' | 'Α' => 'A', 'В' | 'Β' => 'B', 'Е' | 'Ε' => 'E', 'К' | 'Κ' => 'K', 'М' | 'Μ' => 'M', 'Н' | 'Η' => 'H',
        'О' | 'Ο' => 'O', 'Р' | 'Ρ' => 'P', 'С' => 'C', 'Т' | 'Τ' => 'T', 'Х' | 'Χ' => 'X', 'І' | 'Ι' => 'I', 'Ѕ' => 'S',
        'Ј' => 'J', 'Ү' => 'Y', 'Ζ' => 'Z',
        other => other,
    }
}

/// `text` as a reader takes it: invisible characters dropped, fullwidth forms as ASCII, look-alike letters as the
/// letters they imitate, with the case kept (a registered value is matched with its case).
fn as_read(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{00AD}' | '\u{180E}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{E0000}'..='\u{E007F}'))
        .map(|c| match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => letter_lookalike(c),
        })
        .collect()
}

/// The rule `text` is refused under, or `None`: it holds a value AgentMux knows to be a secret (in any of its
/// spellings, also when invisible characters or look-alike letters are mixed in), or a credential word and a
/// value-shaped string stand within a few words of each other, either first.
pub fn refusal_rule(text: &str) -> Option<&'static str> {
    let read = as_read(text);
    if crate::backend::transcript::withhold::holds(text) || crate::backend::transcript::withhold::holds(&read) {
        return Some(KNOWN_SECRET);
    }
    let words: Vec<&str> = read
        .split(|c: char| c.is_whitespace() || matches!(c, '=' | ':' | '"' | '\'' | ',' | ';' | '{' | '}' | '(' | ')' | '[' | ']' | '`' | '<' | '>'))
        .filter(|w| !w.is_empty())
        .collect();
    for (i, word) in words.iter().enumerate() {
        if !value_shaped(word) {
            continue;
        }
        let from = i.saturating_sub(REACH);
        let to = (i + 1 + REACH).min(words.len());
        if words[from..i].iter().chain(words[i + 1..to].iter()).any(|near| names_a_credential(near)) {
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

    #[test]
    fn a_registered_value_is_still_found_with_an_invisible_character_or_a_look_alike_letter_in_it() {
        let canary = "Canary-Pass7731xQ-fold";
        withhold::register(canary);
        let with_zero_width = canary.replacen("Pass", "Pa\u{200B}ss", 1);
        let with_cyrillic_a = canary.replacen("Canary", "C\u{0430}nary", 1);
        let fullwidth: String = canary.chars().map(|c| if c.is_ascii_alphanumeric() { char::from_u32(c as u32 + 0xFEE0).unwrap() } else { c }).collect();
        for text in [with_zero_width, with_cyrillic_a, fullwidth] {
            assert_eq!(refusal_rule(&format!("note: {text} ok")), Some(KNOWN_SECRET), "{text}");
        }
    }

    #[test]
    fn a_value_first_and_the_newer_nouns_are_refused() {
        let value = ["ghp", "_aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5"].concat();
        for text in [
            format!("{value} is my token"),
            format!("{value} <- the key"),
            format!("passphrase: {value}"),
            format!("pwd={value}"),
            format!("Bearer {value}"),
            format!("the passwd is {value}"),
        ] {
            assert_eq!(refusal_rule(&text), Some(WORD_THEN_VALUE), "{text}");
        }
        // A long value far from any credential word, and a credential word far from any value, are not enough.
        assert_eq!(refusal_rule(&format!("the build id is {} and nothing else follows here", ["d3b0", "7384d9a04c9b8a6d1f2e3c4b5a697881"].concat())), None);
    }

    #[test]
    fn a_refusal_says_how_to_post_the_text_anyway_and_is_counted_by_rule() {
        assert!(help(WORD_THEN_VALUE).contains("rephrase") && help(KNOWN_SECRET).contains("Leave the value out"));
        assert_eq!(help("password"), "");
        let before = refusal_counts().into_iter().find(|(r, _)| *r == WORD_THEN_VALUE).map_or(0, |(_, n)| n);
        count_refusal(WORD_THEN_VALUE);
        let after = refusal_counts().into_iter().find(|(r, _)| *r == WORD_THEN_VALUE).map_or(0, |(_, n)| n);
        assert_eq!(after, before + 1);
    }
}
