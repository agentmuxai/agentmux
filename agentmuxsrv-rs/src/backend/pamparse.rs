// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PAM environment file parsing utilities.
//! Port of Go's pkg/util/pamparse/.
//!
//! Parses /etc/environment, /etc/security/pam_env.conf, and ~/.pam_environment
//! file formats to extract environment variable definitions.


use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead};

// ---- Types ----

/// Options for PAM environment parsing, providing values
/// for `@{HOME}` and `@{SHELL}` placeholder substitution.
#[derive(Debug, Clone)]
pub struct PamParseOpts {
    pub home: String,
    pub shell: String,
}

// ---- Parsing functions ----

/// Parse an /etc/environment format file.
///
/// Each line has the form `KEY=VALUE` or `export KEY=VALUE`.
/// Comments (#) and quoting are handled.
pub fn parse_environment_file(path: &str) -> io::Result<HashMap<String, String>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut result = HashMap::new();

    for line in reader.lines() {
        let line = line?;
        if let Some((key, val)) = parse_environment_line(&line) {
            result.insert(key.to_string(), val.to_string());
        }
    }

    Ok(result)
}

/// Parse a pam_env.conf or ~/.pam_environment format file.
///
/// Supports `KEY DEFAULT=value OVERRIDE=value` format with
/// `@{HOME}` and `@{SHELL}` placeholder substitution.
/// Falls back to /etc/environment line format if conf format fails.
pub fn parse_environment_conf_file(
    path: &str,
    opts: &PamParseOpts,
) -> io::Result<HashMap<String, String>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut result = HashMap::new();

    for line in reader.lines() {
        let line = line?;
        let (key, val) = match parse_environment_conf_line(&line) {
            Some((k, v)) => (k, v),
            None => match parse_environment_line(&line) {
                Some((k, v)) => (k, v),
                None => continue,
            },
        };
        let val = replace_home_and_shell(&val, &opts.home, &opts.shell);
        result.insert(key, val);
    }

    Ok(result)
}

/// Parse /etc/passwd to find the current user's home directory and shell.
pub fn parse_passwd() -> io::Result<Option<PamParseOpts>> {
    let user = std::env::var("USER").unwrap_or_default();
    if user.is_empty() {
        return Ok(None);
    }

    let file = fs::File::open("/etc/passwd")?;
    let reader = io::BufReader::new(file);
    let prefix = format!("{}:", user);

    for line in reader.lines() {
        let line = line?;
        if line.starts_with(&prefix) {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() < 7 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid passwd entry: insufficient fields",
                ));
            }
            return Ok(Some(PamParseOpts {
                home: parts[5].to_string(),
                shell: parts[6].to_string(),
            }));
        }
    }

    Ok(None)
}

/// Safe version of parse_passwd that returns None on error.
pub fn parse_passwd_safe() -> Option<PamParseOpts> {
    parse_passwd().ok().flatten()
}

// ---- Internal helpers ----

/// Replace `@{HOME}` and `@{SHELL}` placeholders in a value string.
fn replace_home_and_shell(val: &str, home: &str, shell: &str) -> String {
    val.replace("@{HOME}", home).replace("@{SHELL}", shell)
}

/// Parse a line from /etc/environment format.
///
/// Accepts `KEY=VALUE` or `export KEY=VALUE`.
fn parse_environment_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    // Strip optional "export " prefix
    let line = line.strip_prefix("export ").unwrap_or(line);

    let eq_pos = line.find('=')?;
    let key = &line[..eq_pos];

    // Validate key: must match [A-Z0-9_]+[A-Za-z0-9]*
    if key.is_empty() || !is_valid_env_key(key) {
        return None;
    }

    let val = &line[eq_pos + 1..];
    Some((key.to_string(), sanitize_env_var_value(val)))
}

/// Parse a line from pam_env.conf format.
///
/// Format: `KEY DEFAULT=value [OVERRIDE=value]`
fn parse_environment_conf_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    // Split into key and rest
    let mut parts = line.splitn(2, char::is_whitespace);
    let key = parts.next()?;
    let rest = parts.next()?.trim();

    if !is_valid_env_key(key) {
        return None;
    }

    // Parse DEFAULT= and optional OVERRIDE=
    let mut default_val = None;
    let mut override_val = None;

    for part in rest.split_whitespace() {
        if let Some(val) = part.strip_prefix("DEFAULT=") {
            default_val = Some(sanitize_env_var_value(val));
        } else if let Some(val) = part.strip_prefix("OVERRIDE=") {
            override_val = Some(sanitize_env_var_value(val));
        }
    }

    let default_val = default_val?;

    let final_val = if let Some(ov) = override_val {
        format!("{}:{}", ov, default_val)
    } else {
        default_val
    };

    Some((key.to_string(), final_val))
}

/// Validate an environment variable key.
fn is_valid_env_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    key.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && (key.as_bytes()[0].is_ascii_uppercase()
            || key.as_bytes()[0].is_ascii_digit()
            || key.as_bytes()[0] == b'_')
}

