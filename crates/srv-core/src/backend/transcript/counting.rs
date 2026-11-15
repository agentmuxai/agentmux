// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Counting a transcript's lines once, off the request path.
//!
//! A transcript with no line epoch (`filestore/counter.rs`) cannot open: `agent:open_tail` has no line
//! positions to give, and `output.idx` cannot be labelled with a generation, so the index is rebuilt
//! from byte 0 on every read and, while the file grows under it, never serves one. On a gigabyte
//! transcript each rebuild is seconds, and the read is then refused. An epoch is lost when bytes inside
//! the file are rewritten (a fork's copy is signed in place) or an older build appends past the
//! catch-up window.
//!
//! Starting the epoch is one scan of the file (`FileStore::init_line_counter`), which takes the
//! connection lock one window at a time. This module makes sure it runs once per file at a time:
//! readers that find no epoch start the count if none is running, then wait for it a little (or answer
//! "counting" and let the pane ask again), instead of each starting their own scan or rebuilding the
//! index.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::backend::storage::filestore::FileStore;

/// A count in progress: finished when `done` is set.
struct Flight {
    done: Mutex<bool>,
    ended: Condvar,
}

impl Flight {
    fn wait(&self, wait: Duration) -> bool {
        let guard = self.done.lock().unwrap();
        let (guard, _) = self.ended.wait_timeout_while(guard, wait, |done| !*done).unwrap();
        *guard
    }
}

static FLIGHTS: Mutex<Option<HashMap<String, Arc<Flight>>>> = Mutex::new(None);

/// The key of `zone`'s count in `fs`: the store by identity, so two stores' zones of one name stay apart.
pub fn flight_key(fs: &Arc<FileStore>, zone: &str) -> String {
    format!("{:p}:{zone}", Arc::as_ptr(fs))
}

/// Run `work` on a thread of its own unless a flight for `key` is already running, and wait up to
/// `wait` for it. True when the flight ended in that time.
pub fn run_once(key: String, wait: Duration, work: impl FnOnce() + Send + 'static) -> bool {
    let (flight, starts) = {
        let mut flights = FLIGHTS.lock().unwrap();
        let map = flights.get_or_insert_with(HashMap::new);
        match map.get(&key) {
            Some(f) => (f.clone(), false),
            None => {
                let f = Arc::new(Flight { done: Mutex::new(false), ended: Condvar::new() });
                map.insert(key.clone(), f.clone());
                (f, true)
            }
        }
    };
    if starts {
        let ending = flight.clone();
        let spawned = std::thread::Builder::new().name("transcript-count".into()).spawn(move || {
            work();
            // Leave the map first: a reader that finds the flight over starts a new one if it still needs it.
            if let Some(map) = FLIGHTS.lock().unwrap().as_mut() {
                map.remove(&key);
            }
            *ending.done.lock().unwrap() = true;
            ending.ended.notify_all();
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "transcript count: couldn't start its thread");
            return false;
        }
    }
    flight.wait(wait)
}

/// Whether `zone`'s `output` has a line epoch now (a valid one, or one only behind, which is caught up).
fn counted(fs: &FileStore, zone: &str) -> bool {
    matches!(fs.catch_up_line_counter(zone, "output"), Ok(Some(state)) if state.counted.is_some())
}

/// `zone`'s `output` counted: true when it has an epoch within `wait`. Cheap when it already has one
/// (or only needs catching up); otherwise the count starts if none is running and is waited for. False
/// means it is still being counted, or could not be (the file moved under the scan): ask again later.
pub fn ensure_counted(fs: &Arc<FileStore>, zone: &str, wait: Duration) -> bool {
    if counted(fs, zone) {
        return true;
    }
    let (store, name) = (fs.clone(), zone.to_string());
    let began = std::time::Instant::now();
    run_once(flight_key(fs, zone), wait, move || match store.init_line_counter(&name, "output") {
        Ok(Some(_)) => tracing::info!(zone = %name, ms = began.elapsed().as_millis() as u64, "transcript count: counted"),
        Ok(None) => tracing::info!(zone = %name, "transcript count: the file moved under the scan; the next read counts it again"),
        Err(e) => tracing::warn!(zone = %name, error = %e, "transcript count: failed"),
    });
    counted(fs, zone)
}

#[cfg(test)]
#[path = "counting_tests.rs"]
mod tests;
