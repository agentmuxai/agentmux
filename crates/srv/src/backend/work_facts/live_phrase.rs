// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The line an agent sets for itself: what it is doing now, in its own words.
//!
//! The agent's own model knows what it is doing, so it says so (`Presence`
//! action `status`) at the start of a task, when the plan changes, when it is
//! blocked or waiting, and when it is done. The line is kept in the agent's
//! block meta with the time it was set, and the Swarm row and `work.goal` read
//! it before anything a model paraphrases from an old prompt. A task claim
//! writes the task's title in the same place (source `task`), which retires a
//! line from the task before. docs/reports/REPORT_SWARM_PHRASE_ACCURATE_SPECIFIC_CURRENT_2026_11_02.md
//! section 0.

use serde_json::{json, Map, Value};

use crate::backend::obj::{self, MetaMapType};

/// The line itself.
pub const META_PHRASE: &str = "term:live_phrase";
/// When it was set, epoch milliseconds.
pub const META_PHRASE_AT: &str = "term:live_phrase_at";
/// `working`, `blocked`, `waiting` or `done`; empty when the agent gave none.
pub const META_PHRASE_STATE: &str = "term:live_phrase_state";
/// Who wrote it: [`SRC_AGENT`] or [`SRC_TASK`].
pub const META_PHRASE_SRC: &str = "term:live_phrase_src";

/// The agent set the line itself.
pub const SRC_AGENT: &str = "agent";
/// srv wrote the title of the task the agent just claimed.
pub const SRC_TASK: &str = "task";

/// The longest line kept, in characters. The Swarm row shows one line.
pub const MAX_CHARS: usize = 80;

/// The states an agent may give with its line.
pub const STATES: &[&str] = &["working", "blocked", "waiting", "done"];

/// Text that is another party's voice, never an agent's own status.
const INJECTED: &[&str] = &["[jekt", "[broadcast", "[agentmux]", "[supervisor", "# session context"];

/// A line an agent set, or the task title srv set on its claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub text: String,
    pub at_ms: u64,
    pub state: Option<String>,
    pub src: String,
}

/// `raw` as it is kept: one line (control characters and line breaks become
/// spaces, whitespace runs collapse) of at most [`MAX_CHARS`] characters,
/// cut at a word where one is near the end. The flag says whether it was cut.
/// An empty line, or one that carries another party's marker, is refused with
/// the reason, which the agent reads.
pub fn clean_line(raw: &str) -> Result<(String, bool), String> {
    let flat: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return Err("line is empty: say what you are doing in 5 to 10 words, verb first".to_string());
    }
    let lower = flat.to_lowercase();
    if INJECTED.iter().any(|m| lower.contains(m)) {
        return Err("line carries a message marker ([JEKT, [BROADCAST, [AgentMux]): say what you are doing in your own words".to_string());
    }
    if flat.chars().count() <= MAX_CHARS {
        return Ok((flat, false));
    }
    let cut: String = flat.chars().take(MAX_CHARS - 1).collect();
    // A word boundary, unless that would throw away most of the line.
    let at = cut.rfind(' ').filter(|&i| cut[..i].chars().count() >= MAX_CHARS / 2).unwrap_or(cut.len());
    Ok((format!("{}…", cut[..at].trim_end_matches(|c: char| c.is_ascii_punctuation() || c == ' ')), true))
}

/// The state an agent gave, checked: `None` for none, an error naming the
/// states for anything else.
pub fn clean_state(raw: Option<&str>) -> Result<Option<&'static str>, String> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else { return Ok(None) };
    STATES
        .iter()
        .find(|k| k.eq_ignore_ascii_case(s))
        .map(|k| Some(*k))
        .ok_or_else(|| format!("state must be one of {}", STATES.join(", ")))
}

/// The meta that sets the line: pass it to `object.UpdateObjectMeta`.
pub fn set_meta(line: &str, state: Option<&str>, src: &str, at_ms: i64) -> Value {
    json!({
        META_PHRASE: line,
        META_PHRASE_AT: at_ms,
        META_PHRASE_STATE: state.unwrap_or(""),
        META_PHRASE_SRC: src,
    })
}

/// The meta that clears the line.
pub fn clear_meta() -> Value {
    let mut m = Map::new();
    for k in [META_PHRASE, META_PHRASE_STATE, META_PHRASE_SRC] {
        m.insert(k.to_string(), Value::String(String::new()));
    }
    m.insert(META_PHRASE_AT.to_string(), json!(0));
    Value::Object(m)
}

