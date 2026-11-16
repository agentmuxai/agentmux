// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The CLI process srv runs for each agent pane, by block: what a sender's
//! process id is walked up to ([`super::sender`]).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static CLI_PIDS: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(Default::default);

fn pids() -> std::sync::MutexGuard<'static, HashMap<String, u32>> {
    CLI_PIDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `pid` is now the CLI process of `block_id`.
pub fn set(block_id: &str, pid: u32) {
    if pid != 0 {
        pids().insert(block_id.to_string(), pid);
    }
}

/// The CLI process `pid` of `block_id` ended. A replacement that already took
/// the slot stays.
pub fn clear(block_id: &str, pid: u32) {
    let mut g = pids();
    if g.get(block_id) == Some(&pid) {
        g.remove(block_id);
    }
}

/// Every (block, CLI process) pair.
pub fn snapshot() -> Vec<(String, u32)> {
    pids().iter().map(|(b, p)| (b.clone(), *p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replacement_process_is_not_cleared_by_the_old_ones_exit() {
        let b = format!("pids-{}", uuid::Uuid::new_v4());
        set(&b, 111);
        set(&b, 222);
        clear(&b, 111);
        assert!(snapshot().contains(&(b.clone(), 222)));
        clear(&b, 222);
        assert!(!snapshot().iter().any(|(k, _)| *k == b));
    }
}
