// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Quote handling utilities for CLI argument processing.
//! Port of Go's pkg/trimquotes/trimquotes.go.

#![allow(dead_code)]

/// Remove surrounding double quotes from a string.
/// Handles escape sequences within the quoted string.
///
/// Returns `(unquoted_string, success)`.
/// If the string is not properly quoted, returns the original and `false`.
///
/// # Examples
///
/// ```
/// use backend_test::backend::trimquotes::trim_quotes;
///
/// let (s, ok) = trim_quotes(r#""hello world""#);
/// assert_eq!(s, "hello world");
/// assert!(ok);
///
/// let (s, ok) = trim_quotes("unquoted");
/// assert_eq!(s, "unquoted");
/// assert!(!ok);
/// ```
pub fn trim_quotes(s: &str) -> (String, bool) {
    // Go uses `len(s) > 2` — only unquote strings longer than 2 chars.
    // This means `""` (empty quoted string) returns (s, false).
    if s.len() <= 2 || !s.starts_with('"') {
        return (s.to_string(), false);
    }

    match unescape_quoted(s) {
        Some(unquoted) => (unquoted, true),
        None => (s.to_string(), false),
    }
}

/// Convenience wrapper that ignores the success flag.
/// Always returns a string, falling back to the original on failure.
pub fn try_trim_quotes(s: &str) -> String {
    trim_quotes(s).0
}

/// Conditionally wrap a string in double quotes with proper escaping.
pub fn replace_quotes(s: &str, should_replace: bool) -> String {
    if should_replace {
        escape_and_quote(s)
    } else {
        s.to_string()
    }
}

/// Unescape a double-quoted string (like Go's strconv.Unquote).
/// The input must start and end with `"`.
fn unescape_quoted(s: &str) -> Option<String> {
    if !s.starts_with('"') || !s.ends_with('"') || s.len() < 2 {
        return None;
    }

    let inner = &s[1..s.len() - 1];
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('0') => result.push('\0'),
                Some('a') => result.push('\x07'), // bell
                Some('b') => result.push('\x08'), // backspace
                Some('f') => result.push('\x0C'), // form feed
                Some('v') => result.push('\x0B'), // vertical tab
                Some(other) => {
                    // Unknown escape: keep as-is
                    result.push('\\');
                    result.push(other);
                }
                None => return None, // trailing backslash
            }
        } else if c == '"' {
            // Unescaped quote inside — invalid
            return None;
        } else {
            result.push(c);
        }
    }

    Some(result)
}

/// Escape a string and wrap in double quotes.
fn escape_and_quote(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\0' => result.push_str("\\0"),
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_quotes_basic() {
        let (s, ok) = trim_quotes(r#""hello""#);
        assert_eq!(s, "hello");
        assert!(ok);
    }

    #[test]
    fn test_trim_quotes_with_spaces() {
        let (s, ok) = trim_quotes(r#""hello world""#);
        assert_eq!(s, "hello world");
        assert!(ok);
    }

    #[test]
    fn test_trim_quotes_with_escapes() {
        let (s, ok) = trim_quotes(r#""hello\nworld""#);
        assert_eq!(s, "hello\nworld");
        assert!(ok);
    }

    #[test]
    fn test_trim_quotes_escaped_quote() {
        let (s, ok) = trim_quotes(r#""hello\"world""#);
        assert_eq!(s, "hello\"world");
        assert!(ok);
    }

    #[test]
    fn test_trim_quotes_escaped_backslash() {
        let (s, ok) = trim_quotes(r#""path\\to\\file""#);
        assert_eq!(s, "path\\to\\file");
        assert!(ok);
    }

    #[test]
    fn test_trim_quotes_not_quoted() {
        let (s, ok) = trim_quotes("unquoted");
        assert_eq!(s, "unquoted");
        assert!(!ok);
    }

    #[test]
    fn test_trim_quotes_empty() {
        let (s, ok) = trim_quotes("");
        assert_eq!(s, "");
        assert!(!ok);
    }

    #[test]
    fn test_trim_quotes_single_char() {
        let (s, ok) = trim_quotes("a");
        assert_eq!(s, "a");
        assert!(!ok);
    }

    #[test]
    fn test_trim_quotes_empty_quoted() {
        // Go behavior: `""` (2 chars) does NOT unquote — returns as-is with false.
        let (s, ok) = trim_quotes(r#""""#);
        assert_eq!(s, r#""""#);
        assert!(!ok);
    }

    #[test]
    fn test_try_trim_quotes() {
        assert_eq!(try_trim_quotes(r#""test""#), "test");
        assert_eq!(try_trim_quotes("unquoted"), "unquoted");
    }

    #[test]
    fn test_replace_quotes_true() {
        let result = replace_quotes("hello", true);
        assert_eq!(result, r#""hello""#);
    }

    #[test]
    fn test_replace_quotes_false() {
        let result = replace_quotes("hello", false);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_replace_quotes_with_special() {
        let result = replace_quotes("hello\nworld", true);
        assert_eq!(result, r#""hello\nworld""#);
    }

    #[test]
    fn test_replace_quotes_with_quotes() {
        let result = replace_quotes(r#"say "hi""#, true);
        assert_eq!(result, r#""say \"hi\"""#);
    }

    #[test]
    fn test_escape_and_quote_roundtrip() {
        let original = "hello \"world\"\nnew\\line";
        let quoted = escape_and_quote(original);
        let (unquoted, ok) = trim_quotes(&quoted);
        assert!(ok);
        assert_eq!(unquoted, original);
    }

    #[test]
    fn test_all_escape_sequences() {
        let (s, ok) = trim_quotes(r#""tab\there\nnewline\rreturn\0null""#);
        assert!(ok);
        assert!(s.contains('\t'));
        assert!(s.contains('\n'));
        assert!(s.contains('\r'));
        assert!(s.contains('\0'));
    }

    #[test]
    fn test_trailing_backslash_fails() {
        let (s, ok) = trim_quotes(r#""trailing\"#);
        assert!(!ok);
        assert_eq!(s, r#""trailing\"#);
    }
}
