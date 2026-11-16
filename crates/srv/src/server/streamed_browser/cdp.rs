// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A small Chrome DevTools Protocol client over a pipe: JSON messages each ended by a NUL byte
//! (what `--remote-debugging-pipe` speaks). Calls are matched to their answers by id; events go
//! to the session they belong to, each session with a queue of its own, so a screencast frame is
//! never dropped (a frame that is not acknowledged stops the stream).
//! docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §3.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// How long a call may wait for its answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// The longest message read from the browser (a screencast frame is the largest by far).
const MAX_MESSAGE: usize = 32 * 1024 * 1024;

/// An event the browser sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub method: String,
    pub params: Value,
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;
type Sessions = Mutex<HashMap<String, mpsc::UnboundedSender<Event>>>;

pub struct Cdp {
    out: mpsc::UnboundedSender<Vec<u8>>,
    next: AtomicU64,
    pending: Pending,
    sessions: Sessions,
    /// Events about the browser itself (no session).
    browser: Mutex<Option<mpsc::UnboundedSender<Event>>>,
    dead: AtomicBool,
}

/// One message as it goes on the pipe.
pub fn encode(message: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).unwrap_or_default();
    bytes.push(0);
    bytes
}

/// Split what has been read into whole messages, keeping the unfinished tail.
pub fn split_messages(buffer: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(end) = buffer.iter().position(|b| *b == 0) {
        let rest = buffer.split_off(end + 1);
        buffer.pop();
        if !buffer.is_empty() {
            out.push(std::mem::take(buffer));
        }
        *buffer = rest;
    }
    out
}

