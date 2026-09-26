// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shape-based, best-effort secret redaction — the one Rust implementation
//! `SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md` §4.5 asks every formatter that
//! puts error/log text on the clipboard to share, instead of each inventing
//! its own. Moved here from `agentmux-srv/src/backend/continuity.rs`, which
//! only recognized credential-prefixed tokens and PEM private keys — enough
//! for a continuation packet, not enough for raw stderr and request details
//! (Codex P1 on #3689).
//!
//! Deliberately manual string scanning, not the `regex` crate: nothing in
//! this workspace depends on `regex` today, and `continuity.rs`'s own
//! pre-existing redaction (kept here unchanged) already established that as
//! the house style for this kind of text scan.
//!
//! Mirrored on the TypeScript side by `frontend/app/errors/redact.ts`, and
//! both are tested against the same `docs/specs/fixtures/redaction-vectors.json`
//! so the two can't drift apart (§4.5).

const SECRET_PREFIXES: &[&str] = &[
    "github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "sk-", "xoxa-", "xoxb-", "xoxo-", "xoxp-", "xoxr-", "xoxs-",
    "AKIA",
];

const REDACTED_HEADERS: &[&str] = &["authorization", "proxy-authorization", "x-api-key", "api-key", "cookie", "set-cookie"];

const SENSITIVE_KEY_SUBSTRINGS: &[&str] = &[
    "password", "passwd", "pwd", "secret", "token", "api_key", "apikey", "access_key", "private_key", "client_secret",
    "credential",
];

/// The full redaction pipeline. Order matters: the original credential-prefix
/// and private-key passes run first (unchanged, so their existing callers'
/// exact output is preserved byte-for-byte on inputs that only they match),
/// then the newer, broader shape-based passes.
pub fn redact_secrets(text: &str) -> String {
    let out = redact_credential_prefixes(text);
    let out = redact_private_keys(&out);
    let out = redact_headers(&out);
    let out = redact_key_value_pairs(&out);
    let out = redact_url_userinfo(&out);
    let out = redact_jwts(&out);
    redact_aws_secret_key(&out)
}

/// Blanks credential-prefixed tokens (`ghp_...`, `sk-...`, AWS access-key
/// ids, …). Deliberately shape-based: it errs toward redacting a long token
/// that merely looks like a secret. Was `continuity.rs::redact_secrets`.
fn redact_credential_prefixes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let hit = SECRET_PREFIXES
            .iter()
            .filter_map(|p| rest.find(p).map(|i| (i, *p)))
            .min_by_key(|(i, p)| (*i, std::cmp::Reverse(p.len())));
        let Some((at, prefix)) = hit else {
            out.push_str(rest);
            break;
        };
        let after = &rest[at + prefix.len()..];
        let body = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(after.len());
        out.push_str(&rest[..at]);
        if body >= 16 {
            out.push_str("[redacted secret]");
        } else {
            out.push_str(&rest[at..at + prefix.len() + body]);
        }
        rest = &after[body..];
    }
    out
}

/// Blanks PEM private keys (`-----BEGIN ... PRIVATE KEY-----` blocks), never
/// a certificate. Public — `continuity.rs`'s own tests exercise this pass in
/// isolation (ReAgent P0 on #3643: a certificate ahead of a key must not end
/// the scan and let the key through).
pub fn redact_private_keys(text: &str) -> String {
    let mut out = text.to_string();
    let mut from = 0;
    while let Some(found) = out[from..].find("-----BEGIN") {
        let start = from + found;
        let header_end = out[start..].find('\n').map_or(out.len(), |i| start + i);
        if !out[start..header_end].contains("PRIVATE KEY") {
            from = header_end;
            continue;
        }
        let end = out[start..]
            .find("-----END")
            .and_then(|i| out[start + i..].find("KEY-----").map(|j| start + i + j + "KEY-----".len()))
            .unwrap_or(out.len());
        out.replace_range(start..end, "[redacted private key]");
        from = start + "[redacted private key]".len();
    }
    out
}

