// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-provider stdout/stderr pattern matchers for the pre-launch
//! OAuth flow. The `auth login` subprocess of each CLI provider
//! emits an OAuth URL (or device code) to stdout/stderr in a slightly
//! different shape. This module knows how to extract them.
//!
//! See `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §4.

/// What a pattern matcher extracted from a single line of provider output.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthPatternMatch {
    /// CLI emitted an OAuth URL the user should open in a browser.
    OAuthUrl(String),
    /// CLI emitted a device-code pair (GitHub Copilot style).
    DeviceCode {
        /// The pairing code the user types into the verification URL.
        code: String,
        /// Where the user goes to enter the code.
        verification_url: String,
    },
    /// CLI emitted a "logged in as <email>" line. Used to know we're
    /// done and to populate the bundle's display name.
    LoginSuccess { email: Option<String> },
    /// CLI emitted an "authentication failed" line.
    LoginFailure { message: String },
}

/// Try every pattern for the given provider against a single line of
/// captured output. Returns the FIRST match found — patterns are
/// listed by descending specificity in `patterns_for(provider_id)`.
pub fn match_line(provider_id: &str, line: &str) -> Option<AuthPatternMatch> {
    for matcher in patterns_for(provider_id) {
        if let Some(m) = matcher(line) {
            return Some(m);
        }
    }
    // Universal fallback — any oauth-ish https URL gets surfaced for
    // user paste-back if the specific matcher missed it. Skipped
    // entirely for API-key providers because their onboarding output
    // ("get your key at https://.../auth") would otherwise be
    // mis-classified as OAuth and drive the wrong UI branch.
    // (reagent P1 + codex P2 on PR #840.)
    if is_api_key_provider(provider_id) {
        return None;
    }
    if let Some(url) = extract_first_https_url(line) {
        if looks_like_oauth_url(&url) {
            return Some(AuthPatternMatch::OAuthUrl(url));
        }
    }
    None
}

fn is_api_key_provider(provider_id: &str) -> bool {
    matches!(provider_id, "openclaw" | "kimi" | "pi")
}

type LineMatcher = fn(&str) -> Option<AuthPatternMatch>;

fn patterns_for(provider_id: &str) -> &'static [LineMatcher] {
    match provider_id {
        "claude" => &[match_claude_url, match_logged_in_as],
        "codex" => &[match_codex_url, match_logged_in_as],
        "gemini" => &[match_gemini_url, match_logged_in_as],
        "copilot" => &[match_copilot_device_code, match_logged_in_as],
        // API-key providers don't OAuth — these patterns never match.
        // Listed here so the dispatch is exhaustive and adding a new
        // provider always lands a code edit in this table.
        "openclaw" | "kimi" | "pi" => &[],
        _ => &[],
    }
}

// ────────────────────────────────────────────────────────────────────
// Per-provider matchers
// ────────────────────────────────────────────────────────────────────

fn match_claude_url(line: &str) -> Option<AuthPatternMatch> {
    // Claude Code emits something like:
    //   "Open this URL in your browser to authorize:"
    //   "https://console.anthropic.com/oauth/authorize?response_type=..."
    if let Some(url) = extract_first_https_url(line) {
        if url.contains("anthropic.com/oauth") || url.contains("console.anthropic.com") {
            return Some(AuthPatternMatch::OAuthUrl(url));
        }
    }
    None
}

fn match_codex_url(line: &str) -> Option<AuthPatternMatch> {
    if let Some(url) = extract_first_https_url(line) {
        if url.contains("auth.openai.com")
            || url.contains("platform.openai.com")
            || url.contains("openai.com/oauth")
        {
            return Some(AuthPatternMatch::OAuthUrl(url));
        }
    }
    None
}

fn match_gemini_url(line: &str) -> Option<AuthPatternMatch> {
    if let Some(url) = extract_first_https_url(line) {
        if url.contains("accounts.google.com") || url.contains("oauth2.googleapis.com") {
            return Some(AuthPatternMatch::OAuthUrl(url));
        }
    }
    None
}

fn match_copilot_device_code(line: &str) -> Option<AuthPatternMatch> {
    // GitHub device flow output:
    //   "! First copy your one-time code: XXXX-YYYY"
    //   "Then press Enter to open github.com in your browser..."
    //   or
    //   "Please visit https://github.com/login/device and enter code XXXX-YYYY"
    let line_low = line.to_lowercase();
    let mentions_device = line_low.contains("github.com/login/device")
        || line_low.contains("one-time code")
        || line_low.contains("enter code");
    if !mentions_device {
        return None;
    }
    let code = extract_device_code(line);
    if let Some(code) = code {
        // The URL is constant for GitHub device flow.
        return Some(AuthPatternMatch::DeviceCode {
            code,
            verification_url: "https://github.com/login/device".to_string(),
        });
    }
    None
}

