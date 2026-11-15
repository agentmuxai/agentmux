// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The event a Drone run's Wait block (`on`) waits for
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "Waiting").
//!
//! The wait is a random key made when it starts, not derived from anything an
//! outsider can see. Whoever holds the key can end the wait, once, and what they
//! send with it is outside text: the run reads it as tainted. The key goes out in
//! the run's `Waiting` event and is kept in the run's checkpoint, so a restart
//! takes the same wait up again; an event that arrives while AgentMux is stopped
//! is not kept, its source sends it again. It lives in the core so the RPC handler
//! can answer without reaching up into `drone/`; the executor is the one that waits.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use tokio::sync::oneshot;

/// The waits that are open, by key.
static WAITING: LazyLock<Mutex<HashMap<String, oneshot::Sender<Value>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// A new key: 256 random bits as hex, so it cannot be guessed.
pub fn new_key() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// A wait on `key` is open: the receiver gets what the event brings.
pub fn open(key: &str) -> oneshot::Receiver<Value> {
    let (tx, rx) = oneshot::channel();
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), tx);
    rx
}

/// The wait on `key` is over (it was answered, or its run ended).
pub fn close(key: &str) {
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
}

/// End the wait on `key` with `payload`. `Err` when none is open: the key is
/// wrong, the wait was answered already, or its run ended.
pub fn signal(key: &str, payload: Value) -> Result<(), String> {
    let open = WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
    let Some(tx) = open else { return Err("no run is waiting on that key".to_string()) };
    tx.send(payload).map_err(|_| "the run stopped waiting".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_long_and_never_repeat() {
        let (a, b) = (new_key(), new_key());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn a_wait_is_answered_once_with_what_the_event_brought() {
        let key = new_key();
        let rx = open(&key);
        signal(&key, json!({ "ok": true })).unwrap();
        assert_eq!(rx.await.unwrap(), json!({ "ok": true }));
        assert!(signal(&key, json!(1)).is_err(), "a second answer finds nothing waiting");
    }

    #[test]
    fn a_wrong_key_or_a_closed_wait_is_refused() {
        assert!(signal("not-a-key", json!(1)).is_err());
        let key = new_key();
        let _rx = open(&key);
        close(&key);
        assert!(signal(&key, json!(1)).is_err());
    }
}
