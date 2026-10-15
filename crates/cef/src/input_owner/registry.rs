// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every window's input owner, and the one way to step one: under a lock, with
//! the time, logging each change as one `[input]` line. The platform adapter
//! runs the effects it returns. docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §2.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use parking_lot::Mutex;

use super::machine::{Effect, Event, WindowInput};

/// Each window's machine, by window label (`main`, `floating-<uuid>`, …).
static OWNERS: LazyLock<Mutex<HashMap<String, WindowInput>>> = LazyLock::new(Default::default);

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Milliseconds on a monotonic clock, for the machine's timing.
pub fn now_ms() -> u64 {
    EPOCH.elapsed().as_millis() as u64
}

/// Step `window`'s machine with `event` and return the effects to run. Logs a
/// line when the state changes or there is something to do.
pub fn step(window: &str, event: Event) -> Vec<Effect> {
    let now = now_ms();
    let (before, after, effects) = {
        let mut owners = OWNERS.lock();
        let input = owners.entry(window.to_string()).or_default();
        let before = input.describe();
        let effects = input.step(event.clone(), now);
        (before, input.describe(), effects)
    };
    if before != after || !effects.is_empty() {
        let fx: Vec<String> = effects.iter().map(ToString::to_string).collect();
        tracing::info!("[input] win={window} {event}: {before} -> {after} fx=[{}]", fx.join(", "));
    }
    effects
}

/// The window whose machine knows pane `block`.
pub fn window_of_pane(block: &str) -> Option<String> {
    OWNERS.lock().iter().find(|(_, w)| w.panes.contains_key(block)).map(|(k, _)| k.clone())
}
