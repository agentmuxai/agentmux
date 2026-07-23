// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Pool state (H.4) ─────────────────────────────────────────────────────

/// Pre-warmed window pool state. Replaces three separate fields on
/// `AppState`: `window_pool: Mutex<VecDeque<String>>`,
/// `unpromoted_pool_labels: Mutex<HashSet<String>>`, and
/// `window_pool_respawn_in_flight: AtomicBool`. PR #3 migrates each
/// caller through the a→e ratchet.
#[derive(Default, Clone, Debug)]
#[allow(dead_code)]
pub struct PoolState {
    /// Labels of pool windows whose renderer signaled ready (eligible
    /// for promotion).
    pub queue: std::collections::VecDeque<String>,
    /// Labels spawned but not yet renderer-ready (and therefore not yet
    /// in `queue`). Used for taskbar/exclusion filters during the spawn
    /// → ready window.
    pub unpromoted: std::collections::HashSet<String>,
    /// Single-flight semaphore: true while a respawn task is in flight,
    /// preventing stacked refills.
    pub respawn_in_flight: bool,
}

/// Pre-warmed pane (floating) window pool state.
/// Mirrors `PoolState` but for `floating-pool-{uuid}` frameless windows.
#[derive(Default, Clone, Debug)]
#[allow(dead_code)]
pub struct PanePoolState {
    pub queue: std::collections::VecDeque<String>,
    pub unpromoted: std::collections::HashSet<String>,
    pub respawn_in_flight: bool,
}
