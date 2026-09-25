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
/// `app_api::mod`. `mps.rs`'s own test module has an equivalent `TestClient`,
/// but it's private to that file — this is the shared, crate-visible
/// version for every OTHER module's tests, rather than three private copies.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecordingWpsClient {
    events: std::sync::Mutex<Vec<(String, crate::backend::mps::MuxEvent)>>,
}

#[cfg(test)]
impl RecordingWpsClient {
    pub(crate) fn received_events(&self) -> Vec<(String, crate::backend::mps::MuxEvent)> {
        self.events.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl crate::backend::mps::WpsClient for std::sync::Arc<RecordingWpsClient> {
    fn send_event(&self, route_id: &str, event: crate::backend::mps::MuxEvent) {
        self.events.lock().unwrap().push((route_id.to_string(), event));
    }
}

/// Build a `Broker` already subscribed (all scopes) to `event_type` under a
/// fixed `"test-route"` route id, with a fresh `RecordingWpsClient` wired in
/// as its client — the minimum setup every `agent:memory:changed:*` publish
/// test needs, so each call site's test isn't re-deriving the same six lines.
#[cfg(test)]
pub(crate) fn broker_recording(event_type: &str) -> (crate::backend::mps::Broker, std::sync::Arc<RecordingWpsClient>) {
    let broker = crate::backend::mps::Broker::new();
    let client = std::sync::Arc::new(RecordingWpsClient::default());
    broker.set_client(Box::new(std::sync::Arc::clone(&client)));
    broker.subscribe(
        "test-route",
        crate::backend::mps::SubscriptionRequest {
            event: event_type.to_string(),
            scopes: vec![],
            allscopes: true,
        },
    );
    (broker, client)
}

/// The fake App Server test binary (`tests/fixtures/fake_app_server.rs`),
/// compiled once per source content and reused across test runs. It used to
/// be built into a fresh temp dir that was deliberately leaked every run —
/// 4.4 MB each, gigabytes of `/tmp` on a machine where many agents run the
/// srv tests.
#[cfg(test)]
pub(crate) fn fake_app_server_binary() -> &'static std::path::PathBuf {
    static BINARY: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    BINARY.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("fake_app_server.rs");
        let code = std::fs::read(&source).expect("read fake App Server source");
        let hash = hex::encode(Sha256::digest(&code));
        let dir = std::env::temp_dir().join(format!("agentmux-test-fake-app-server-{}", &hash[..16]));
        let mut binary = dir.join("fake-app-server");
        if cfg!(windows) {
            binary.set_extension("exe");
        }
        if binary.is_file() {
            return binary;
        }
        std::fs::create_dir_all(&dir).expect("create fake server build dir");
        // Build to a unique name and rename into place, so concurrent test
        // processes never run a half-written binary.
        let partial = dir.join(format!(".build-{}", uuid::Uuid::new_v4().simple()));
        let output = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg(&source)
            .arg("-o")
            .arg(&partial)
            .output()
            .expect("run rustc for fake App Server");
        assert!(
            output.status.success(),
            "fake App Server compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // A failed rename is fine only if another process put its build in
        // place first (Windows won't rename over an existing file).
        if let Err(e) = std::fs::rename(&partial, &binary) {
            let _ = std::fs::remove_file(&partial);
            assert!(binary.is_file(), "install fake App Server binary at {}: {e}", binary.display());
        }
        binary
    })
}