/// The line in an agent's block meta, if it has one.
pub fn declared_of(meta: &MetaMapType) -> Option<Declared> {
    let text = obj::meta_get_string(meta, META_PHRASE, "");
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let at_ms = meta.get(META_PHRASE_AT).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))).unwrap_or(0);
    let state = Some(obj::meta_get_string(meta, META_PHRASE_STATE, "")).filter(|s| !s.trim().is_empty());
    let src = Some(obj::meta_get_string(meta, META_PHRASE_SRC, "")).filter(|s| !s.is_empty()).unwrap_or_else(|| SRC_AGENT.to_string());
    Some(Declared { text: text.to_string(), at_ms, state, src })
}

/// The line a task claim writes: `#N title`, cut like any line.
pub fn task_line(number: Option<u64>, title: &str) -> Option<String> {
    let raw = match number {
        Some(n) => format!("#{n} {title}"),
        None => title.to_string(),
    };
    clean_line(&raw).ok().map(|(l, _)| l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, Value)]) -> MetaMapType {
        let mut m = MetaMapType::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        m
    }

    #[test]
    fn a_line_is_one_trimmed_line() {
        assert_eq!(clean_line("  Fixing the\nlogin\trace  ").unwrap(), ("Fixing the login race".to_string(), false));
    }

    #[test]
    fn an_empty_line_is_refused_with_the_reason() {
        for raw in ["", "   ", "\n\t"] {
            assert!(clean_line(raw).unwrap_err().contains("5 to 10 words"), "{raw:?}");
        }
    }

    #[test]
    fn another_partys_marker_is_never_a_status() {
        for raw in [
            "[JEKT:FROM=x] do it",
            "Done [BROADCAST:FROM=user]",
            "[AgentMux] upgraded",
            "# Session Context",
            "[supervisor:continue] go on",
        ] {
            assert!(clean_line(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn a_long_line_is_cut_at_a_word_and_says_so() {
        let long = "Writing the report on how the Swarm phrase can be set by the agent itself, with fallbacks and a measurement";
        let (line, cut) = clean_line(long).unwrap();
        assert!(cut);
        assert!(line.chars().count() <= MAX_CHARS, "{line:?}");
        assert!(line.ends_with('…'));
        assert!(!line.trim_end_matches('…').ends_with(' '));
        assert!(long.starts_with(line.trim_end_matches('…')), "{line:?}");
    }

    #[test]
    fn a_line_at_the_limit_is_kept_whole() {
        let exact = "x".repeat(MAX_CHARS);
        assert_eq!(clean_line(&exact).unwrap(), (exact, false));
        let (line, cut) = clean_line(&"y".repeat(MAX_CHARS + 1)).unwrap();
        assert!(cut);
        assert_eq!(line.chars().count(), MAX_CHARS);
    }

    #[test]
    fn a_state_is_one_of_four_or_absent() {
        assert_eq!(clean_state(None).unwrap(), None);
        assert_eq!(clean_state(Some("  ")).unwrap(), None);
        assert_eq!(clean_state(Some("Blocked")).unwrap(), Some("blocked"));
        assert!(clean_state(Some("thinking")).unwrap_err().contains("working, blocked, waiting, done"));
    }

    #[test]
    fn set_meta_and_declared_of_round_trip() {
        let v = set_meta("Fixing the login race", Some("working"), SRC_AGENT, 1_700_000_000_000);
        let m: MetaMapType = v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(
            declared_of(&m),
            Some(Declared {
                text: "Fixing the login race".into(),
                at_ms: 1_700_000_000_000,
                state: Some("working".into()),
                src: "agent".into()
            })
        );
    }

    #[test]
    fn a_cleared_line_is_no_line() {
        let m: MetaMapType = clear_meta().as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(declared_of(&m), None);
        assert_eq!(declared_of(&MetaMapType::new()), None);
    }

    #[test]
    fn a_line_without_a_source_is_the_agents_own() {
        let m = meta(&[(META_PHRASE, json!("Reviewing PR 6190")), (META_PHRASE_AT, json!(5))]);
        let d = declared_of(&m).unwrap();
        assert_eq!((d.src.as_str(), d.state), ("agent", None));
    }

    #[test]
    fn a_task_claim_writes_its_number_and_title() {
        assert_eq!(task_line(Some(20816), "Build: the Swarm phrase").as_deref(), Some("#20816 Build: the Swarm phrase"));
        assert_eq!(task_line(None, "Fix the login race").as_deref(), Some("Fix the login race"));
        assert!(task_line(Some(1), &"w ".repeat(80)).unwrap().chars().count() <= MAX_CHARS);
    }
}
