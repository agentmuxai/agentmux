// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Event listener system for AgentMux RPC.
//! Port of Go's `pkg/wshutil/wshevent.go`.
//!
//! Provides a pub/sub event system that converts WaveEvents into
//! a listener-based API with register/unregister support.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Callback type for event listeners.
pub type EventCallback = Box<dyn Fn(&WaveEvent) + Send + Sync>;

/// A wave event with a name and JSON payload.
#[derive(Debug, Clone)]
pub struct WaveEvent {
    pub event: String,
    pub scopes: Vec<String>,
    pub data: Option<serde_json::Value>,
}

struct SingleListener {
    id: String,
    callback: EventCallback,
}

/// Thread-safe event listener with pub/sub support.
pub struct EventListener {
    listeners: Mutex<HashMap<String, Vec<SingleListener>>>,
}

impl EventListener {
    pub fn new() -> Self {
        Self {
            listeners: Mutex::new(HashMap::new()),
        }
    }

    /// Register a listener for an event. Returns a listener ID for unregistration.
    pub fn on(&self, event_name: &str, callback: EventCallback) -> String {
        let id = Uuid::new_v4().to_string();
        let mut listeners = self.listeners.lock().unwrap();
        let entry = listeners.entry(event_name.to_string()).or_default();
        entry.push(SingleListener {
            id: id.clone(),
            callback,
        });
        id
    }

    /// Unregister a listener by event name and listener ID.
    pub fn unregister(&self, event_name: &str, id: &str) {
        let mut listeners = self.listeners.lock().unwrap();
        if let Some(entry) = listeners.get_mut(event_name) {
            entry.retain(|sl| sl.id != id);
        }
    }

    /// Dispatch an event to all registered listeners.
    pub fn recv_event(&self, event: &WaveEvent) {
        let listeners = self.listeners.lock().unwrap();
        if let Some(entry) = listeners.get(&event.event) {
            for sl in entry {
                (sl.callback)(event);
            }
        }
    }

    /// Get the count of listeners for a specific event.
    pub fn listener_count(&self, event_name: &str) -> usize {
        let listeners = self.listeners.lock().unwrap();
        listeners.get(event_name).map_or(0, |v| v.len())
    }
}

impl Default for EventListener {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_event_listener_on_and_recv() {
        let el = EventListener::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        el.on("test_event", Box::new(move |_| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        }));

        let event = WaveEvent {
            event: "test_event".to_string(),
            scopes: vec![],
            data: None,
        };
        el.recv_event(&event);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        el.recv_event(&event);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_event_listener_unregister() {
        let el = EventListener::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let id = el.on("test_event", Box::new(move |_| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        }));

        assert_eq!(el.listener_count("test_event"), 1);
        el.unregister("test_event", &id);
        assert_eq!(el.listener_count("test_event"), 0);

        let event = WaveEvent {
            event: "test_event".to_string(),
            scopes: vec![],
            data: None,
        };
        el.recv_event(&event);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_event_listener_no_listeners() {
        let el = EventListener::new();
        let event = WaveEvent {
            event: "unknown".to_string(),
            scopes: vec![],
            data: None,
        };
        // Should not panic
        el.recv_event(&event);
    }
}
