// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Wait block: the run stops for a time or for an event
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "Waiting").
//!
//! `{ "for": "30m" }` waits that long (`500ms`, `45s`, `30m`, `2h`, `1d`, or a
//! number of seconds); `{ "until": "2026-12-01T09:00:00Z" }` waits for a time
//! (RFC 3339); `{ "on": "label" }` waits for an event: the label says what it is
//! for, the key that ends it is made when the wait starts (`backend::drone_wait`).
//! Exactly one of the three. The scheduler does the waiting
//! (`scheduler/wait.rs`), because the wait is kept in the run's checkpoint.

use serde_json::Value;

use crate::backend::storage::drone_types::FlowNode;

/// The longest a run may be told to wait: 30 days.
pub const MAX_WAIT_MS: i64 = 30 * 24 * 3_600_000;

/// What a Wait block waits for.
#[derive(Debug, Clone, PartialEq)]
pub enum Wait {
    /// This long from when the wait starts, in milliseconds.
    For(i64),
    /// This time, in milliseconds since 1970.
    Until(i64),
    /// An event, called `label`.
    On(String),
}

/// A duration as written: `500ms`, `45s`, `30m`, `2h`, `1d` or a number of seconds.
fn duration_ms(text: &str) -> Option<i64> {
    let t = text.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (number, unit) = t.split_at(split);
    let n: f64 = number.parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let unit_ms = match unit.trim() {
        "ms" => 1.0,
        "" | "s" => 1_000.0,
        "m" => 60_000.0,
        "h" => 3_600_000.0,
        "d" => 86_400_000.0,
        _ => return None,
    };
    let ms = n * unit_ms;
    (ms <= MAX_WAIT_MS as f64).then_some(ms as i64)
}

/// The block's setting, with templates filled in by `resolve`. A time that is
/// not in the future is allowed (the wait is over at once); one more than 30
/// days from `now_ms` is not.
pub fn parse(node: &FlowNode, resolve: &dyn Fn(&str) -> String, now_ms: i64) -> Result<Wait, String> {
    let text = |key: &str| match node.data.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(resolve(s)),
        Some(other) => Some(other.to_string()),
    };
    let (f, u, o) = (text("for"), text("until"), text("on"));
    let id = &node.id;
    match (f, u, o) {
        (Some(f), None, None) => duration_ms(&f)
            .map(Wait::For)
            .ok_or_else(|| format!("{id}: `for` must be a length of time up to 30 days, like 90s, 30m, 2h or 1d, and it is {f:?}")),
        (None, Some(u), None) => {
            let at = chrono::DateTime::parse_from_rfc3339(u.trim())
                .map_err(|_| format!("{id}: `until` must be a time like 2026-12-01T09:00:00Z, and it is {u:?}"))?
                .timestamp_millis();
            if at - now_ms > MAX_WAIT_MS {
                return Err(format!("{id}: `until` is more than 30 days away, the longest a run waits"));
            }
            Ok(Wait::Until(at))
        }
        (None, None, Some(o)) if !o.trim().is_empty() => Ok(Wait::On(o.trim().to_string())),
        (None, None, Some(_)) => Err(format!("{id}: `on` must say what the run waits for")),
        (None, None, None) => Err(format!("{id}: a Wait block needs `for`, `until` or `on`")),
        _ => Err(format!("{id}: a Wait block takes only one of `for`, `until` and `on`")),
    }
}

/// The block setting is well formed, as far as it can be told before the run:
/// one that is a template is checked when the wait starts.
pub fn check(node: &FlowNode) -> Result<(), String> {
    let templated = ["for", "until", "on"].iter().any(|k| node.data.get(*k).and_then(Value::as_str).is_some_and(|s| s.contains("{{")));
    if templated {
        return Ok(());
    }
    parse(node, &|t| t.to_string(), agentmux_common::time::now_ms()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;
    use serde_json::json;

    fn wait_node(extra: Value) -> FlowNode {
        let mut data = json!({ "kind": "wait" });
        data.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        FlowNode { id: "w".to_string(), position: NodePosition::default(), data, node_type: None }
    }

    fn read(extra: Value) -> Result<Wait, String> {
        parse(&wait_node(extra), &|t| t.replace("{{n}}", "5m"), 1_796_000_000_000)
    }

    #[test]
    fn a_length_of_time_is_read_in_the_units_people_write() {
        for (text, ms) in [("500ms", 500), ("45s", 45_000), ("30m", 1_800_000), ("2h", 7_200_000), ("1d", 86_400_000), ("90", 90_000), (" 1.5h ", 5_400_000), ("0s", 0)] {
            assert_eq!(read(json!({ "for": text })), Ok(Wait::For(ms)), "{text}");
        }
        assert_eq!(read(json!({ "for": 20 })), Ok(Wait::For(20_000)), "a number is seconds");
        assert_eq!(read(json!({ "for": "{{n}}" })), Ok(Wait::For(300_000)), "templates are filled in");
    }

    #[test]
    fn a_length_that_is_not_one_is_refused_and_says_what_to_write() {
        for bad in ["", "soon", "-5m", "5 weeks", "31d", "NaNs"] {
            let why = read(json!({ "for": bad })).unwrap_err();
            assert!(why.contains("`for` must be a length of time up to 30 days"), "{bad}: {why}");
        }
    }

    #[test]
    fn a_time_is_read_as_rfc_3339_and_may_already_have_passed() {
        assert_eq!(read(json!({ "until": "2026-12-01T09:00:00Z" })), Ok(Wait::Until(1_796_115_600_000)));
        assert_eq!(read(json!({ "until": "2026-12-01T10:00:00+01:00" })), Ok(Wait::Until(1_796_115_600_000)));
        assert!(matches!(read(json!({ "until": "1999-01-01T00:00:00Z" })), Ok(Wait::Until(_))));
        assert!(read(json!({ "until": "tomorrow" })).unwrap_err().contains("`until` must be a time like"));
        assert!(read(json!({ "until": "2999-01-01T00:00:00Z" })).unwrap_err().contains("more than 30 days"));
    }

    #[test]
    fn an_event_wait_needs_a_label_and_exactly_one_setting_is_given() {
        assert_eq!(read(json!({ "on": "deploy.finished" })), Ok(Wait::On("deploy.finished".to_string())));
        assert!(read(json!({ "on": "  " })).unwrap_err().contains("`on` must say"));
        assert!(read(json!({})).unwrap_err().contains("needs `for`, `until` or `on`"));
        assert!(read(json!({ "for": "1m", "on": "x" })).unwrap_err().contains("only one of"));
    }
}
