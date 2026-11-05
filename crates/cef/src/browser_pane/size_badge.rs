// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `WxH` badge shown while a splitter is dragged, drawn inside a browser pane's page.
//!
//! A native pane covers anything the app's page draws over it, so the badge (a DOM element of
//! the app) used to get a strip at the pane's bottom that the page stopped short of. For panes
//! that are Views overlays on Windows (`pane_backend`) the page is the pane's own: this adds a small
//! fixed element at its bottom left, with the colours the app's badge has, and removes it when
//! the drag ends. The page keeps its full size and the badge background is only as big as the
//! text.

use std::sync::Arc;

use cef::{CefString, ImplBrowser, ImplFrame};
use serde_json::Value;

use crate::state::AppState;

/// The badge's element id in the page.
const ELEMENT_ID: &str = "__agentmux_size_badge";

/// The longest string taken from the app for any badge field.
const MAX_FIELD: usize = 160;

/// The script that shows `badge` (`{ text, color, background, border }`) in the page, or removes the
/// badge for `None`. Strings go through JSON, and the element's style is set through the style
/// object, so a page's content security policy does not stop it.
pub(crate) fn script_for(badge: Option<&Value>) -> String {
    let field = |key: &str| -> Value {
        let text = badge.and_then(|b| b.get(key)).and_then(Value::as_str).unwrap_or("");
        Value::String(text.chars().take(MAX_FIELD).collect())
    };
    let arg = match badge {
        Some(_) => serde_json::json!({
            "text": field("text"),
            "color": field("color"),
            "background": field("background"),
            "border": field("border"),
        })
        .to_string(),
        None => "null".to_string(),
    };
    format!(
        "(function(b){{var e=document.getElementById('{ELEMENT_ID}');\
         if(!b){{if(e)e.remove();return;}}\
         if(!e){{e=document.createElement('div');e.id='{ELEMENT_ID}';var s=e.style;\
         s.position='fixed';s.left='8px';s.bottom='8px';s.zIndex='2147483647';s.padding='2px 6px';\
         s.font='500 12px/1.4 ui-monospace,Consolas,monospace';s.letterSpacing='0.02em';\
         s.border='1px solid';s.pointerEvents='none';s.userSelect='none';s.whiteSpace='nowrap';\
         (document.body||document.documentElement).appendChild(e);}}\
         var t=e.style;t.color=b.color;t.background=b.background;t.borderColor=b.border;e.textContent=b.text;}})({arg});"
    )
}

/// `browser_pane_set_badge`: `{ block_id, badge }`, `badge` an object to show or anything else to remove.
pub(crate) fn handle(state: &Arc<AppState>, args: &Value) -> Value {
    let block_id = args.get("block_id").and_then(Value::as_str).unwrap_or("");
    set(state, block_id, args.get("badge").filter(|b| b.is_object()));
    Value::Bool(true)
}

/// Show (`Some`) or remove (`None`) the badge in pane `block_id`'s page. A pane that is not live is left alone.
pub(crate) fn set(state: &Arc<AppState>, block_id: &str, badge: Option<&Value>) {
    let Some(label) = state.live_browser_pane_label(block_id) else { return };
    let Some(frame) = state.get_browser(&label).and_then(|b| b.main_frame()) else { return };
    let code = CefString::from(script_for(badge).as_str());
    frame.execute_java_script(Some(&code), Some(&CefString::from("")), 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_badge_removes_the_element() {
        let s = script_for(None);
        assert!(s.contains("e.remove()"));
        assert!(s.ends_with("(null);"));
    }

    #[test]
    fn a_badge_carries_its_text_and_colours_as_json_strings() {
        let b = json!({ "text": "797\u{d7}395", "color": "rgb(1, 2, 3)", "background": "rgba(0, 0, 0, 0.8)", "border": "red" });
        let s = script_for(Some(&b));
        assert!(s.contains("\"text\":\"797\u{d7}395\""));
        assert!(s.contains("\"background\":\"rgba(0, 0, 0, 0.8)\""));
        assert!(s.contains(ELEMENT_ID));
    }

    #[test]
    fn a_quote_in_a_field_cannot_leave_its_string() {
        let b = json!({ "text": "\"});alert(1);(\"", "color": "", "background": "", "border": "" });
        let s = script_for(Some(&b));
        assert!(s.contains("\\\"});alert(1);(\\\""), "{s}");
    }

    #[test]
    fn a_long_field_is_cut() {
        let b = json!({ "text": "x".repeat(1000), "color": "", "background": "", "border": "" });
        let s = script_for(Some(&b));
        assert!(!s.contains(&"x".repeat(MAX_FIELD + 1)));
    }
}
