// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What changed in a form between the ask a person approved and the one the page shows when the click is made
//! (`act::check_click`): the refusal says what, as far as the person could see it, instead of only "the form
//! changed".

use serde_json::Value;

/// What differs between the ask the person approved and the one the page shows now, as far as the person could see
/// it: the destination, the button's name, or one field by its label. A change in what the person never saw (a
/// hidden or secret value) says so without the value.
pub(super) fn what_changed(approved: &Value, now: &Value) -> String {
    let text = |v: &Value, k: &str| v.get(k).map(|x| x.to_string());
    if text(approved, "action") != text(now, "action") {
        return "where it sends the form changed".to_string();
    }
    if text(approved, "what") != text(now, "what") {
        return "the button's name changed".to_string();
    }
    let fields = |v: &Value| v.get("fields").and_then(|f| f.as_array()).cloned().unwrap_or_default();
    let (old, new) = (fields(approved), fields(now));
    if old.len() != new.len() {
        return "the fields it sends changed".to_string();
    }
    for (a, b) in old.iter().zip(new.iter()) {
        if a != b {
            let label = |f: &Value| f.get(0).and_then(|l| l.as_str()).unwrap_or("").to_string();
            if label(a) != label(b) {
                return "the fields it sends changed".to_string();
            }
            let masked = agentmux_common::shown_once::mask(&label(b));
            let shown: String = masked.chars().take(60).collect();
            return format!("the value of \"{shown}\" changed");
        }
    }
    "something it sends that the page does not show, a hidden or a secret value, changed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refusal_after_approval_names_what_the_person_could_see_change() {
        let ask = |what: &str, action: &str, fields: Value| json!({ "what": what, "action": action, "fields": fields });
        let base = ask("Click \"Send\"", "https://x.test/a", json!([["Name", "Ann", false], ["Message", "hi", true]]));
        // The destination, the button's name, a field's value, the field list.
        let to = ask("Click \"Send\"", "https://x.test/b", json!([["Name", "Ann", false], ["Message", "hi", true]]));
        assert_eq!(what_changed(&base, &to), "where it sends the form changed");
        let named = ask("Click \"Pay\"", "https://x.test/a", json!([["Name", "Ann", false], ["Message", "hi", true]]));
        assert_eq!(what_changed(&base, &named), "the button's name changed");
        let value = ask("Click \"Send\"", "https://x.test/a", json!([["Name", "Ann", false], ["Message", "bye", true]]));
        assert_eq!(what_changed(&base, &value), "the value of \"Message\" changed");
        let fewer = ask("Click \"Send\"", "https://x.test/a", json!([["Name", "Ann", false]]));
        assert_eq!(what_changed(&base, &fewer), "the fields it sends changed");
        // Nothing the person saw differs: a hidden or secret value, said without it.
        assert!(what_changed(&base, &base).contains("hidden or a secret value"));
    }
}