fn match_logged_in_as(line: &str) -> Option<AuthPatternMatch> {
    let line_low = line.to_lowercase();
    // Negative forms ("not authenticated", "not logged in", "isn't
    // authenticated", "n't logged in", "failed to authenticate") are
    // status messages, not success — fall through. Reagent caught the
    // bare `contains("authenticated")` matching error lines on PR #840.
    if line_low.contains("not authenticated")
        || line_low.contains("not logged in")
        || line_low.contains("n't authenticated")
        || line_low.contains("n't logged in")
        || line_low.contains("failed to authenticate")
        || line_low.contains("authentication failed")
    {
        return None;
    }
    if !line_low.contains("logged in") && !line_low.contains("authenticated") {
        return None;
    }
    // Heuristic email extraction. We don't gate on it — `email`
    // can be None and the bundle gets a default name.
    let email = extract_email(line);
    Some(AuthPatternMatch::LoginSuccess { email })
}

// ────────────────────────────────────────────────────────────────────
// Generic helpers
// ────────────────────────────────────────────────────────────────────

fn extract_first_https_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let tail = &line[start..];
    // URL ends at whitespace, quote, backtick, or closing bracket.
    // Keeps ports, paths, query strings, fragments.
    let end = tail
        .find(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '`' || c == ')')
        .unwrap_or(tail.len());
    let url = &tail[..end];
    // Trim trailing sentence punctuation that a CLI message might
    // append (e.g. "Authorize at https://...?state=xyz."). The
    // browser would otherwise see an invalid URL. Be conservative —
    // only strip end-of-sentence chars, not anything that could be
    // part of a legitimate URL token. (reagent P1 on PR #840.)
    let trimmed = url.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | '!' | '?'));
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn looks_like_oauth_url(url: &str) -> bool {
    let u = url.to_lowercase();
    u.contains("oauth")
        || u.contains("/authorize")
        || u.contains("/login")
        || u.contains("/auth")
        || u.contains("device")
}

fn extract_email(line: &str) -> Option<String> {
    // Conservative email extractor: find a token containing '@' with
    // valid surrounding chars. Skips placeholders like "<email>".
    for token in line.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '.' && c != '-' && c != '_' && c != '+');
        if !token.contains('@') {
            continue;
        }
        // `if let` (not `?`) — `?` would exit the function and abort
        // the search instead of just skipping this token. The `@`
        // check above means split_once should always be Some here in
        // practice, but the safe pattern is to never `?` inside a
        // for-loop unless the function-exit semantics are intended.
        // (reagent P1 / codex P2 on PR #840.)
        let Some((local, domain)) = token.split_once('@') else {
            continue;
        };
        if local.is_empty() || domain.is_empty() {
            continue;
        }
        if !domain.contains('.') {
            continue;
        }
        return Some(token.to_string());
    }
    None
}

