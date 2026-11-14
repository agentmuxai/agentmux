// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! What `Browser` `key` says about a custom list a key acted on (#47619).

/// What a key did to a custom list that kept focus (`was` is how it stood before, for what differs):
/// it closed or opened, and the option it shows changed. A list's value is its trigger's text, so a key
/// that took an option shows here, not as a changed field (#47619).
pub(super) fn list_effect_text(was: Option<&serde_json::Value>, after: &serde_json::Value) -> Option<String> {
    let was = was?.as_object()?;
    let mut parts = Vec::new();
    if let Some(before) = was.get("expanded").and_then(|v| v.as_bool()) {
        parts.push(if before { "The list closed.".to_string() } else { "The list opened.".to_string() });
    }
    if let Some(before) = was.get("text") {
        let now = after.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let before = before.as_str().unwrap_or("");
        parts.push(format!(
            "What it shows changed from \"{before}\" to \"{now}\": a key chose that option, check it is the one you meant."
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::super::browser::key_result_text;
    use serde_json::json;

        #[test]
        fn a_list_that_closed_or_took_an_option_says_so() {
            let after = json!({ "tag": "button", "text": "AGENTMUX", "expanded": false, "element": "node:7" });
            let took = json!({ "after": after, "changed": true, "focus_moved": false, "was": { "text": "Choose an option", "expanded": true } });
            let text = key_result_text(Some(&took));
            assert!(text.contains("The list closed."), "{text}");
            assert!(text.contains("changed from \"Choose an option\" to \"AGENTMUX\""), "{text}");
            assert!(!text.contains("didn't change"), "{text}");
            let closed = json!({ "after": { "tag": "button", "text": "Choose an option", "expanded": false }, "changed": false, "focus_moved": false, "was": { "expanded": true } });
            let text = key_result_text(Some(&closed));
            assert!(text.starts_with("Dispatched. The list closed. Now: "), "{text}");
            assert!(!text.contains("changed from"), "{text}");
            let opened = json!({ "after": { "tag": "button", "text": "A", "expanded": true }, "changed": false, "was": { "expanded": false } });
            assert!(key_result_text(Some(&opened)).contains("The list opened."));
        }
}
