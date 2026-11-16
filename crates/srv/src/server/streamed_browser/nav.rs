// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The address bar's side of a streamed pane: where the page is, whether it can go back or forward,
//! and the moves the bar asks for. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4.

use serde_json::{json, Value};

use super::cdp::Cdp;
use super::protocol::{check_url, Go};

/// The `nav` message for the pane: its address, title and what the buttons may do.
pub fn nav_message(history: &Value, loading: bool) -> Value {
    let entries = history.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    let at = history.get("currentIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
    let here = entries.get(at);
    let text = |key: &str| here.and_then(|e| e.get(key)).and_then(Value::as_str).unwrap_or("").to_string();
    json!({
        "t": "nav",
        "url": text("url"),
        "title": text("title"),
        "canGoBack": at > 0,
        "canGoForward": at + 1 < entries.len(),
        "loading": loading,
    })
}

/// The entry id to go to, one step back (`-1`) or forward (`1`) in a history, when there is one.
pub fn history_step(history: &Value, step: i64) -> Option<i64> {
    let entries = history.get("entries")?.as_array()?;
    let at = history.get("currentIndex")?.as_i64()?;
    let target = usize::try_from(at + step).ok()?;
    entries.get(target)?.get("id")?.as_i64()
}

pub async fn history(cdp: &Cdp, session: &str) -> Value {
    cdp.call("Page.getNavigationHistory", json!({}), Some(session)).await.unwrap_or(Value::Null)
}

/// Do what the bar asked. A refused address is an `Err` in plain words.
pub async fn go(cdp: &Cdp, session: &str, go: Go, url: &str) -> Result<(), String> {
    match go {
        Go::Url => {
            let url = check_url(url)?;
            let answer = cdp.call("Page.navigate", json!({ "url": url }), Some(session)).await?;
            match answer.get("errorText").and_then(Value::as_str) {
                Some(e) if !e.is_empty() => Err(format!("the page could not be reached ({e})")),
                _ => Ok(()),
            }
        }
        Go::Back | Go::Forward => {
            let h = history(cdp, session).await;
            let step = if go == Go::Back { -1 } else { 1 };
            match history_step(&h, step) {
                Some(id) => cdp.call("Page.navigateToHistoryEntry", json!({ "entryId": id }), Some(session)).await.map(|_| ()),
                None => Ok(()),
            }
        }
        Go::Reload => cdp.call("Page.reload", json!({}), Some(session)).await.map(|_| ()),
        Go::Stop => cdp.call("Page.stopLoading", json!({}), Some(session)).await.map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Value {
        json!({ "currentIndex": 1, "entries": [
            { "id": 7, "url": "https://a.test/", "title": "A" },
            { "id": 9, "url": "https://b.test/", "title": "B" },
            { "id": 12, "url": "https://c.test/", "title": "C" },
        ]})
    }

    #[test]
    fn the_bar_shows_where_the_page_is_and_what_the_buttons_may_do() {
        let m = nav_message(&history(), true);
        assert_eq!(m["url"], "https://b.test/");
        assert_eq!(m["title"], "B");
        assert_eq!((m["canGoBack"].as_bool(), m["canGoForward"].as_bool(), m["loading"].as_bool()), (Some(true), Some(true), Some(true)));
        let first = json!({ "currentIndex": 0, "entries": [{ "id": 1, "url": "about:blank", "title": "" }] });
        let m = nav_message(&first, false);
        assert_eq!((m["canGoBack"].as_bool(), m["canGoForward"].as_bool()), (Some(false), Some(false)));
        assert_eq!(nav_message(&Value::Null, false)["url"], "");
    }

    #[test]
    fn back_and_forward_find_the_neighbouring_entry_or_none_at_the_end() {
        assert_eq!(history_step(&history(), -1), Some(7));
        assert_eq!(history_step(&history(), 1), Some(12));
        let at_end = json!({ "currentIndex": 2, "entries": history()["entries"] });
        assert_eq!(history_step(&at_end, 1), None);
        let at_start = json!({ "currentIndex": 0, "entries": history()["entries"] });
        assert_eq!(history_step(&at_start, -1), None);
        assert_eq!(history_step(&Value::Null, -1), None);
    }
}
