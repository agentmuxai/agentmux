// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `.env` files and shell `export` lines: each `NAME=value` is an API key, with
//! the variable name as its label and suggested environment variable
//! (spec §8.4).

use super::{Imported, Incoming, ReadError, Source, MAX_ITEMS};

/// A line's `NAME` and raw value, or `None` for a blank line, a comment or
/// anything else.
fn split_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").map(str::trim_start).unwrap_or(line);
    let (name, value) = line.split_once('=')?;
    let name = name.trim();
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() || first == '_').then_some(())?;
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_').then_some(())?;
    Some((name, value.trim()))
}

/// The value of a line: quoted (single quotes are literal, double quotes take
/// `\n`, `\t`, `\"` and `\\`), or bare up to a ` #` comment.
fn value_of(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(q @ ('"' | '\'')) => {
            let mut out = String::new();
            let mut escaped = false;
            for c in chars {
                if q == '"' && escaped {
                    out.push(match c {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    escaped = false;
                } else if q == '"' && c == '\\' {
                    escaped = true;
                } else if c == q {
                    break;
                } else {
                    out.push(c);
                }
            }
            out
        }
        _ => raw.split(" #").next().unwrap_or("").trim().to_string(),
    }
}

/// Whether text is a `.env` file: at least one `NAME=value` line, and every
/// non-blank, non-comment line is one.
pub fn looks_like_env(text: &str) -> bool {
    let mut any = false;
    for line in text.lines().filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#')).take(200) {
        if split_line(line).is_none() {
            return false;
        }
        any = true;
    }
    any
}

pub fn read(text: &str) -> Result<Imported, ReadError> {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let Some((name, raw)) = split_line(line) else { continue };
        if entries.len() + skipped.len() >= MAX_ITEMS {
            return Err(format!("this file has more than {MAX_ITEMS} variables, more than the Vault imports").into());
        }
        let value = value_of(raw);
        if value.is_empty() {
            skipped.push((name.to_string(), "it has no value".to_string()));
            continue;
        }
        if !seen.insert(name.to_string()) {
            // The last assignment is the one that counts in a shell: the later one wins.
            entries.retain(|e: &Incoming| e.label != name);
        }
        entries.push(Incoming::new("api-key", name).part("value", &value).field("env_name", name));
    }
    if entries.is_empty() && skipped.is_empty() {
        return Err("no NAME=value lines were found".into());
    }
    Ok(Imported { source: Source::Env, entries, skipped, plaintext: true })
}