fn ends_with_newline_preserved(original: &str, out: String) -> String {
    if original.ends_with('\n') && !out.ends_with('\n') {
        let mut out = out;
        out.push('\n');
        out
    } else {
        out
    }
}

/// Blanks `Authorization:` / `Proxy-Authorization:` / `x-api-key:` /
/// `api-key:` / `cookie:` / `set-cookie:` header *values* (any scheme —
/// `Bearer`, `Basic`, `Token`, a raw key, a cookie jar), line by line.
fn redact_headers(text: &str) -> String {
    let redacted = text
        .lines()
        .map(|line| {
            let trimmed_start = line.trim_start();
            let indent_len = line.len() - trimmed_start.len();
            if let Some(colon) = trimmed_start.find(':') {
                let name = trimmed_start[..colon].trim();
                if REDACTED_HEADERS.iter().any(|h| h.eq_ignore_ascii_case(name)) {
                    return format!("{}{}: [redacted]", &line[..indent_len], name);
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    ends_with_newline_preserved(text, redacted)
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase();
    SENSITIVE_KEY_SUBSTRINGS.iter().any(|s| lower.contains(s))
}

/// Blanks `key=value`, `key: value` and JSON `"key": "value"` pairs whose
/// key *contains* a sensitive substring, case-insensitive — deliberately
/// "contains", not "equals" (§4.5's literal wording), matching this module's
/// existing err-toward-redacting philosophy. A key like `tokenizer` is
/// redacted by this rule even though the word alone is harmless prose
/// (`ordinary_words_that_share_a_prefix_survive_redaction` below covers the
/// prose case, which never has the `key=`/`key:` shape this rule requires).
fn redact_key_value_pairs(text: &str) -> String {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0usize;
    let mut idx = 0usize;
    while idx < chars.len() {
        let (_, ch) = chars[idx];
        if ch != '=' && ch != ':' {
            idx += 1;
            continue;
        }
        // A JSON key's closing quote sits right before ':' — skip past it so
        // the key-char scan below reaches the actual key text, not nothing.
        let mut key_end_idx = idx;
        if key_end_idx > 0 && matches!(chars[key_end_idx - 1].1, '"' | '\'') {
            key_end_idx -= 1;
        }
        let mut key_start_idx = key_end_idx;
        while key_start_idx > 0 && is_key_char(chars[key_start_idx - 1].1) {
            key_start_idx -= 1;
        }
        if key_start_idx == key_end_idx {
            idx += 1;
            continue;
        }
        let key_start_byte = chars[key_start_idx].0;
        let key_end_byte = chars[key_end_idx].0;
        let key = &text[key_start_byte..key_end_byte];
        if !is_sensitive_key(key) {
            idx += 1;
            continue;
        }
        // Forward: separator, optional whitespace, optional quote, the value.
        let mut v = idx + 1;
        while v < chars.len() && (chars[v].1 == ' ' || chars[v].1 == '\t') {
            v += 1;
        }
        let val_quote = if v < chars.len() && (chars[v].1 == '"' || chars[v].1 == '\'') { Some(chars[v].1) } else { None };
        let val_start = if val_quote.is_some() { v + 1 } else { v };
        let mut w = val_start;
        if let Some(vq) = val_quote {
            while w < chars.len() && chars[w].1 != vq {
                w += 1;
            }
        } else {
            while w < chars.len() && !matches!(chars[w].1, ' ' | '\t' | '\n' | '\r' | ',' | '}' | ')' | ';') {
                w += 1;
            }
        }
        let value_prefix_end = if chars.get(val_start).is_some() || val_start == chars.len() {
            if val_start < chars.len() { chars[val_start].0 } else { text.len() }
        } else {
            text.len()
        };
        out.push_str(&text[last_end..value_prefix_end]);
        out.push_str("[redacted]");
        let skip_to = if val_quote.is_some() { w + 1 } else { w };
        last_end = if skip_to < chars.len() { chars[skip_to].0 } else { text.len() };
        if val_quote.is_some() && w < chars.len() {
            out.push(chars[w].1);
        }
        idx = skip_to;
    }
    out.push_str(&text[last_end..]);
    out
}

/// Blanks URL userinfo passwords: `scheme://user:password@host` keeps
/// `scheme://user:[redacted]@host`.
fn redact_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0usize;
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find("://") {
        let scheme_end = search_from + rel + 3;
        let authority_end = text[scheme_end..]
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .map(|i| scheme_end + i)
            .unwrap_or(text.len());
        let authority = &text[scheme_end..authority_end];
        if let Some(at_rel) = authority.find('@') {
            let at_pos = scheme_end + at_rel;
            let userinfo = &authority[..at_rel];
            if let Some(colon_rel) = userinfo.find(':') {
                let colon_pos = scheme_end + colon_rel;
                out.push_str(&text[last_end..=colon_pos]);
                out.push_str("[redacted]");
                last_end = at_pos;
            }
        }
        search_from = authority_end.max(scheme_end + 1);
        if search_from > text.len() {
            break;
        }
    }
    out.push_str(&text[last_end..]);
    out
}

fn is_base64url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Blanks JWTs: three base64url segments, the first starting `eyJ` (a
/// base64url-encoded `{"` — every JWT header starts with a JSON object).
fn redact_jwts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0usize;
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find("eyJ") {
        let start = search_from + rel;
        let prev_is_word = start > 0 && is_base64url_char(text[..start].chars().next_back().unwrap());
        if prev_is_word {
            search_from = start + 3;
            continue;
        }
        let seg1_end = start + text[start..].find(|c: char| !is_base64url_char(c)).unwrap_or(text.len() - start);
        let dot1_ok = seg1_end < text.len() && text.as_bytes()[seg1_end] == b'.';
        if !dot1_ok {
            search_from = start + 3;
            continue;
        }
        let seg2_start = seg1_end + 1;
        let seg2_end = seg2_start + text[seg2_start..].find(|c: char| !is_base64url_char(c)).unwrap_or(text.len() - seg2_start);
        let dot2_ok = seg2_end > seg2_start && seg2_end < text.len() && text.as_bytes()[seg2_end] == b'.';
        if !dot2_ok {
            search_from = start + 3;
            continue;
        }
        let seg3_start = seg2_end + 1;
        let seg3_end = seg3_start + text[seg3_start..].find(|c: char| !is_base64url_char(c)).unwrap_or(text.len() - seg3_start);
        if seg3_end == seg3_start {
            search_from = start + 3;
            continue;
        }
        out.push_str(&text[last_end..start]);
        out.push_str("[redacted]");
        last_end = seg3_end;
        search_from = seg3_end;
    }
    out.push_str(&text[last_end..]);
    out
}

fn is_base64_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='
}

/// Blanks a 40-character base64 value on the same line as
/// `aws_secret_access_key` (any case).
fn redact_aws_secret_key(text: &str) -> String {
    let redacted = text
        .lines()
        .map(|line| {
            if !line.to_ascii_lowercase().contains("aws_secret_access_key") {
                return line.to_string();
            }
            let chars: Vec<(usize, char)> = line.char_indices().collect();
            let mut i = 0usize;
            while i < chars.len() {
                if !is_base64_char(chars[i].1) {
                    i += 1;
                    continue;
                }
                let run_start = i;
                let mut j = i;
                while j < chars.len() && is_base64_char(chars[j].1) {
                    j += 1;
                }
                if j - run_start == 40 {
                    let start_byte = chars[run_start].0;
                    let end_byte = if j < chars.len() { chars[j].0 } else { line.len() };
                    return format!("{}[redacted]{}", &line[..start_byte], &line[end_byte..]);
                }
                i = j;
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    ends_with_newline_preserved(text, redacted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_private_key_after_a_certificate_is_still_redacted() {
        let text = "-----BEGIN CERTIFICATE-----\nMIIBcert\n-----END CERTIFICATE-----\n\
                    -----BEGIN RSA PRIVATE KEY-----\nMIIEsecret\n-----END RSA PRIVATE KEY-----\ndone";
        let out = redact_private_keys(text);
        assert!(out.contains("MIIBcert"), "a certificate is public: {out}");
        assert!(!out.contains("MIIEsecret"), "{out}");
        assert!(out.contains("[redacted private key]") && out.ends_with("done"));
    }

    #[test]
    fn every_slack_token_family_is_redacted() {
        for prefix in ["xoxa-", "xoxb-", "xoxo-", "xoxp-", "xoxr-", "xoxs-"] {
            let token = format!("{prefix}1234567890-abcdefghijklmnop");
            assert_eq!(redact_secrets(&format!("t {token} t")), "t [redacted secret] t", "{prefix}");
        }
    }

    #[test]
    fn ordinary_words_that_share_a_prefix_survive_redaction() {
        assert_eq!(redact_secrets("a task-list and a desk-lamp"), "a task-list and a desk-lamp");
    }

    #[test]
    fn authorization_header_any_scheme_is_redacted() {
        assert_eq!(redact_secrets("Authorization: Bearer abc.def.ghi"), "Authorization: [redacted]");
        assert_eq!(redact_secrets("Authorization: Basic dXNlcjpwYXNz"), "Authorization: [redacted]");
        assert_eq!(redact_secrets("proxy-authorization: Token xyz"), "proxy-authorization: [redacted]");
    }

    #[test]
    fn cookie_and_api_key_headers_are_redacted() {
        assert_eq!(redact_secrets("x-api-key: abc123"), "x-api-key: [redacted]");
        assert_eq!(redact_secrets("Cookie: session=abc123"), "Cookie: [redacted]");
        assert_eq!(redact_secrets("Set-Cookie: session=abc123; Path=/"), "Set-Cookie: [redacted]");
    }

    #[test]
    fn key_value_forms_are_redacted() {
        assert_eq!(redact_secrets("password=hunter2"), "password=[redacted]");
        assert_eq!(redact_secrets("password: hunter2"), "password: [redacted]");
        assert_eq!(redact_secrets(r#"{"password": "hunter2"}"#), r#"{"password": "[redacted]"}"#);
        assert_eq!(redact_secrets("client_secret=abc123"), "client_secret=[redacted]");
        assert_eq!(redact_secrets("credential=abc123"), "credential=[redacted]");
    }

    #[test]
    fn url_userinfo_password_is_redacted_not_the_username() {
        assert_eq!(
            redact_secrets("fetch https://alice:hunter2@example.com/path?q=1"),
            "fetch https://alice:[redacted]@example.com/path?q=1"
        );
    }

    #[test]
    fn a_jwt_is_redacted_whole() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(redact_secrets(&format!("token is {jwt} end")), "token is [redacted] end");
    }

    #[test]
    fn an_aws_secret_access_key_is_redacted() {
        assert_eq!(
            redact_secrets("aws_secret_access_key=ABCDEFGHIJabcdefghij1234567890ABCDEFGHIJ"),
            "aws_secret_access_key=[redacted]"
        );
    }

    #[test]
    fn a_secret_straddling_a_cut_boundary_is_still_redacted() {
        let long = format!("{}ghp_abcdefghijklmnopqrstuvwxyz0123{}", "a".repeat(1_994), "b".repeat(5_000));
        let out = redact_secrets(&long);
        assert!(!out.contains("ghp_ab"), "a fragment of the token leaked");
        assert!(out.contains("[redacted secret]"));
    }

    /// §4.5 / §8: one shared set of vectors, run by both this crate and
    /// `frontend/app/errors/redact.test.ts`, so the two implementations
    /// can't silently drift apart.
    #[test]
    fn shared_redaction_vectors() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("agentmux-common has a parent dir")
            .join("docs/specs/fixtures/redaction-vectors.json");
        let raw = std::fs::read_to_string(&root).unwrap_or_else(|e| panic!("reading {root:?}: {e}"));
        let vectors: Vec<serde_json::Value> = serde_json::from_str(&raw).expect("valid JSON");
        assert!(!vectors.is_empty());
        for v in &vectors {
            let name = v["name"].as_str().unwrap_or("<unnamed>");
            let input = v["input"].as_str().expect("vector.input is a string");
            let expected = v["expected"].as_str().expect("vector.expected is a string");
            assert_eq!(redact_secrets(input), expected, "vector {name:?}");
        }
    }
}
