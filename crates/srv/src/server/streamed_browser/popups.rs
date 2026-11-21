// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who owns the windows a page opens. The browser tells everyone about every new page and every page that ends; each pane
//! claims the pages it shows, so a window opened by one of a pane's pages is told to that pane (which shows it), and a window
//! nobody owns is closed as it appears, as before. A pane never hears of another pane's windows.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

/// What a pane is told about its windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetEvent {
    /// A page of the pane opened a window (`target_id`); `opener_id` is the page that opened it.
    Opened { target_id: String, opener_id: String, url: String },
    /// A window of the pane ended (its page closed it, or it crashed).
    Gone { target_id: String },
}

fn owners() -> &'static Mutex<HashMap<String, UnboundedSender<TargetEvent>>> {
    static OWNERS: OnceLock<Mutex<HashMap<String, UnboundedSender<TargetEvent>>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// This pane shows `target_id`: windows it opens, and its end, are told to `tx`.
pub fn claim(target_id: &str, tx: UnboundedSender<TargetEvent>) {
    owners().lock().unwrap_or_else(|e| e.into_inner()).insert(target_id.to_string(), tx);
}

pub fn release(target_id: &str) {
    owners().lock().unwrap_or_else(|e| e.into_inner()).remove(target_id);
}

/// What to do with a browser-level event. The window to close, when nobody owns a page that has an opener.
pub fn route(method: &str, params: &Value) -> Option<String> {
    match method {
        "Target.targetCreated" => {
            let info = &params["targetInfo"];
            let opener = info.get("openerId").and_then(Value::as_str).filter(|o| !o.is_empty())?;
            if info["type"] != "page" {
                return None;
            }
            let target = info["targetId"].as_str()?;
            let url = info["url"].as_str().unwrap_or("").to_string();
            let mut owners = owners().lock().unwrap_or_else(|e| e.into_inner());
            match owners.get(opener) {
                Some(tx) if tx.send(TargetEvent::Opened { target_id: target.to_string(), opener_id: opener.to_string(), url }).is_ok() => None,
                Some(_) => {
                    // The pane that owned the opener is gone.
                    owners.remove(opener);
                    Some(target.to_string())
                }
                None => Some(target.to_string()),
            }
        }
        "Target.targetDestroyed" => {
            let target = params["targetId"].as_str()?;
            let owners = owners().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(tx) = owners.get(target) {
                let _ = tx.send(TargetEvent::Gone { target_id: target.to_string() });
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::mpsc;

    fn created(target: &str, opener: Option<&str>, kind: &str) -> Value {
        let mut info = json!({ "targetId": target, "type": kind, "url": "https://idp.example/auth" });
        if let Some(o) = opener {
            info["openerId"] = json!(o);
        }
        json!({ "targetInfo": info })
    }

    #[test]
    fn a_window_a_pane_s_page_opens_is_told_to_that_pane_and_is_not_closed() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        claim("t-owned-1", tx);
        assert_eq!(route("Target.targetCreated", &created("t-pop-1", Some("t-owned-1"), "page")), None);
        assert_eq!(rx.try_recv().unwrap(), TargetEvent::Opened { target_id: "t-pop-1".into(), opener_id: "t-owned-1".into(), url: "https://idp.example/auth".into() });
        release("t-owned-1");
    }

    #[test]
    fn a_window_nobody_owns_is_closed_and_one_without_an_opener_or_a_page_is_left_alone() {
        assert_eq!(route("Target.targetCreated", &created("t-pop-2", Some("t-unowned"), "page")).as_deref(), Some("t-pop-2"));
        assert_eq!(route("Target.targetCreated", &created("t-new", None, "page")), None);
        assert_eq!(route("Target.targetCreated", &created("t-w", Some("t-unowned"), "service_worker")), None);
        assert_eq!(route("Target.targetCreated", &json!({})), None);
        assert_eq!(route("Target.attachedToTarget", &json!({})), None);
    }

    #[test]
    fn a_pane_hears_only_of_its_own_windows() {
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        claim("t-a-3", a_tx);
        claim("t-b-3", b_tx);
        route("Target.targetCreated", &created("t-pop-3", Some("t-b-3"), "page"));
        assert!(a_rx.try_recv().is_err());
        assert!(matches!(b_rx.try_recv(), Ok(TargetEvent::Opened { .. })));
        route("Target.targetDestroyed", &json!({ "targetId": "t-a-3" }));
        assert_eq!(a_rx.try_recv().unwrap(), TargetEvent::Gone { target_id: "t-a-3".into() });
        assert!(b_rx.try_recv().is_err());
        release("t-a-3");
        release("t-b-3");
    }

    #[test]
    fn a_window_whose_pane_has_gone_is_closed_and_the_claim_is_dropped() {
        let (tx, rx) = mpsc::unbounded_channel();
        claim("t-gone-4", tx);
        drop(rx);
        assert_eq!(route("Target.targetCreated", &created("t-pop-4", Some("t-gone-4"), "page")).as_deref(), Some("t-pop-4"));
        assert_eq!(route("Target.targetCreated", &created("t-pop-5", Some("t-gone-4"), "page")).as_deref(), Some("t-pop-5"));
    }
}
