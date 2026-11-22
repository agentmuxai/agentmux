// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A stand-in for the browser, for the pane's tests: it answers the calls a pane makes (a page is made and attached, an address is
//! read back) and records every call with the session it was made on, so a test can say what the pane asked the browser to do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

use super::cdp::{encode, split_messages, Cdp};

#[derive(Debug, Clone)]
pub(super) struct Call {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

pub(super) struct FakeBrowser {
    pub cdp: Arc<Cdp>,
    calls: Arc<Mutex<Vec<Call>>>,
    addresses: Arc<Mutex<HashMap<String, (String, String)>>>,
    refused: Arc<Mutex<Vec<(String, String)>>>,
}

impl FakeBrowser {
    pub fn start() -> FakeBrowser {
        let (to_peer_w, mut to_peer_r) = duplex(1 << 20);
        let (mut from_peer_w, from_peer_r) = duplex(1 << 20);
        let cdp = Cdp::over(from_peer_r, to_peer_w);
        let calls = Arc::new(Mutex::new(Vec::<Call>::new()));
        let addresses = Arc::new(Mutex::new(HashMap::<String, (String, String)>::new()));
        let refused = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let (log, at, no) = (calls.clone(), addresses.clone(), refused.clone());
        tokio::spawn(async move {
            let mut made = 0u32;
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let Ok(n) = to_peer_r.read(&mut chunk).await else { return };
                if n == 0 {
                    return;
                }
                buffer.extend_from_slice(&chunk[..n]);
                for message in split_messages(&mut buffer) {
                    let Ok(call) = serde_json::from_slice::<Value>(&message) else { continue };
                    let method = call["method"].as_str().unwrap_or("").to_string();
                    let params = call["params"].clone();
                    let session = call.get("sessionId").and_then(Value::as_str).map(str::to_string);
                    log.lock().unwrap().push(Call { method: method.clone(), params: params.clone(), session: session.clone() });
                    let reply = match method.as_str() {
                        "Target.createTarget" => {
                            made += 1;
                            Ok(json!({ "targetId": format!("T{made}") }))
                        }
                        "Target.attachToTarget" => {
                            let target = params["targetId"].as_str().unwrap_or("").to_string();
                            if no.lock().unwrap().iter().any(|(m, t)| m == "Target.attachToTarget" && *t == target) {
                                Err("the browser would not attach")
                            } else {
                                Ok(json!({ "sessionId": format!("S-{target}") }))
                            }
                        }
                        "Page.getNavigationHistory" => {
                            let (url, title) = session.as_ref().and_then(|s| at.lock().unwrap().get(s).cloned()).unwrap_or(("about:blank".into(), String::new()));
                            Ok(json!({ "currentIndex": 0, "entries": [{ "id": 1, "url": url, "title": title }] }))
                        }
                        _ => Ok(json!({})),
                    };
                    let answer = match reply {
                        Ok(result) => json!({ "id": call["id"], "result": result }),
                        Err(message) => json!({ "id": call["id"], "error": { "code": -32000, "message": message } }),
                    };
                    if from_peer_w.write_all(&encode(&answer)).await.is_err() {
                        return;
                    }
                }
            }
        });
        FakeBrowser { cdp, calls, addresses, refused }
    }

    /// What the page on `session` reads back as its address and title.
    pub fn set_address(&self, session: &str, url: &str, title: &str) {
        self.addresses.lock().unwrap().insert(session.to_string(), (url.to_string(), title.to_string()));
    }

    /// `method` on `target` is refused by the browser.
    pub fn refuse(&self, method: &str, target: &str) {
        self.refused.lock().unwrap().push((method.to_string(), target.to_string()));
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}