fn extract_device_code(line: &str) -> Option<String> {
    // GitHub device codes look like XXXX-YYYY (uppercase alphanumeric).
    // Find a token of the form `[A-Z0-9]{4}-[A-Z0-9]{4}`.
    for token in line.split(|c: char| c.is_whitespace() || c == ':' || c == ',' || c == '.') {
        if token.len() == 9 {
            let bytes = token.as_bytes();
            let is_code = bytes[4] == b'-'
                && bytes[..4]
                    .iter()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                && bytes[5..]
                    .iter()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
            if is_code {
                return Some(token.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_url_matched() {
        let line = "Open this URL in your browser to authorize: https://console.anthropic.com/oauth/authorize?response_type=code&state=xyz";
        let m = match_line("claude", line);
        assert!(matches!(m, Some(AuthPatternMatch::OAuthUrl(_))));
        if let Some(AuthPatternMatch::OAuthUrl(u)) = m {
            assert!(u.contains("anthropic.com/oauth"));
            assert!(u.contains("response_type=code"));
        }
    }

    #[test]
    fn codex_url_matched() {
        let line = "Please visit https://auth.openai.com/u/login/identifier?state=abc to continue";
        let m = match_line("codex", line);
        assert!(matches!(m, Some(AuthPatternMatch::OAuthUrl(u)) if u.contains("openai.com")));
    }

    #[test]
    fn gemini_url_matched() {
        let line = "Visit https://accounts.google.com/o/oauth2/auth?client_id=abc&scope=...";
        let m = match_line("gemini", line);
        assert!(matches!(m, Some(AuthPatternMatch::OAuthUrl(u)) if u.contains("google.com")));
    }

    #[test]
    fn copilot_device_code_matched() {
        let line = "! First copy your one-time code: ABCD-1234";
        let m = match_line("copilot", line);
        if let Some(AuthPatternMatch::DeviceCode { code, verification_url }) = m {
            assert_eq!(code, "ABCD-1234");
            assert_eq!(verification_url, "https://github.com/login/device");
        } else {
            panic!("expected DeviceCode, got {m:?}");
        }
    }

    #[test]
    fn copilot_device_url_line_matched() {
        let line = "Please visit https://github.com/login/device and enter code WXYZ-5678";
        let m = match_line("copilot", line);
        if let Some(AuthPatternMatch::DeviceCode { code, .. }) = m {
            assert_eq!(code, "WXYZ-5678");
        } else {
            panic!("expected DeviceCode, got {m:?}");
        }
    }

    #[test]
    fn login_success_with_email() {
        let m = match_line("claude", "Successfully logged in as asaf@example.com");
        if let Some(AuthPatternMatch::LoginSuccess { email }) = m {
            assert_eq!(email.as_deref(), Some("asaf@example.com"));
        } else {
            panic!("expected LoginSuccess, got {m:?}");
        }
    }

    #[test]
    fn login_success_without_email() {
        let m = match_line("claude", "You are now authenticated.");
        assert!(matches!(m, Some(AuthPatternMatch::LoginSuccess { email: None })));
    }

    #[test]
    fn login_success_skips_negative_forms() {
        // Reagent P2 on PR #840: bare `contains("authenticated")`
        // matched "Error: not authenticated" and "Authentication
        // failed" lines, falsely promoting them to LoginSuccess.
        for line in [
            "Error: not authenticated",
            "you are not authenticated",
            "user isn't authenticated yet",
            "You aren't logged in",
            "failed to authenticate",
            "Authentication failed",
        ] {
            let m = match_line("claude", line);
            assert!(m.is_none(), "expected None for {line:?}, got {m:?}");
        }
    }

    #[test]
    fn fallback_https_matches_for_unknown_providers() {
        // Unknown provider, but the line has an OAuth-ish URL. The
        // fallback should still catch it so the user gets the paste
        // option in the UI.
        let line = "Open https://example.com/oauth/authorize?state=x";
        let m = match_line("future-provider", line);
        assert!(matches!(m, Some(AuthPatternMatch::OAuthUrl(_))));
    }

    #[test]
    fn fallback_skips_non_oauth_https() {
        // A line with an https URL that doesn't look like OAuth (e.g.
        // a documentation link) shouldn't false-positive.
        let line = "See https://docs.example.com/getting-started for details.";
        let m = match_line("future-provider", line);
        assert!(m.is_none());
    }

    #[test]
    fn api_key_providers_match_nothing() {
        // openclaw / kimi / pi don't have OAuth flow. Even if their
        // output includes an https URL during onboarding, we don't
        // want to misinterpret it as OAuth.
        for provider in ["openclaw", "kimi", "pi"] {
            let m = match_line(provider, "Get your API key at https://example.com/keys");
            assert!(m.is_none(), "provider {provider} unexpectedly matched");
        }
    }

    #[test]
    fn url_extraction_trims_trailing_punctuation() {
        // Period, comma, semicolon, !, ? at the END of a URL are
        // almost certainly sentence terminators, not URL chars.
        // The browser would reject the URL with them attached.
        // (reagent P1 on PR #840.)
        let cases = [
            ("Go to https://example.com/login.", "https://example.com/login"),
            ("Open https://example.com/auth, then press Enter", "https://example.com/auth"),
            ("Visit https://example.com/login!", "https://example.com/login"),
            ("Done at https://example.com/oauth?state=xyz?", "https://example.com/oauth?state=xyz"),
        ];
        for (line, expected) in cases {
            let url = extract_first_https_url(line).expect("url");
            assert_eq!(url, expected, "for line: {line}");
        }
    }

    #[test]
    fn url_extraction_preserves_internal_punctuation() {
        // Make sure we don't over-trim — query strings legitimately
        // have `&` and `=`, paths can have `.`, etc.
        let line = "Open https://example.com/path.html?key=value&other=v2";
        let url = extract_first_https_url(line).expect("url");
        assert_eq!(url, "https://example.com/path.html?key=value&other=v2");
    }

    #[test]
    fn api_key_provider_fallback_returns_none() {
        // Reagent P1 + codex P2 on PR #840: the universal fallback
        // used to run for API-key providers. Their onboarding output
        // ("get your key at https://.../auth") would mis-classify
        // as OAuth and drive the wrong UI branch. Now: no fallback
        // for openclaw/kimi/pi at all.
        for provider in ["openclaw", "kimi", "pi"] {
            let line = "Get your API key at https://example.com/auth/keys";
            assert!(match_line(provider, line).is_none(), "{provider} matched");
        }
        // Whereas an unknown provider still gets the fallback.
        let m = match_line("unknown-provider", "Open https://example.com/oauth/authorize");
        assert!(matches!(m, Some(AuthPatternMatch::OAuthUrl(_))));
    }

    #[test]
    fn email_extractor_ignores_placeholders() {
        // `<email>` is a typical CLI placeholder shown in help text.
        assert!(extract_email("Sign in as <email>").is_none());
        // Plain "@example" with no TLD shouldn't match.
        assert!(extract_email("Twitter handle: @example").is_none());
    }

    #[test]
    fn device_code_extractor_recognises_correct_shape() {
        assert_eq!(extract_device_code("code: ABCD-1234"), Some("ABCD-1234".to_string()));
        assert_eq!(extract_device_code("the code is XYZW-0987 right here"), Some("XYZW-0987".to_string()));
        // Wrong length, wrong separator — should miss.
        assert_eq!(extract_device_code("code: ABC-1234"), None);
        assert_eq!(extract_device_code("code: ABCD_1234"), None);
        // Lowercase — GitHub uses uppercase only.
        assert_eq!(extract_device_code("code: abcd-1234"), None);
    }
}
