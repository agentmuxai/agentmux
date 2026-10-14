// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Navigate a recycled window's page by REPLACING its history entry.
//!
//! A recycled window (demoted into the pool, or parked and blanked) is
//! navigated away from the page it hosted. A plain `load_url` adds a history
//! entry, which makes the old page eligible for Chromium's back/forward cache,
//! and the cache keeps it (up to six per window, the whole workspace app with
//! its DOM and heap): in the lab, 24 demotes left every recycled pool window
//! at 16 documents and ~10,800 DOM nodes, against 1 document and 143 nodes
//! with the cache off (docs/specs/SPEC_WINDOW_CHURN_RENDERER_LEAK_2026_10_13.md
//! §3.2, §4). AgentMux never navigates back in these windows. A replacing
//! navigation leaves nothing to restore, so nothing is cached, while browser
//! panes keep the cache for their own Back/Forward.

/// The script that navigates a page to `url` by replacing its history entry.
pub(crate) fn replace_page_script(url: &str) -> String {
    // A JSON string is a valid JS string literal.
    format!(
        "location.replace({})",
        serde_json::to_string(url).unwrap_or_else(|_| "\"about:blank\"".to_string())
    )
}

/// Navigate `frame` to `url`, replacing its history entry (see
/// `replace_page_script`).
pub(crate) fn replace_page(frame: &cef::Frame, url: &str) {
    use cef::ImplFrame;
    let code = cef::CefString::from(replace_page_script(url).as_str());
    let empty = cef::CefString::from("");
    frame.execute_java_script(Some(&code), Some(&empty), 0);
}

#[cfg(test)]
mod tests {
    use super::replace_page_script;

    #[test]
    fn replaces_rather_than_pushes() {
        assert_eq!(replace_page_script("about:blank"), r#"location.replace("about:blank")"#);
    }

    #[test]
    fn the_url_is_a_js_string_literal() {
        let js = replace_page_script(r#"http://localhost:5173/?pool=1&label=window-pool-1"#);
        assert_eq!(js, r#"location.replace("http://localhost:5173/?pool=1&label=window-pool-1")"#);
        // Quotes and backslashes can't break out of the literal.
        let js = replace_page_script(r#"x"); alert(1); ("#);
        assert!(js.starts_with("location.replace(\"") && js.ends_with("\")"), "{js}");
        assert!(!js.contains(r#"x"); alert"#), "{js}");
    }
}
