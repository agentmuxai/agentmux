// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shape a captured value must have (#47629).
//!
//! `Browser` `capture` can be told what the value should look like: a `prefix` it starts with, a `pattern`
//! (a regular expression the whole value matches). The check runs here, on the value the host read, before
//! anything is stored; a value that does not match is dropped and nothing is saved. It guards against
//! saving the wrong text (a neighbouring label, a masked placeholder, a second value) in a row the agent
//! could not read for itself.
//!
//! A guard that answers yes or no to the agent is a guess-the-secret game: a prefix tried one character at
//! a time reads a value out. So the answer never says which part of the shape failed or quotes the value,
//! and a pane that has had [`MISSES`] values refused in [`WINDOW`] is refused any more with a shape until
//! the window passes. A capture with no shape is not limited: it never answers about the value.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::{Regex, RegexBuilder};

/// The longest prefix and pattern taken.
const MAX_PREFIX: usize = 64;
const MAX_PATTERN: usize = 200;
/// How many values a pane can have refused for their shape, in [`WINDOW`], before it is refused more.
const MISSES: usize = 5;
const WINDOW: Duration = Duration::from_secs(10 * 60);

/// What a value must look like.
#[derive(Debug)]
pub(super) struct Shape {
    prefix: Option<String>,
    pattern: Option<Regex>,
}

impl Shape {
    /// The shape the agent asked for, or none when it gave neither; an error says what to fix.
    pub(super) fn new(prefix: Option<&str>, pattern: Option<&str>) -> Result<Option<Self>, String> {
        let prefix = prefix.map(str::trim).filter(|p| !p.is_empty());
        let pattern = pattern.map(str::trim).filter(|p| !p.is_empty());
        if prefix.is_none() && pattern.is_none() {
            return Ok(None);
        }
        if let Some(p) = prefix {
            if p.chars().count() > MAX_PREFIX || p.chars().any(char::is_whitespace) {
                return Err(format!("prefix is one word of at most {MAX_PREFIX} characters"));
            }
        }
        let pattern = match pattern {
            None => None,
            Some(p) if p.chars().count() > MAX_PATTERN => return Err(format!("pattern is at most {MAX_PATTERN} characters")),
            // The whole value must match, not a part of it.
            Some(p) => Some(
                RegexBuilder::new(&format!("^(?:{p})$"))
                    .size_limit(256 * 1024)
                    .build()
                    .map_err(|_| "pattern is not a valid regular expression".to_string())?,
            ),
        };
        Ok(Some(Self { prefix: prefix.map(str::to_string), pattern }))
    }

    /// Whether `value` (trimmed) has the shape.
    pub(super) fn matches(&self, value: &str) -> bool {
        let v = value.trim();
        self.prefix.as_deref().is_none_or(|p| v.starts_with(p)) && self.pattern.as_ref().is_none_or(|r| r.is_match(v))
    }
}

fn misses() -> &'static Mutex<HashMap<String, Vec<Instant>>> {
    static MISSES_BY_PANE: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
    MISSES_BY_PANE.get_or_init(Mutex::default)
}

/// Refuse a capture with a shape on a pane that has had too many values refused lately.
pub(super) fn check_not_probing(pane: &str) -> Result<(), String> {
    let mut all = misses().lock().unwrap_or_else(|p| p.into_inner());
    all.retain(|_, v| {
        v.retain(|t| t.elapsed() < WINDOW);
        !v.is_empty()
    });
    if all.get(pane).is_some_and(|v| v.len() >= MISSES) {
        return Err(format!(
            "{MISSES} captures on this pane were refused for their shape in the last {} minutes: a shape is for choosing between values, not for testing what one says. Capture without a shape, or ask the person",
            WINDOW.as_secs() / 60
        ));
    }
    Ok(())
}

/// Note a value refused for its shape on `pane`.
pub(super) fn note_miss(pane: &str) {
    misses().lock().unwrap_or_else(|p| p.into_inner()).entry(pane.to_string()).or_default().push(Instant::now());
}

#[cfg(test)]
#[path = "browser_capture_shape_tests.rs"]
mod tests;
