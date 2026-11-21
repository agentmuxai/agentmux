// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The values srv holds secret (its instance key, an agent's token, a Vault value it handed to an agent's process) are
//! never written into an agent's transcript, and never served to a paired device from one: a line that carries one is
//! stored, and shown to a device, with the value replaced by [`MARK`]. A value is matched as it is and in its ordinary
//! spellings (base64, URL-encoded, hex, as JSON holds it: `vault::scrub::forms`).
//!
//! The values are registered where srv learns them (a token is minted or loaded, the instance key is made, a Vault value
//! is handed to a process).

use std::borrow::Cow;
use std::sync::{Arc, OnceLock, RwLock};

use regex::{NoExpand, Regex, RegexBuilder};

/// What a withheld value is replaced by.
pub const MARK: &str = "[withheld]";

/// Shorter values are not registered: matching them would mangle ordinary text.
pub const MIN_LEN: usize = 8;

#[derive(Default)]
struct State {
    values: Vec<String>,
    matcher: Option<Arc<Regex>>,
    /// A value was registered since `matcher` was built.
    stale: bool,
}

fn state() -> &'static RwLock<State> {
    static STATE: OnceLock<RwLock<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// Registers `value` to be withheld from now on. A value shorter than [`MIN_LEN`] and one already registered change nothing.
pub fn register(value: &str) {
    let value = value.trim();
    if value.len() < MIN_LEN {
        return;
    }
    let mut state = state().write().unwrap_or_else(|e| e.into_inner());
    if state.values.iter().any(|v| v == value) {
        return;
    }
    state.values.push(value.to_string());
    state.stale = true;
}

/// One matcher for every registered value in every spelling, longest first so a spelling that contains another is
/// replaced whole. `None` when nothing is registered, or when the matcher could not be built.
fn build(values: &[String]) -> Option<Arc<Regex>> {
    let mut needles: Vec<String> = Vec::new();
    for value in values {
        for form in crate::backend::vault::scrub::forms(value) {
            if form.len() >= MIN_LEN && !needles.contains(&form) {
                needles.push(form);
            }
        }
    }
    if needles.is_empty() {
        return None;
    }
    needles.sort_by(|a, b| b.len().cmp(&a.len()));
    let pattern = needles.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
    match RegexBuilder::new(&pattern).size_limit(64 << 20).build() {
        Ok(matcher) => Some(Arc::new(matcher)),
        Err(e) => {
            tracing::warn!(error = %e, values = values.len(), "transcript withhold: the matcher could not be built");
            None
        }
    }
}

fn matcher() -> Option<Arc<Regex>> {
    {
        let state = state().read().unwrap_or_else(|e| e.into_inner());
        if !state.stale {
            return state.matcher.clone();
        }
    }
    let mut state = state().write().unwrap_or_else(|e| e.into_inner());
    if state.stale {
        state.matcher = build(&state.values);
        state.stale = false;
    }
    state.matcher.clone()
}

/// `text` with every registered value replaced by [`MARK`]; the text itself when it holds none.
pub fn text(text: &str) -> Cow<'_, str> {
    match matcher() {
        Some(matcher) => matcher.replace_all(text, NoExpand(MARK)),
        None => Cow::Borrowed(text),
    }
}

/// Whether `text` holds a registered value, in any of its spellings: the same matcher [`text`] replaces with.
pub fn holds(text: &str) -> bool {
    matcher().is_some_and(|matcher| matcher.is_match(text))
}

/// [`text`] over transcript bytes (NDJSON lines); bytes that are not UTF-8 are left as they are.
pub fn lines(data: &[u8]) -> Cow<'_, [u8]> {
    let Ok(as_text) = std::str::from_utf8(data) else { return Cow::Borrowed(data) };
    match text(as_text) {
        Cow::Borrowed(_) => Cow::Borrowed(data),
        Cow::Owned(replaced) => Cow::Owned(replaced.into_bytes()),
    }
}

/// [`text`] over each line, in place.
pub fn in_place(lines: &mut [String]) {
    for line in lines {
        if let Cow::Owned(replaced) = text(line) {
            *line = replaced;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    // The registry is process-wide and tests run in parallel: every test registers values of its own.

    #[test]
    fn a_registered_value_is_withheld_in_every_ordinary_spelling() {
        let value = "canary-withhold-0001-aB3dE";
        register(value);
        let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
        for shown in [
            value.to_string(),
            base64::engine::general_purpose::STANDARD.encode(value),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value),
            hex,
        ] {
            let line = format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"env: KEY={shown}"}}]}}}}"#);
            let out = text(&line);
            assert!(!out.contains(&shown), "{shown} reached the line: {out}");
            assert!(out.contains(MARK), "{out}");
            assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok(), "the line stays JSON: {out}");
        }
    }

    #[test]
    fn a_value_with_json_special_characters_is_withheld_as_json_holds_it() {
        let value = "canary\"withhold\\0002/ö-xY";
        register(value);
        let line = serde_json::json!({ "type": "user", "content": format!("echo {value}") }).to_string();
        assert!(!line.contains(value), "the stored form is escaped, not the plain value");
        let out = text(&line);
        assert!(out.contains(MARK) && !out.contains("withhold"), "{out}");
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok(), "{out}");
    }

    #[test]
    fn text_with_no_registered_value_is_returned_as_it_is() {
        register("canary-withhold-0003-zQ9");
        assert!(matches!(text("an ordinary line"), Cow::Borrowed(_)));
        assert!(matches!(lines(b"{\"type\":\"result\"}\n"), Cow::Borrowed(_)));
        assert!(matches!(lines(&[0xff, 0xfe, 0x00]), Cow::Borrowed(_)), "bytes that are not text are left alone");
    }

    #[test]
    fn a_value_too_short_to_match_safely_is_not_registered() {
        register("shorty");
        assert_eq!(text("shorty and shorty"), "shorty and shorty");
    }

    #[test]
    fn a_value_registered_later_is_withheld_from_then_on() {
        let before = "canary-withhold-0004-late";
        assert_eq!(text(&format!("key {before}")), format!("key {before}"));
        register(before);
        assert_eq!(text(&format!("key {before}")), format!("key {MARK}"));
        register(before);
        assert_eq!(text(&format!("{before}{before}")), format!("{MARK}{MARK}"), "a value registered twice is one value");
    }

    #[test]
    fn lines_and_in_place_withhold_what_text_does() {
        let value = "canary-withhold-0005-lines";
        register(value);
        let out = lines(format!("a {value}\nb {value}\n").as_bytes()).into_owned();
        assert_eq!(String::from_utf8(out).unwrap(), format!("a {MARK}\nb {MARK}\n"));
        let mut many = vec![format!("x {value}"), "clean".to_string()];
        in_place(&mut many);
        assert_eq!(many, vec![format!("x {MARK}"), "clean".to_string()]);
    }
}
