// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPIKE (not for merge) — idle hung-renderer probe.
//! See docs/specs/SPEC_IDLE_HUNG_RENDERER_DETECTION_2026_09_23.md.
//!
//! Every PROBE_INTERVAL the host runs a one-line script in each app-UI
//! browser that logs a sentinel console message; `on_console_message` records
//! it. A browser silent for longer than SILENCE_BEFORE_POKE gets one F24 key
//! event, which a hung renderer cannot acknowledge — arming Chromium's hang
//! monitor so unresponsive.rs (#3594) can recover it.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

pub(crate) const SENTINEL: &str = "__agentmux_liveness__";
const PROBE_INTERVAL_MS: i64 = 10_000;
const SILENCE_BEFORE_POKE: Duration = Duration::from_secs(25);

/// Test hook: `AGENTMUX_SPIKE_SILENCE_SECS=0` pokes every healthy window about
/// every 10 s, to check that the poke is harmless.
fn silence_before_poke() -> Duration {
    std::env::var("AGENTMUX_SPIKE_SILENCE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(SILENCE_BEFORE_POKE)
}
const VK_F24: i32 = 0x87;

struct Entry {
    last_alive: Instant,
    poked_at: Option<Instant>,
}

fn table() -> &'static Mutex<HashMap<i32, Entry>> {
    static T: OnceLock<Mutex<HashMap<i32, Entry>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

wrap_task! {
    struct LivenessTick {
        state: Arc<AppState>,
    }
    impl Task { fn execute(&self) {
        tick(&self.state);
        let mut next = LivenessTick::new(self.state.clone());
        post_delayed_task(ThreadId::UI, Some(&mut next), PROBE_INTERVAL_MS);
    }}
}

/// Start the probe loop once per process.
pub(crate) fn start(state: &Arc<AppState>) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_ok() {
        tracing::info!(target: "crash", kind = "liveness_probe_started", "SPIKE liveness probe started");
        let mut first = LivenessTick::new(state.clone());
        post_delayed_task(ThreadId::UI, Some(&mut first), PROBE_INTERVAL_MS);
    }
}

fn tick(state: &Arc<AppState>) {
    let now = Instant::now();
    let browsers = state.list_top_level_browsers();
    let mut t = table().lock();
    for (label, browser) in browsers {
        let id = browser.identifier();
        let e = t.entry(id).or_insert(Entry { last_alive: now, poked_at: None });
        let silent = now.duration_since(e.last_alive);
        if silent > silence_before_poke() && e.poked_at.is_none() {
            e.poked_at = Some(now);
            tracing::warn!(
                target: "crash",
                kind = "renderer_probe_silent",
                browser_id = id,
                label = %label,
                silent_secs = silent.as_secs(),
                "app-UI renderer silent — poking it so Chromium's hang monitor measures it",
            );
            poke(&browser);
        }
        let js = format!("console.debug('{SENTINEL} {id}')");
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
        }
    }
}

fn poke(browser: &Browser) {
    let Some(host) = browser.host() else { return };
    let mode = std::env::var("AGENTMUX_SPIKE_POKE_MODE").unwrap_or_else(|_| "keyup".into());
    let mut ev = KeyEvent::default();
    ev.windows_key_code = VK_F24;
    if mode == "keydown" {
        ev.type_ = KeyEventType::RAWKEYDOWN;
        host.send_key_event(Some(&ev));
    }
    ev.type_ = KeyEventType::KEYUP;
    host.send_key_event(Some(&ev));
    tracing::info!(target: "crash", kind = "renderer_poked", mode = %mode, "SPIKE poke sent");
}

/// Called from `on_console_message`. Returns true if it was the sentinel.
pub(crate) fn on_console_message(browser: Option<&mut Browser>, message: &str) -> bool {
    if !message.starts_with(SENTINEL) {
        return false;
    }
    if let Some(b) = browser {
        let id = b.identifier();
        let mut t = table().lock();
        let e = t.entry(id).or_insert(Entry { last_alive: Instant::now(), poked_at: None });
        if let Some(p) = e.poked_at.take() {
            tracing::info!(
                target: "crash",
                kind = "renderer_probe_alive_again",
                browser_id = id,
                since_poke_ms = p.elapsed().as_millis() as u64,
                "app-UI renderer answering the probe again",
            );
        }
        e.last_alive = Instant::now();
    }
    true
}
