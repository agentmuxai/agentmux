// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The size each block's PTY was last given: at spawn (its runtime options)
//! and on every resize. Kept so a reader that doesn't drive the terminal (a
//! paired device watching it) can render at the desktop's width; the PTY
//! itself keeps no record srv can ask.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static SIZES: LazyLock<Mutex<HashMap<String, (u16, u16)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record `block_id`'s PTY size as `(rows, cols)`.
pub fn record(block_id: &str, rows: u16, cols: u16) {
    SIZES.lock().unwrap_or_else(|p| p.into_inner()).insert(block_id.to_string(), (rows, cols));
}

/// `block_id`'s PTY size as `(rows, cols)`, if it has had one since srv started.
pub fn get(block_id: &str) -> Option<(u16, u16)> {
    SIZES.lock().unwrap_or_else(|p| p.into_inner()).get(block_id).copied()
}

/// Forget `block_id` (its controller is gone).
pub fn forget(block_id: &str) {
    SIZES.lock().unwrap_or_else(|p| p.into_inner()).remove(block_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_size_wins_until_forgotten() {
        let block = uuid::Uuid::new_v4().to_string();
        assert_eq!(get(&block), None);
        record(&block, 25, 200);
        record(&block, 40, 120);
        assert_eq!(get(&block), Some((40, 120)));
        forget(&block);
        assert_eq!(get(&block), None);
    }
}
