// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "An agent is waiting for you", announced by srv for the calls to action it
//! holds open: a browser hand-off or approval, an SSH consent or other host
//! question, a widget install
//! (docs/reports/REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §2).
//!
//! Each one publishes `userattention` with `active: true` when it starts and
//! `active: false` when it ends, however it ends: the guard's `Drop` sends
//! the end. The frontend's waiting-for-you registry turns them into the
//! waiting tone, the tab flash and the OS notification for the pane, in the
//! windows that show it (`window_ids`; empty means every window).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use super::mps::{Broker, MuxEvent, EVENT_USER_ATTENTION};
use super::storage::store::Store;

/// The announcements open now, for a window that starts while one is.
fn open() -> &'static Mutex<HashMap<String, Value>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

/// Every open announcement (`active: true`), oldest first.
pub fn open_requests() -> Vec<Value> {
    let mut v: Vec<Value> = open().lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    v.sort_by_key(|a| a["since_ms"].as_u64().unwrap_or(0));
    v
}

/// The request is open while this lives.
pub struct Asking {
    broker: Arc<Broker>,
    payload: Value,
}

impl Asking {
    /// Announces a call to action in pane `block_id` (empty: no pane) of
    /// kind `kind` (`browser`, `consent`, `widget`), described by `text`.
    pub fn start(broker: &Arc<Broker>, store: &Store, block_id: &str, kind: &str, text: &str) -> Self {
        let window_ids = if block_id.is_empty() {
            Vec::new()
        } else {
            super::notify::router::resolve_click_target(store, block_id).map(|t| t.window_ids).unwrap_or_default()
        };
        let payload = json!({
            "key": uuid::Uuid::new_v4().to_string(),
            "block_id": block_id,
            "kind": kind,
            "text": text.chars().take(200).collect::<String>(),
            "window_ids": window_ids,
            "since_ms": agentmux_common::time::now_ms_u64(),
        });
        let mut opened = payload.clone();
        opened["active"] = json!(true);
        open().lock().unwrap_or_else(|p| p.into_inner()).insert(payload["key"].as_str().unwrap_or_default().to_string(), opened);
        publish(broker, &payload, true);
        Self { broker: broker.clone(), payload }
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        open().lock().unwrap_or_else(|p| p.into_inner()).remove(self.payload["key"].as_str().unwrap_or_default());
        publish(&self.broker, &self.payload, false);
    }
}

fn publish(broker: &Broker, payload: &Value, active: bool) {
    let mut data = payload.clone();
    data["active"] = json!(active);
    broker.publish(MuxEvent {
        event: EVENT_USER_ATTENTION.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: Some(data),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn announces_the_start_and_always_the_end() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let broker = Arc::new(Broker::new());
        let log = seen.clone();
        broker.add_observer(Arc::new(move |ev| {
            if ev.event == EVENT_USER_ATTENTION {
                log.lock().unwrap().push(ev.data.clone().unwrap_or_default());
            }
        }));
        let store = Store::open(Path::new(":memory:")).unwrap();
        {
            let _asking = Asking::start(&broker, &store, "", "consent", "Agent access to area54");
            let s = seen.lock().unwrap();
            assert_eq!(s.len(), 1);
            assert_eq!((s[0]["active"].clone(), s[0]["kind"].clone()), (json!(true), json!("consent")));
            assert_eq!(s[0]["window_ids"], json!([]), "no pane: every window");
            let key = s[0]["key"].clone();
            assert!(open_requests().iter().any(|a| a["key"] == key && a["active"] == json!(true)), "listed while open");
        }
        assert!(open_requests().iter().all(|a| a["key"] != seen.lock().unwrap()[0]["key"]), "gone once ended");
        let s = seen.lock().unwrap();
        assert_eq!(s.len(), 2, "the end is announced when it's dropped");
        assert_eq!(s[1]["active"], json!(false));
        assert_eq!(s[1]["key"], s[0]["key"]);
    }
}
