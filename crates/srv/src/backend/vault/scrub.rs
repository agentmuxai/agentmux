// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Takes a Vault value out of a command's output before anything reads it
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §7.2).
//!
//! **A guard, not a boundary.** A command the agent writes can transform the
//! value and print it, or send it anywhere. What limits that is the person
//! seeing the command before an entry is first used. The scrubber catches the
//! ordinary ways a tool echoes a key it was given: the value itself, and its
//! base64, URL-encoded and hex forms. For a value of several lines (a private
//! key), each line is scrubbed too, since output is read line by line.

use base64::Engine as _;

/// Shorter values are refused as command secrets: scrubbing "123" would
/// mangle every output that has it.
pub const MIN_LEN: usize = 4;

/// The shortest line of a multi-line value that is scrubbed on its own.
const MIN_LINE: usize = 6;

pub struct Scrubber {
    /// (what to find, what to put there), longest first so a form that contains
    /// another is replaced whole.
    needles: Vec<(String, String)>,
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn forms(value: &str) -> Vec<String> {
    let engines = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    let mut out = vec![value.to_string(), percent_encode(value), percent_encode(value).to_lowercase()];
    out.extend(engines.iter().map(|e| e.encode(value)));
    let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
    out.push(hex.to_uppercase());
    out.push(hex);
    out.extend(json_forms(value));
    out
}

/// A value as a JSON string holds it, without the quotes: escaped as serde_json
/// writes it, with `/` escaped too, and with everything outside ASCII as `\uXXXX`.
/// A tool that prints JSON prints a value with a quote, a backslash or a newline
/// in one of these forms, never as is.
fn json_forms(value: &str) -> Vec<String> {
    let quoted = serde_json::to_string(value).unwrap_or_default();
    let Some(plain) = quoted.strip_prefix('"').and_then(|q| q.strip_suffix('"')) else {
        return Vec::new();
    };
    let ascii: String = plain
        .chars()
        .flat_map(|c| -> Vec<char> {
            if c.is_ascii() {
                vec![c]
            } else {
                c.encode_utf16(&mut [0; 2]).iter().flat_map(|u| format!("\\u{u:04x}").chars().collect::<Vec<_>>()).collect()
            }
        })
        .collect();
    vec![plain.to_string(), plain.replace('/', "\\/"), ascii]
}

impl Scrubber {
    /// `secrets` are (entry id, value).
    pub fn new(secrets: &[(String, String)]) -> Self {
        let mut needles: Vec<(String, String)> = Vec::new();
        for (id, value) in secrets {
            let mark = format!("[vault:{id}]");
            let mut parts = vec![value.clone()];
            if value.contains('\n') {
                // Output arrives line by line, so a multi-line value never appears whole.
                parts.extend(value.lines().map(str::trim).filter(|l| l.len() >= MIN_LINE).map(str::to_string));
            }
            for part in parts {
                for form in forms(&part) {
                    if form.len() >= MIN_LEN && !needles.iter().any(|(n, _)| *n == form) {
                        needles.push((form, mark.clone()));
                    }
                }
            }
        }
        needles.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Self { needles }
    }

    pub fn scrub<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        let mut out = std::borrow::Cow::Borrowed(text);
        for (needle, mark) in &self.needles {
            if out.contains(needle.as_str()) {
                out = std::borrow::Cow::Owned(out.replace(needle.as_str(), mark));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrubber(value: &str) -> Scrubber {
        Scrubber::new(&[("api-key-stripe".to_string(), value.to_string())])
    }

    #[test]
    fn the_value_and_its_ordinary_encodings_are_replaced_by_the_entrys_name() {
        let s = scrubber("sk_live_a/b+c=1234");
        let mark = "[vault:api-key-stripe]";
        assert_eq!(s.scrub("key is sk_live_a/b+c=1234 ok"), format!("key is {mark} ok"));
        let b64 = base64::engine::general_purpose::STANDARD.encode("sk_live_a/b+c=1234");
        assert_eq!(s.scrub(&format!("Authorization: Basic {b64}")), format!("Authorization: Basic {mark}"));
        let url = percent_encode("sk_live_a/b+c=1234");
        assert_eq!(s.scrub(&format!("?key={url}&x=1")), format!("?key={mark}&x=1"));
        let hex: String = "sk_live_a/b+c=1234".bytes().map(|b| format!("{b:02x}")).collect();
        assert_eq!(s.scrub(&hex.to_uppercase()), mark);
        assert_eq!(s.scrub(&hex), mark);
        let nopad = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("sk_live_a/b+c=1234");
        assert_eq!(s.scrub(&nopad), mark);
    }

    #[test]
    fn output_without_the_value_is_untouched() {
        let s = scrubber("hunter2-secret");
        assert_eq!(s.scrub("all fine here"), "all fine here");
        assert!(matches!(s.scrub("all fine here"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn each_line_of_a_multi_line_value_is_scrubbed() {
        let key = "-----BEGIN KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nabcdEFGHijklMNOP\n-----END KEY-----";
        let s = scrubber(key);
        assert_eq!(s.scrub("MIIEvQIBADANBgkqhkiG9w0BAQEFAASC"), "[vault:api-key-stripe]");
        assert_eq!(s.scrub("  abcdEFGHijklMNOP  "), "  [vault:api-key-stripe]  ");
        assert_eq!(s.scrub("-----BEGIN KEY-----"), "[vault:api-key-stripe]");
    }

    #[test]
    fn a_longer_form_is_replaced_whole_before_a_shorter_one_inside_it() {
        let s = Scrubber::new(&[("a".to_string(), "abcd1234".to_string()), ("b".to_string(), "abcd1234-and-more".to_string())]);
        assert_eq!(s.scrub("x abcd1234-and-more y"), "x [vault:b] y");
        assert_eq!(s.scrub("x abcd1234 y"), "x [vault:a] y");
    }
}