/// Sanitize an env var value by stripping comments and trimming quotes.
fn sanitize_env_var_value(val: &str) -> String {
    strip_comments(&trim_quotes(val))
}

/// Trim surrounding quotes (single or double) from a value.
fn trim_quotes(val: &str) -> String {
    let val = val.trim();
    if val.len() >= 2 {
        let first = val.as_bytes()[0];
        let last = val.as_bytes()[val.len() - 1];
        if (first == b'"' || first == b'\'') && last == first {
            return val[1..val.len() - 1].to_string();
        }
        // Opening quote with no matching close: strip open quote only
        if first == b'"' || first == b'\'' {
            return val[1..].to_string();
        }
    }
    val.to_string()
}

/// Strip inline comments (everything after #).
fn strip_comments(val: &str) -> String {
    match val.find('#') {
        Some(idx) => val[..idx].to_string(),
        None => val.to_string(),
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_environment_line_simple() {
        let (k, v) = parse_environment_line("PATH=/usr/bin").unwrap();
        assert_eq!(k, "PATH");
        assert_eq!(v, "/usr/bin");
    }

    #[test]
    fn test_parse_environment_line_export() {
        let (k, v) = parse_environment_line("export HOME=/home/user").unwrap();
        assert_eq!(k, "HOME");
        assert_eq!(v, "/home/user");
    }

    #[test]
    fn test_parse_environment_line_quoted() {
        let (k, v) = parse_environment_line("MSG=\"hello world\"").unwrap();
        assert_eq!(k, "MSG");
        assert_eq!(v, "hello world");
    }

    #[test]
    fn test_parse_environment_line_comment() {
        assert!(parse_environment_line("# this is a comment").is_none());
    }

    #[test]
    fn test_parse_environment_line_empty() {
        assert!(parse_environment_line("").is_none());
        assert!(parse_environment_line("   ").is_none());
    }

    #[test]
    fn test_parse_environment_line_inline_comment() {
        let (k, v) = parse_environment_line("VAR=value # comment").unwrap();
        assert_eq!(k, "VAR");
        assert_eq!(v, "value ");
    }

    #[test]
    fn test_parse_environment_conf_line() {
        let (k, v) = parse_environment_conf_line("PATH DEFAULT=/usr/bin").unwrap();
        assert_eq!(k, "PATH");
        assert_eq!(v, "/usr/bin");
    }

    #[test]
    fn test_parse_environment_conf_line_with_override() {
        let (k, v) =
            parse_environment_conf_line("PATH DEFAULT=/usr/bin OVERRIDE=/usr/local/bin").unwrap();
        assert_eq!(k, "PATH");
        assert_eq!(v, "/usr/local/bin:/usr/bin");
    }

    #[test]
    fn test_parse_environment_conf_line_comment() {
        assert!(parse_environment_conf_line("# comment").is_none());
    }

    #[test]
    fn test_replace_home_and_shell() {
        assert_eq!(
            replace_home_and_shell("@{HOME}/.config", "/home/user", "/bin/bash"),
            "/home/user/.config"
        );
        assert_eq!(
            replace_home_and_shell("@{SHELL}", "/home/user", "/bin/zsh"),
            "/bin/zsh"
        );
    }

    #[test]
    fn test_trim_quotes_double() {
        assert_eq!(trim_quotes("\"hello\""), "hello");
    }

    #[test]
    fn test_trim_quotes_single() {
        assert_eq!(trim_quotes("'hello'"), "hello");
    }

    #[test]
    fn test_trim_quotes_none() {
        assert_eq!(trim_quotes("hello"), "hello");
    }

    #[test]
    fn test_trim_quotes_mismatched() {
        // Mismatched quotes should NOT strip both ends
        assert_eq!(trim_quotes("\"hello'"), "hello'");
        assert_eq!(trim_quotes("'hello\""), "hello\"");
    }

    #[test]
    fn test_strip_comments() {
        assert_eq!(strip_comments("value # comment"), "value ");
        assert_eq!(strip_comments("no comment"), "no comment");
    }

    #[test]
    fn test_is_valid_env_key() {
        assert!(is_valid_env_key("PATH"));
        assert!(is_valid_env_key("_PRIVATE"));
        assert!(is_valid_env_key("MY_VAR_123"));
        assert!(!is_valid_env_key(""));
        assert!(!is_valid_env_key("my-var"));
    }

    #[test]
    fn test_sanitize_env_var_value() {
        assert_eq!(sanitize_env_var_value("\"hello\""), "hello");
        assert_eq!(sanitize_env_var_value("value # comment"), "value ");
        assert_eq!(sanitize_env_var_value("'quoted'"), "quoted");
    }

    #[test]
    fn test_parse_passwd_safe() {
        // Just verify it doesn't panic; actual parsing depends on /etc/passwd
        let _opts = parse_passwd_safe();
    }
}
