// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Wave Pub/Sub system: event brokering with scoped subscriptions.
//! Port of Go's pkg/wps/wps.go + wpstypes.go.

#![allow(dead_code)]
//!
//! The Broker supports:
//! - All-scope subscriptions (receive all events of a type)
//! - Exact-scope subscriptions (e.g., "block:uuid")
//! - Star-scope subscriptions (e.g., "block:*")
//! - Event persistence (history/replay)

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---- Event type constants (match Go) ----

pub const EVENT_BLOCK_CLOSE: &str = "blockclose";
pub const EVENT_CONN_CHANGE: &str = "connchange";
pub const EVENT_SYS_INFO: &str = "sysinfo";
pub const EVENT_CONTROLLER_STATUS: &str = "controllerstatus";
pub const EVENT_WAVE_OBJ_UPDATE: &str = "waveobj:update";
pub const EVENT_BLOCK_FILE: &str = "blockfile";
pub const EVENT_CONFIG: &str = "config";
pub const EVENT_USER_INPUT: &str = "userinput";
pub const EVENT_ROUTE_GONE: &str = "route:gone";
pub const EVENT_WORKSPACE_UPDATE: &str = "workspace:update";
pub const EVENT_WAVE_AI_RATE_LIMIT: &str = "waveai:ratelimit";
pub const EVENT_BLOCK_STATS: &str = "blockstats";

// File operation constants
pub const FILE_OP_CREATE: &str = "create";
pub const FILE_OP_DELETE: &str = "delete";
pub const FILE_OP_APPEND: &str = "append";
pub const FILE_OP_TRUNCATE: &str = "truncate";
pub const FILE_OP_INVALIDATE: &str = "invalidate";

const MAX_PERSIST: usize = 4096;
const REMAKE_ARR_THRESHOLD: usize = 10 * 1024;

// ---- Types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveEvent {
    pub event: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sender: String,
    #[serde(skip_serializing_if = "is_zero", default)]
    pub persist: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

