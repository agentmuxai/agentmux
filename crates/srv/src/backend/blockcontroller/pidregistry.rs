// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block PID registry: maps blockId → OS process ID for per-pane metrics collection.
//! Used by the sysinfo loop to query per-process CPU/memory stats.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

static BLOCK_PIDS: LazyLock<RwLock<HashMap<String, u32>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn register(block_id: &str, pid: u32) {
    BLOCK_PIDS.write().unwrap().insert(block_id.to_string(), pid);
    tracing::debug!(block_id = %block_id, pid = pid, "pidregistry: registered");
}

pub fn unregister(block_id: &str) {
    BLOCK_PIDS.write().unwrap().remove(block_id);
    tracing::debug!(block_id = %block_id, "pidregistry: unregistered");
}

pub fn get_all() -> Vec<(String, u32)> {
    BLOCK_PIDS
        .read()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}
