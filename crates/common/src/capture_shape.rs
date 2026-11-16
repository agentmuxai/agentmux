// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shape a `Browser` `capture` can ask a value to have (#47629).
//!
//! A capture of text with no ref of its own chooses the value among the text around it, and nothing is
//! saved if it is not the shape the agent said. A guard that answers yes or no about a value the agent
//! cannot read is a question about the value, so the shapes are a closed list, never an expression the
//! agent writes: a short `prefix` (at most [`MAX_PREFIX`] characters), one of the [`NAMES`] of a kind of
//! secret, and a length range. The host (to choose among the words of a row) and srv (before anything is
//! stored) use the same rule from here, and srv budgets how many shaped captures an agent may make
//! (`browser_capture_shape.rs`).

use serde::{Deserialize, Serialize};

/// The longest prefix taken: enough to tell a kind of key from its neighbours, too little to read a value out.
pub const MAX_PREFIX: usize = 12;
/// The longest value a range can allow.
pub const MAX_LENGTH: u32 = 16 * 1024;

/// The named shapes, as the agent is told them. `digits:N` is N digits, 1 to 32 (a six-digit code).
pub const NAMES: &[&str] =
    &["stripe-secret", "stripe-publishable", "stripe-webhook", "aws-access-key", "aws-secret-key", "github-token", "slack-token", "jwt", "uuid", "hex", "base64", "alnum", "digits:N"];

fn all(text: &str, ok: impl Fn(char) -> bool) -> bool {
    !text.is_empty() && text.chars().all(ok)
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// `text` after one of `prefixes`, when it has one.
fn after<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| text.strip_prefix(p))
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5 && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether `value` is the kind of secret `name` says; `None` when there is no such name.
fn is_named(name: &str, value: &str) -> Option<bool> {
    let rest_alnum = |rest: Option<&str>, min: usize| rest.is_some_and(|r| r.len() >= min && all(r, alnum));
    Some(match name {
        "stripe-secret" => rest_alnum(after(value, &[concat!("sk_live", "_"), "sk_test_", "rk_live_", "rk_test_"]), 16),
        "stripe-publishable" => rest_alnum(after(value, &["pk_live_", "pk_test_"]), 16),
        "stripe-webhook" => rest_alnum(after(value, &["whsec_"]), 16),
        "aws-access-key" => value.len() == 20 && after(value, &["AKIA", "ASIA"]).is_some_and(|r| all(r, |c| c.is_ascii_uppercase() || c.is_ascii_digit())),
        "aws-secret-key" => value.len() == 40 && all(value, |c| alnum(c) || c == '/' || c == '+'),
        "github-token" => {
            rest_alnum(after(value, &[concat!("ghp", "_"), "gho_", "ghu_", "ghs_", "ghr_"]), 36)
                || after(value, &["github_pat_"]).is_some_and(|r| r.len() >= 22 && all(r, |c| alnum(c) || c == '_'))
        }
        "slack-token" => after(value, &["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"]).is_some_and(|r| r.len() >= 10 && all(r, |c| alnum(c) || c == '-')),
        "jwt" => {
            let parts: Vec<&str> = value.split('.').collect();
            parts.len() == 3 && parts[0].starts_with("eyJ") && parts.iter().all(|p| all(p, |c| alnum(c) || c == '-' || c == '_' || c == '='))
        }
        "uuid" => is_uuid(value),
        "hex" => value.len() >= 8 && all(value, |c| c.is_ascii_hexdigit()),
        "base64" => value.len() >= 8 && all(value, |c| alnum(c) || "+/=_-".contains(c)),
        "alnum" => value.len() >= 8 && all(value, alnum),
        n => {
            let digits: usize = n.strip_prefix("digits:")?.parse().ok().filter(|d| (1..=32).contains(d))?;
            value.len() == digits && all(value, |c| c.is_ascii_digit())
        }
    })
}

/// What the agent asked a captured value to be.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureShape {
    /// The value starts with this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// The value is this kind of secret, one of [`NAMES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The value is this many characters at least, and at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}

impl CaptureShape {
    /// The shape asked for, or none when nothing was; an error says what to fix.
    pub fn new(prefix: Option<&str>, name: Option<&str>, min_length: Option<u32>, max_length: Option<u32>) -> Result<Option<Self>, String> {
        let prefix = prefix.map(str::trim).filter(|p| !p.is_empty());
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        if prefix.is_none() && name.is_none() && min_length.is_none() && max_length.is_none() {
            return Ok(None);
        }
        if prefix.is_some_and(|p| p.chars().count() > MAX_PREFIX || p.chars().any(char::is_whitespace)) {
            return Err(format!("prefix is one word of at most {MAX_PREFIX} characters"));
        }
        if name.is_some_and(|n| is_named(n, "").is_none()) {
            return Err(format!("shape is one of: {}", NAMES.join(", ")));
        }
        let (min, max) = (min_length.unwrap_or(1), max_length.unwrap_or(MAX_LENGTH));
        if min == 0 || max > MAX_LENGTH || min > max {
            return Err(format!("min_length and max_length are from 1 to {MAX_LENGTH}, the first not above the second"));
        }
        Ok(Some(Self { prefix: prefix.map(str::to_string), name: name.map(str::to_string), min_length, max_length }))
    }

