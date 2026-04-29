// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.9.1 — position-event debounce.
//
// `EVENT_OBJECT_LOCATIONCHANGE` fires on every WM_WINDOWPOSCHANGED
// dispatch, including during a drag (60+ events/sec). The reducer
// only needs the FINAL rect after the burst settles — the
// `OffMonitor` classification is the same at every intermediate
// position as at the final position, but emitting 60 redundant
// drift events spams the launcher log.
//
// Strategy: per-HWND last-emit timestamp. When a new event fires,
// drop it if the previous emit was less than `DEBOUNCE_MS` ago.
// We accept up to `DEBOUNCE_MS` of staleness in the launcher's
// last-known rect — fine for B.9.1's observation purpose.
//
// This is NOT a heartbeat — there is no thread waking periodically.
// We only ever emit in response to OS events, just sometimes drop
// them.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Maximum frequency at which any single HWND will produce a
/// position report. Currently 50ms ≈ 20 Hz.
const DEBOUNCE_MS: u128 = 50;

/// Per-HWND last-emit time. Lock is held only for HashMap
/// operations — never across IPC. Mutex-poisoning is treated as
/// unrecoverable (poison = a panic in the WRR hook callback,
/// which means the hook is broken anyway); we recover via
/// `into_inner` so the next caller starts fresh.
fn map() -> &'static Mutex<HashMap<u64, Instant>> {
    static M: std::sync::OnceLock<Mutex<HashMap<u64, Instant>>> = std::sync::OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Phase B.9.1 — should we emit a position report for this HWND
/// right now? Returns `true` if at least `DEBOUNCE_MS` has
/// elapsed since the last emit for this HWND (or this is the
/// first emit for it). Updates the timestamp atomically with the
/// decision.
pub fn should_emit(hwnd: u64) -> bool {
    let now = Instant::now();
    let mut m = match map().lock() {
        Ok(g) => g,
        Err(poisoned) => {
            // Recover: clear the poisoned state and continue.
            let mut g = poisoned.into_inner();
            g.clear();
            g
        }
    };
    let last = m.get(&hwnd).copied();
    let allow = match last {
        None => true,
        Some(t) => now.duration_since(t).as_millis() >= DEBOUNCE_MS,
    };
    if allow {
        m.insert(hwnd, now);
    }
    allow
}

/// Phase B.9.1 — drop the debounce entry for an HWND. Called from
/// the destroy hook so a recycled HWND value doesn't inherit the
/// previous occupant's debounce state.
pub fn forget(hwnd: u64) {
    let mut m = match map().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    m.remove(&hwnd);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn first_emit_for_an_hwnd_is_allowed() {
        // Use a unique value to avoid interference with other tests.
        let h = 0xABCD_0001;
        forget(h);
        assert!(should_emit(h));
    }

    #[test]
    fn rapid_second_emit_is_dropped() {
        let h = 0xABCD_0002;
        forget(h);
        assert!(should_emit(h));
        assert!(!should_emit(h));
    }

    #[test]
    fn after_debounce_window_emits_again() {
        let h = 0xABCD_0003;
        forget(h);
        assert!(should_emit(h));
        sleep(Duration::from_millis(60));
        assert!(should_emit(h));
    }

    #[test]
    fn forget_resets_state() {
        let h = 0xABCD_0004;
        forget(h);
        assert!(should_emit(h));
        forget(h);
        // Forget cleared the entry, so the next emit is the "first" again.
        assert!(should_emit(h));
    }
}
