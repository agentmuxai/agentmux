// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! When the window pool and the pane pool start. They used to be spawned the
//! moment the "main" browser was created, before main's page had even loaded,
//! so every launch ran three renderers (main, a window-pool and a pane-pool
//! window) through the same frontend bundle at once, and a fourth soon after.
//! They only matter for tear-offs and new windows, so they now wait until main
//! has loaded.

use std::sync::Arc;

use cef::{rc::Rc, ImplTask, Task, WrapTask};

use crate::state::AppState;

/// Set when the "main" browser is created; cleared by whichever comes first
/// of the main window's `wave-ready` and the fallback timer, which then start
/// the window and pane pools.
static STARTUP_POOLS_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// How long after the "main" browser is created the pools start anyway if the
/// main window never reports `wave-ready` (its frontend failed to initialize).
const STARTUP_POOLS_FALLBACK_MS: i64 = 15_000;

/// Called from `on_after_created` for the "main" label: arm the pools. They
/// start when main reports `wave-ready` ([`on_main_window_ready`]), or after
/// `STARTUP_POOLS_FALLBACK_MS` if it never does. A tear-off or new window
/// before then takes the cold path, as it does whenever the pool is empty.
pub fn defer_startup_pools(state: &Arc<AppState>) {
    STARTUP_POOLS_PENDING.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut task = StartPendingPoolsTask::new(Arc::clone(state), "fallback timer");
    cef::post_delayed_task(cef::ThreadId::UI, Some(&mut task), STARTUP_POOLS_FALLBACK_MS);
}

/// The main window finished loading (`wave-ready`): start the pools if they
/// are still pending. Safe from any thread; the spawn runs on the UI thread.
pub fn on_main_window_ready(state: &Arc<AppState>) {
    if STARTUP_POOLS_PENDING.load(std::sync::atomic::Ordering::SeqCst) {
        let mut task = StartPendingPoolsTask::new(Arc::clone(state), "main window ready");
        cef::post_task(cef::ThreadId::UI, Some(&mut task));
    }
}

fn start_pending_pools(state: &Arc<AppState>, trigger: &str) {
    if !STARTUP_POOLS_PENDING.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tracing::info!(target: "dnd:tearoff:pool", trigger, "[pool] starting the window and pane pools");
    super::window_pool::init_pool(state);
    super::window_pool::init_pane_pool(state);
}

cef::wrap_task! {
    struct StartPendingPoolsTask {
        state: Arc<AppState>,
        trigger: &'static str,
    }

    impl Task {
        fn execute(&self) {
            start_pending_pools(&self.state, self.trigger);
        }
    }
}