    /// Whether `value` (trimmed) has the shape.
    pub fn matches(&self, value: &str) -> bool {
        let v = value.trim();
        let len = v.chars().count() as u32;
        self.prefix.as_deref().is_none_or(|p| v.starts_with(p))
            && self.name.as_deref().is_none_or(|n| is_named(n, v) == Some(true))
            && self.min_length.is_none_or(|m| len >= m)
            && self.max_length.is_none_or(|m| len <= m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture key, built from two pieces so no whole one sits in the source.
    fn key(prefix: &str, body: &str) -> String {
        format!("{prefix}{body}")
    }

    fn shape(prefix: Option<&str>, name: Option<&str>, min: Option<u32>, max: Option<u32>) -> CaptureShape {
        CaptureShape::new(prefix, name, min, max).unwrap().unwrap()
    }

    #[test]
    fn nothing_asked_for_is_no_shape() {
        assert!(CaptureShape::new(None, None, None, None).unwrap().is_none());
        assert!(CaptureShape::new(Some("  "), Some(""), None, None).unwrap().is_none());
    }

    #[test]
    fn each_named_shape_takes_its_kind_of_value_and_refuses_the_others() {
        let cases: [(&str, &str, &str); 11] = [
            ("stripe-secret", &key(concat!("sk_live", "_"), "abcdefghij123456"), &key("pk_live_", "abcdefghij123456")),
            ("stripe-publishable", &key("pk_test_", "abcdefghij123456"), &key("sk_test_", "abcdefghij123456")),
            ("stripe-webhook", &key("whsec_", "abcdefghij123456"), &key("whsec_", "short")),
            ("aws-access-key", concat!("AKI", "AIOSFODNN7EXAMPLE"), concat!("AKI", "AIOSFODNN7EXAMPLE1")),
            ("aws-secret-key", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKE"),
            ("github-token", concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789"), concat!("ghp", "_short")),
            ("slack-token", "xoxb-1234567890-abcdef", "xoxz-1234567890-abcdef"),
            ("jwt", "eyJhbGciOi.eyJzdWIiOiIx.c2lnbmF0dXJl", "eyJhbGciOi.eyJzdWIiOiIx"),
            ("uuid", "0f4c2a1e-9b7d-4e3a-8c6f-1a2b3c4d5e6f", "0f4c2a1e-9b7d-4e3a-8c6f"),
            ("digits:6", "482913", "48291"),
            ("hex", "c58ebaa8e2f1b0d9", "c58ebaa8e2f1b0dg"),
        ];
        for (name, yes, no) in cases {
            let s = shape(None, Some(name), None, None);
            assert!(s.matches(yes), "{name} takes {yes}");
            assert!(!s.matches(no), "{name} refuses {no}");
        }
    }

    #[test]
    fn a_prefix_and_a_length_range_narrow_it_further() {
        let s = shape(Some(concat!("sk_live", "_")), Some("stripe-secret"), Some(20), Some(40));
        assert!(s.matches(concat!("  sk_live", "_abcdefghij123456\n")), "trimmed");
        assert!(!s.matches(&key("sk_test_", "abcdefghij123456")), "the prefix");
        assert!(!s.matches(&key(concat!("sk_live", "_"), "abcdefghij1234567890abcdefghij1234567890")), "the range");
        assert!(shape(Some("ab"), None, None, None).matches("abc") && !shape(Some("ab"), None, None, None).matches("ba"));
    }

    #[test]
    fn a_shape_that_is_not_one_of_the_list_says_what_the_list_is() {
        let err = CaptureShape::new(None, Some("[a-m].*"), None, None).unwrap_err();
        assert!(err.contains("stripe-secret") && err.contains("digits:N"), "{err}");
        assert!(CaptureShape::new(None, Some("digits:0"), None, None).is_err());
        assert!(CaptureShape::new(None, Some("digits:33"), None, None).is_err());
        assert!(CaptureShape::new(Some("thirteen_char"), None, None, None).unwrap_err().contains("12"));
        assert!(CaptureShape::new(Some("two words"), None, None, None).is_err());
        assert!(CaptureShape::new(None, None, Some(10), Some(5)).is_err());
        assert!(CaptureShape::new(None, None, Some(0), None).is_err());
    }
}
