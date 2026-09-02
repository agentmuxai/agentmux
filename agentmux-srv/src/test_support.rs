// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Crate-wide test-only helpers shared across `#[cfg(test)]` modules.
//!
//! `ISOLATED_AUTH_ENV_LOCK` guards every test that mutates the process-global
//! `AGENTMUX_ISOLATED_AUTH`/`AGENTMUX_INSTANCE_DIR` env vars. Before this
//! existed, `registry::paths`, `migrations::runner`, and
//! `migrations::m0011_shared_store_backfill` each declared their own
//! module-local `Mutex<()>` — serializing tests *within* a module but not
//! *across* them. Cargo's default test runner executes all of a crate's
//! tests in one process with many threads, so those three modules' tests
//! could still interleave: one test clears the flag while another (holding
//! only its own module's lock) is mid-assertion on it, producing
//! nondeterministic failures (reagent/codex on PR #2318). Every test that
//! touches these env vars must acquire THIS lock instead of a local one.

pub(crate) static ISOLATED_AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A `WpsClient` that records every event it's sent, for tests asserting a
/// `Broker::publish` call actually fired (and with what event name/scopes) —
/// e.g. the `agent:memory:changed:{agent_id}` events
/// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md added across
/// `native_memory_handlers.rs`, `native_memory_drift.rs`, and
/// `app_api::mod`. `wps.rs`'s own test module has an equivalent `TestClient`,
/// but it's private to that file — this is the shared, crate-visible
/// version for every OTHER module's tests, rather than three private copies.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecordingWpsClient {
    events: std::sync::Mutex<Vec<(String, crate::backend::wps::WaveEvent)>>,
}

#[cfg(test)]
impl RecordingWpsClient {
    pub(crate) fn received_events(&self) -> Vec<(String, crate::backend::wps::WaveEvent)> {
        self.events.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl crate::backend::wps::WpsClient for std::sync::Arc<RecordingWpsClient> {
    fn send_event(&self, route_id: &str, event: crate::backend::wps::WaveEvent) {
        self.events.lock().unwrap().push((route_id.to_string(), event));
    }
}

/// Build a `Broker` already subscribed (all scopes) to `event_type` under a
/// fixed `"test-route"` route id, with a fresh `RecordingWpsClient` wired in
/// as its client — the minimum setup every `agent:memory:changed:*` publish
/// test needs, so each call site's test isn't re-deriving the same six lines.
#[cfg(test)]
pub(crate) fn broker_recording(event_type: &str) -> (crate::backend::wps::Broker, std::sync::Arc<RecordingWpsClient>) {
    let broker = crate::backend::wps::Broker::new();
    let client = std::sync::Arc::new(RecordingWpsClient::default());
    broker.set_client(Box::new(std::sync::Arc::clone(&client)));
    broker.subscribe(
        "test-route",
        crate::backend::wps::SubscriptionRequest {
            event: event_type.to_string(),
            scopes: vec![],
            allscopes: true,
        },
    );
    (broker, client)
}