impl WaveEvent {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionRequest {
    pub event: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub allscopes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WSFileEventData {
    pub zoneid: String,
    pub filename: String,
    pub fileop: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub data64: String,
}

// ---- Client trait ----

/// Trait for event delivery to connected clients.
pub trait WpsClient: Send + Sync {
    fn send_event(&self, route_id: &str, event: WaveEvent);
}

// ---- Subscription internals ----

#[derive(Default)]
struct BrokerSubscription {
    /// Route IDs subscribed to all scopes for this event.
    all_subs: Vec<String>,
    /// Exact scope → route IDs.
    scope_subs: HashMap<String, Vec<String>>,
    /// Star/wildcard scope → route IDs.
    star_subs: HashMap<String, Vec<String>>,
}

impl BrokerSubscription {
    fn is_empty(&self) -> bool {
        self.all_subs.is_empty() && self.scope_subs.is_empty() && self.star_subs.is_empty()
    }
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct PersistKey {
    event: String,
    scope: String,
}

struct PersistEventWrap {
    arr_total_adds: usize,
    events: Vec<WaveEvent>,
}

// ---- Broker ----

/// The central pub/sub broker for WaveEvents.
pub struct Broker {
    inner: Mutex<BrokerInner>,
}

struct BrokerInner {
    client: Option<Box<dyn WpsClient>>,
    sub_map: HashMap<String, BrokerSubscription>,
    persist_map: HashMap<PersistKey, PersistEventWrap>,
}

impl Broker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BrokerInner {
                client: None,
                sub_map: HashMap::new(),
                persist_map: HashMap::new(),
            }),
        }
    }

    pub fn set_client(&self, client: Box<dyn WpsClient>) {
        let mut inner = self.inner.lock().unwrap();
        inner.client = Some(client);
    }

    /// Subscribe a route to an event, optionally scoped.
    pub fn subscribe(&self, route_id: &str, sub: SubscriptionRequest) {
        if sub.event.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        // Remove existing subscription first (re-subscribe)
        Self::unsubscribe_nolock(&mut inner, route_id, &sub.event);

        let bs = inner
            .sub_map
            .entry(sub.event.clone())
            .or_default();

        if sub.allscopes {
            add_unique(&mut bs.all_subs, route_id);
            return;
        }
        for scope in &sub.scopes {
            if scope_has_star(scope) {
                add_to_scope_map(&mut bs.star_subs, scope, route_id);
            } else {
                add_to_scope_map(&mut bs.scope_subs, scope, route_id);
            }
        }
    }

    /// Unsubscribe a route from a specific event.
    pub fn unsubscribe(&self, route_id: &str, event_name: &str) {
        let mut inner = self.inner.lock().unwrap();
        Self::unsubscribe_nolock(&mut inner, route_id, event_name);
    }

    /// Unsubscribe a route from all events.
    pub fn unsubscribe_all(&self, route_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        let events: Vec<String> = inner.sub_map.keys().cloned().collect();
        for event in events {
            Self::unsubscribe_nolock(&mut inner, route_id, &event);
        }
    }

    fn unsubscribe_nolock(inner: &mut BrokerInner, route_id: &str, event_name: &str) {
        let bs = match inner.sub_map.get_mut(event_name) {
            Some(bs) => bs,
            None => return,
        };
        bs.all_subs.retain(|s| s != route_id);
        remove_from_all_scopes(&mut bs.scope_subs, route_id);
        remove_from_all_scopes(&mut bs.star_subs, route_id);
        if bs.is_empty() {
            inner.sub_map.remove(event_name);
        }
    }

    /// Publish an event to all matching subscribers.
    pub fn publish(&self, event: WaveEvent) {
        let mut inner = self.inner.lock().unwrap();

        // Persist if requested
        if event.persist > 0 {
            Self::persist_event(&mut inner, &event);
        }

        let client = match &inner.client {
            Some(c) => c,
            None => return,
        };

        let route_ids = Self::get_matching_routes(&inner, &event);
        for route_id in route_ids {
            client.send_event(&route_id, event.clone());
        }
    }

    /// Read persisted event history.
    pub fn read_event_history(
        &self,
        event_type: &str,
        scope: &str,
        max_items: usize,
    ) -> Vec<WaveEvent> {
        if max_items == 0 {
            return Vec::new();
        }
        let inner = self.inner.lock().unwrap();
        let key = PersistKey {
            event: event_type.to_string(),
            scope: scope.to_string(),
        };
        match inner.persist_map.get(&key) {
            Some(pe) if !pe.events.is_empty() => {
                let n = max_items.min(pe.events.len());
                pe.events[pe.events.len() - n..].to_vec()
            }
            _ => Vec::new(),
        }
    }

    fn persist_event(inner: &mut BrokerInner, event: &WaveEvent) {
        let num_persist = event.persist.min(MAX_PERSIST);
        let mut scope_set: Vec<String> = event.scopes.clone();
        scope_set.push(String::new()); // "" scope for global persistence

        for scope in scope_set {
            let key = PersistKey {
                event: event.event.clone(),
                scope,
            };
            let pe = inner.persist_map.entry(key).or_insert_with(|| {
                PersistEventWrap {
                    arr_total_adds: 0,
                    events: Vec::with_capacity(num_persist),
                }
            });
            pe.events.push(event.clone());
            pe.arr_total_adds += 1;
            // Trim to max persist
            if pe.events.len() > num_persist {
                pe.events.drain(..pe.events.len() - num_persist);
            }
            // Compact if too many additions (reduce memory fragmentation)
            if pe.arr_total_adds > REMAKE_ARR_THRESHOLD {
                let compacted: Vec<WaveEvent> = pe.events.drain(..).collect();
                pe.events = compacted;
                pe.arr_total_adds = pe.events.len();
            }
        }
    }

    fn get_matching_routes(inner: &BrokerInner, event: &WaveEvent) -> Vec<String> {
        let bs = match inner.sub_map.get(&event.event) {
            Some(bs) => bs,
            None => return Vec::new(),
        };

        let mut route_ids: HashMap<&str, ()> = HashMap::new();

        // All-scope subscribers
        for route_id in &bs.all_subs {
            route_ids.insert(route_id, ());
        }

        // Exact-scope subscribers
        for scope in &event.scopes {
            if let Some(routes) = bs.scope_subs.get(scope) {
                for route_id in routes {
                    route_ids.insert(route_id, ());
                }
            }
            // Star-scope subscribers
            for (star_scope, routes) in &bs.star_subs {
                if star_match(star_scope, scope, ":") {
                    for route_id in routes {
                        route_ids.insert(route_id, ());
                    }
                }
            }
        }

        route_ids.keys().map(|s| s.to_string()).collect()
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Helpers ----

fn scope_has_star(scope: &str) -> bool {
    scope.split(':').any(|part| part == "*" || part == "**")
}

/// Simple star matching: each segment separated by `sep` is compared.
/// "*" matches any single segment, "**" matches any remaining segments.
fn star_match(pattern: &str, value: &str, sep: &str) -> bool {
    let pat_parts: Vec<&str> = pattern.split(sep).collect();
    let val_parts: Vec<&str> = value.split(sep).collect();

    let mut pi = 0;
    let mut vi = 0;
    while pi < pat_parts.len() && vi < val_parts.len() {
        if pat_parts[pi] == "**" {
            return true; // matches everything remaining
        }
        if pat_parts[pi] != "*" && pat_parts[pi] != val_parts[vi] {
            return false;
        }
        pi += 1;
        vi += 1;
    }
    pi == pat_parts.len() && vi == val_parts.len()
}

fn add_unique(vec: &mut Vec<String>, val: &str) {
    if !vec.iter().any(|s| s == val) {
        vec.push(val.to_string());
    }
}

fn add_to_scope_map(map: &mut HashMap<String, Vec<String>>, scope: &str, route_id: &str) {
    let entry = map.entry(scope.to_string()).or_default();
    add_unique(entry, route_id);
}

fn remove_from_all_scopes(map: &mut HashMap<String, Vec<String>>, route_id: &str) {
    let empty_scopes: Vec<String> = map
        .iter_mut()
        .filter_map(|(scope, routes)| {
            routes.retain(|r| r != route_id);
            if routes.is_empty() {
                Some(scope.clone())
            } else {
                None
            }
        })
        .collect();
    for scope in empty_scopes {
        map.remove(&scope);
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct TestClient {
        events: Mutex<Vec<(String, WaveEvent)>>,
    }

    impl TestClient {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn received_events(&self) -> Vec<(String, WaveEvent)> {
            self.events.lock().unwrap().clone()
        }
    }

    impl WpsClient for TestClient {
        fn send_event(&self, route_id: &str, event: WaveEvent) {
            self.events
                .lock()
                .unwrap()
                .push((route_id.to_string(), event));
        }
    }

    impl WpsClient for Arc<TestClient> {
        fn send_event(&self, route_id: &str, event: WaveEvent) {
            self.events
                .lock()
                .unwrap()
                .push((route_id.to_string(), event));
        }
    }

    #[test]
    fn test_subscribe_all_scopes() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        let events = client.received_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "route-1");
    }

    #[test]
    fn test_subscribe_exact_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec!["block:abc".to_string()],
                allscopes: false,
            },
        );

        // Should match
        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        // Should NOT match
        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:xyz".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        let events = client.received_events();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_subscribe_star_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec!["block:*".to_string()],
                allscopes: false,
            },
        );

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["tab:xyz".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        let events = client.received_events();
        assert_eq!(events.len(), 1); // only block:* matches block:abc
    }

    #[test]
    fn test_unsubscribe() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_BLOCK_CLOSE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        broker.unsubscribe("route-1", EVENT_BLOCK_CLOSE);

        broker.publish(WaveEvent {
            event: EVENT_BLOCK_CLOSE.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        assert!(client.received_events().is_empty());
    }

    #[test]
    fn test_unsubscribe_all() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_BLOCK_CLOSE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );
        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_CONFIG.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        broker.unsubscribe_all("route-1");

        broker.publish(WaveEvent {
            event: EVENT_BLOCK_CLOSE.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });
        broker.publish(WaveEvent {
            event: EVENT_CONFIG.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        assert!(client.received_events().is_empty());
    }

    #[test]
    fn test_event_persistence() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        // Subscribe so events are dispatched
        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_SYS_INFO.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        // Publish persistent events
        for i in 0..5 {
            broker.publish(WaveEvent {
                event: EVENT_SYS_INFO.to_string(),
                scopes: vec!["cpu".to_string()],
                sender: String::new(),
                persist: 3, // keep last 3
                data: Some(serde_json::json!(i)),
            });
        }

        // Read history (global scope "")
        let history = broker.read_event_history(EVENT_SYS_INFO, "", 10);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].data, Some(serde_json::json!(2)));
        assert_eq!(history[2].data, Some(serde_json::json!(4)));

        // Read scoped history
        let scoped = broker.read_event_history(EVENT_SYS_INFO, "cpu", 2);
        assert_eq!(scoped.len(), 2);
    }

    #[test]
    fn test_star_match() {
        assert!(star_match("block:*", "block:abc", ":"));
        assert!(star_match("*:abc", "block:abc", ":"));
        assert!(!star_match("block:*", "tab:abc", ":"));
        assert!(star_match("**", "block:abc:xyz", ":"));
        assert!(!star_match("block:*", "block:abc:xyz", ":")); // * matches one segment only
    }

    #[test]
    fn test_wave_event_serialization() {
        let event = WaveEvent {
            event: "test".to_string(),
            scopes: vec!["scope1".to_string()],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: WaveEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event, "test");
        assert_eq!(parsed.scopes, vec!["scope1"]);
        // Empty sender and zero persist should be omitted
        assert!(!json.contains("\"sender\""));
        assert!(!json.contains("\"persist\""));
    }

    #[test]
    fn test_subscription_request_serialization() {
        let req = SubscriptionRequest {
            event: "blockclose".to_string(),
            scopes: vec!["block:123".to_string()],
            allscopes: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SubscriptionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event, "blockclose");
    }

    #[test]
    fn test_no_client_publish_does_not_panic() {
        let broker = Broker::new();
        // No client set — should not panic
        broker.publish(WaveEvent {
            event: "test".to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });
    }

    #[test]
    fn test_double_star_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec!["**".to_string()],
                allscopes: false,
            },
        );

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc:def".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        assert_eq!(client.received_events().len(), 1);
    }
}
