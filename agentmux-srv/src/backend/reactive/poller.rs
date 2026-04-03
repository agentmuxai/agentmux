// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use super::handler::ReactiveHandler;
use super::types::{PollerConfig, PollerStatus};
use super::now_unix_millis;

/// Poller state for cross-host message polling from AgentMux.
pub struct Poller {
    config: RwLock<PollerConfig>,
    _handler: &'static ReactiveHandler,
    running: Mutex<bool>,
    poll_count: Mutex<u64>,
    injections_count: Mutex<u64>,
    last_poll: Mutex<Option<u64>>,
    last_error: Mutex<Option<String>>,
}

impl Poller {
    /// Create a new poller with the given config.
    pub fn new(config: PollerConfig, handler: &'static ReactiveHandler) -> Self {
        Self {
            config: RwLock::new(config),
            _handler: handler,
            running: Mutex::new(false),
            poll_count: Mutex::new(0),
            injections_count: Mutex::new(0),
            last_poll: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    /// Check if the poller is configured (has URL and token).
    pub fn is_configured(&self) -> bool {
        let config = self.config.read().unwrap();
        config.agentmux_url.is_some() && config.agentmux_token.is_some()
    }

    /// Check if the poller is running.
    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }

    /// Get poller statistics.
    pub fn stats(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert(
            "poll_count".to_string(),
            serde_json::json!(*self.poll_count.lock().unwrap()),
        );
        m.insert(
            "injections_count".to_string(),
            serde_json::json!(*self.injections_count.lock().unwrap()),
        );
        m.insert(
            "last_poll".to_string(),
            serde_json::json!(*self.last_poll.lock().unwrap()),
        );
        m.insert(
            "last_error".to_string(),
            serde_json::json!(*self.last_error.lock().unwrap()),
        );
        m
    }

    /// Get poller status.
    pub fn status(&self) -> PollerStatus {
        let config = self.config.read().unwrap();
        PollerStatus {
            configured: config.agentmux_url.is_some() && config.agentmux_token.is_some(),
            running: self.is_running(),
            url: config.agentmux_url.clone(),
            has_token: config.agentmux_token.is_some(),
            poll_count: *self.poll_count.lock().unwrap(),
            injections_count: *self.injections_count.lock().unwrap(),
            last_poll: *self.last_poll.lock().unwrap(),
        }
    }

    /// Reconfigure the poller with new URL and token.
    pub fn reconfigure(&self, url: Option<String>, token: Option<String>) {
        let mut config = self.config.write().unwrap();
        config.agentmux_url = url;
        config.agentmux_token = token;
    }

    /// Record a successful poll.
    pub fn record_poll(&self) {
        *self.poll_count.lock().unwrap() += 1;
        *self.last_poll.lock().unwrap() = Some(now_unix_millis());
        *self.last_error.lock().unwrap() = None;
    }

    /// Record a poll error.
    pub fn record_error(&self, err: &str) {
        *self.last_error.lock().unwrap() = Some(err.to_string());
    }

    /// Record injections delivered.
    pub fn record_injections(&self, count: u64) {
        *self.injections_count.lock().unwrap() += count;
    }

    /// Set the running state.
    pub fn set_running(&self, running: bool) {
        *self.running.lock().unwrap() = running;
    }
}