impl Cdp {
    /// A client over a reader and a writer to the browser. Its tasks end with the reader.
    pub fn over<R, W>(mut reader: R, mut writer: W) -> Arc<Cdp>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let cdp = Arc::new(Cdp {
            out,
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            browser: Mutex::new(None),
            dead: AtomicBool::new(false),
        });
        tokio::spawn(async move {
            while let Some(bytes) = out_rx.recv().await {
                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        });
        let reading = cdp.clone();
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                }
                if buffer.len() > MAX_MESSAGE {
                    break;
                }
                for message in split_messages(&mut buffer) {
                    reading.dispatch(&message);
                }
            }
            reading.finish();
        });
        cdp
    }

    /// The browser has gone: every waiting call fails and every session's queue ends.
    fn finish(&self) {
        self.dead.store(true, Ordering::SeqCst);
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err("the browser stopped".into()));
        }
        self.sessions.lock().unwrap().clear();
        *self.browser.lock().unwrap() = None;
    }

    /// Give up on the browser (its start did not complete): calls fail now, and whoever holds the process ends it.
    pub fn abandon(&self) {
        self.finish();
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    fn dispatch(&self, message: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(message) else { return };
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            let answer = match value.get("error") {
                Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("the browser refused that").to_string()),
                None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
            };
            if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                let _ = tx.send(answer);
            }
            return;
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else { return };
        let event = Event { method: method.to_string(), params: value.get("params").cloned().unwrap_or(Value::Null) };
        match value.get("sessionId").and_then(Value::as_str) {
            Some(session) => {
                if let Some(tx) = self.sessions.lock().unwrap().get(session) {
                    let _ = tx.send(event);
                }
            }
            None => {
                if let Some(tx) = self.browser.lock().unwrap().as_ref() {
                    let _ = tx.send(event);
                }
            }
        }
    }

    /// The events of one session, from now on.
    pub fn subscribe(&self, session: &str) -> mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sessions.lock().unwrap().insert(session.to_string(), tx);
        rx
    }

    pub fn unsubscribe(&self, session: &str) {
        self.sessions.lock().unwrap().remove(session);
    }

    /// The events about the browser itself (a window a page opened).
    pub fn browser_events(&self) -> mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.browser.lock().unwrap() = Some(tx);
        rx
    }

    /// Send a call and do not wait for its answer: input and acknowledgements, which must keep their order
    /// and must never hold up the stream behind a page that is busy.
    pub fn send(&self, method: &str, params: Value, session: Option<&str>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            message["sessionId"] = json!(s);
        }
        let _ = self.out.send(encode(&message));
    }

    /// Call a method, on a session or on the browser, and wait for the answer.
    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        if self.is_dead() {
            return Err("the browser stopped".into());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            message["sessionId"] = json!(s);
        }
        if self.out.send(encode(&message)).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("the browser stopped".into());
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err("the browser stopped".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("the browser did not answer {method} in time"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// A fake browser: reads our calls and lets the test answer them.
    struct Peer {
        read: tokio::io::DuplexStream,
        write: tokio::io::DuplexStream,
        buffer: Vec<u8>,
    }

    impl Peer {
        async fn next_call(&mut self) -> Value {
            loop {
                if let Some(m) = split_messages(&mut self.buffer).into_iter().next() {
                    return serde_json::from_slice(&m).unwrap();
                }
                let mut chunk = [0u8; 4096];
                let n = self.read.read(&mut chunk).await.unwrap();
                self.buffer.extend_from_slice(&chunk[..n]);
            }
        }
        async fn say(&mut self, value: Value) {
            self.write.write_all(&encode(&value)).await.unwrap();
        }
    }

    fn pair() -> (Arc<Cdp>, Peer) {
        let (to_peer_w, to_peer_r) = duplex(1 << 20);
        let (from_peer_w, from_peer_r) = duplex(1 << 20);
        (Cdp::over(from_peer_r, to_peer_w), Peer { read: to_peer_r, write: from_peer_w, buffer: Vec::new() })
    }

    #[test]
    fn messages_are_split_at_the_nul_and_a_partial_one_waits() {
        let mut buf = b"{\"a\":1}\0{\"b\":2}\0{\"c\"".to_vec();
        let got = split_messages(&mut buf);
        assert_eq!(got, vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()]);
        assert_eq!(buf, b"{\"c\"".to_vec());
        buf.extend_from_slice(b":3}\0");
        assert_eq!(split_messages(&mut buf), vec![b"{\"c\":3}".to_vec()]);
        assert!(buf.is_empty());
        assert!(split_messages(&mut b"\0\0".to_vec()).is_empty());
    }

    #[tokio::test]
    async fn a_call_gets_its_own_answer_and_an_error_is_its_message() {
        let (cdp, mut peer) = pair();
        let c = cdp.clone();
        let a = tokio::spawn(async move { c.call("Page.enable", json!({}), Some("s1")).await });
        let call = peer.next_call().await;
        assert_eq!((call["method"].as_str(), call["sessionId"].as_str()), (Some("Page.enable"), Some("s1")));
        peer.say(json!({ "id": call["id"], "result": { "ok": true } })).await;
        assert_eq!(a.await.unwrap().unwrap(), json!({ "ok": true }));

        let c = cdp.clone();
        let b = tokio::spawn(async move { c.call("Page.navigate", json!({ "url": "x" }), None).await });
        let call = peer.next_call().await;
        assert!(call.get("sessionId").is_none());
        peer.say(json!({ "id": call["id"], "error": { "code": -32000, "message": "Cannot navigate" } })).await;
        assert_eq!(b.await.unwrap().unwrap_err(), "Cannot navigate");
    }

    #[tokio::test]
    async fn an_event_reaches_only_the_session_it_belongs_to_and_in_order() {
        let (cdp, mut peer) = pair();
        let mut one = cdp.subscribe("s1");
        let mut two = cdp.subscribe("s2");
        for i in 0..3 {
            peer.say(json!({ "method": "Page.screencastFrame", "sessionId": "s1", "params": { "n": i } })).await;
        }
        peer.say(json!({ "method": "Page.frameNavigated", "sessionId": "s2", "params": {} })).await;
        for i in 0..3 {
            assert_eq!(one.recv().await.unwrap().params["n"], i);
        }
        assert_eq!(two.recv().await.unwrap().method, "Page.frameNavigated");
        assert!(one.try_recv().is_err());
    }

    #[tokio::test]
    async fn when_the_browser_goes_a_waiting_call_fails_and_the_client_is_dead() {
        let (cdp, peer) = pair();
        let c = cdp.clone();
        let waiting = tokio::spawn(async move { c.call("Page.enable", json!({}), None).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(peer);
        assert_eq!(waiting.await.unwrap().unwrap_err(), "the browser stopped");
        assert!(cdp.is_dead());
        assert_eq!(cdp.call("Page.enable", json!({}), None).await.unwrap_err(), "the browser stopped");
    }
}
