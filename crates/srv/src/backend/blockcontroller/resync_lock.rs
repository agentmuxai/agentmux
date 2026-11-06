// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One resync of a block at a time. `resync_controller` looks the block's controller up, decides, and
//! then creates and starts one; two of them for the same block at once both saw "no controller, or one
//! still in `init`" and both started it. Seen on a fresh Muse install: a forced resync and an ordinary
//! one 74 ms apart spawned `muse serve` twice, 28 ms apart, and the first process was thrown away.

use std::collections::HashSet;
use std::sync::{Condvar, Mutex};

static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static FREED: Condvar = Condvar::new();

/// Held for the length of one resync of a block; the next resync of that block waits for it.
pub(super) struct ResyncGuard(String);

impl ResyncGuard {
    pub(super) fn acquire(block_id: &str) -> Self {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        while busy.get_or_insert_with(HashSet::new).contains(block_id) {
            busy = FREED.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        busy.get_or_insert_with(HashSet::new).insert(block_id.to_string());
        Self(block_id.to_string())
    }
}

impl Drop for ResyncGuard {
    fn drop(&mut self) {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(set) = busy.as_mut() {
            set.remove(&self.0);
        }
        FREED.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn two_resyncs_of_one_block_never_overlap_and_two_blocks_do() {
        let (inside, worst) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (inside, worst) = (inside.clone(), worst.clone());
                std::thread::spawn(move || {
                    let _g = ResyncGuard::acquire("resync-lock-same");
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    worst.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert_eq!(worst.load(Ordering::SeqCst), 1, "one block, one resync at a time");

        // A different block is not held up by the first one.
        let held = ResyncGuard::acquire("resync-lock-a");
        let other = std::thread::spawn(|| drop(ResyncGuard::acquire("resync-lock-b")));
        other.join().unwrap();
        drop(held);
        drop(ResyncGuard::acquire("resync-lock-a")); // free again once released
    }
}
