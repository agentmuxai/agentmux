// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The hybrid logical clock that orders a collection's records (SPEC_USER_DATA_SYNC_LAYER §3.2): wall
//! milliseconds, a counter for changes in the same millisecond, and the machine that made the change, so any
//! two versions are totally ordered. A version far ahead of the receiver's own clock is refused, so a machine
//! with a wrong clock cannot win every merge.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// How far ahead of the receiver's clock a version may be before it is refused.
pub const MAX_SKEW_MS: i64 = 5 * 60 * 1000;

/// A record's version: ordered by wall time, then counter, then machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub wall_ms: i64,
    pub counter: u32,
    /// The machine that made the change (its pinned instance identifier).
    pub machine: String,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.wall_ms, self.counter, &self.machine).cmp(&(other.wall_ms, other.counter, &other.machine))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A version was too far ahead of this machine's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkewRefused {
    /// How far ahead of this machine's clock it was.
    pub ahead_ms: i64,
}

/// One machine's clock for one collection.
#[derive(Debug, Clone)]
pub struct Clock {
    machine: String,
    last_wall: i64,
    last_counter: u32,
}

impl Clock {
    pub fn new(machine: &str) -> Clock {
        Clock { machine: machine.to_string(), last_wall: 0, last_counter: 0 }
    }

    /// The version of a change made here at `now_ms`; always after every version this clock has made or seen.
    pub fn tick(&mut self, now_ms: i64) -> Version {
        if now_ms > self.last_wall {
            self.last_wall = now_ms;
            self.last_counter = 0;
        } else if let Some(next) = self.last_counter.checked_add(1) {
            self.last_counter = next;
        } else {
            // The counter is full (a received version can carry any counter): carry into the wall time.
            self.last_wall += 1;
            self.last_counter = 0;
        }
        Version { wall_ms: self.last_wall, counter: self.last_counter, machine: self.machine.clone() }
    }

    /// Takes in a version seen from another machine, so the next one made here is after it; refuses one more
    /// than [`MAX_SKEW_MS`] ahead of `now_ms`.
    pub fn observe(&mut self, seen: &Version, now_ms: i64) -> Result<(), SkewRefused> {
        let ahead = seen.wall_ms - now_ms;
        if ahead > MAX_SKEW_MS {
            return Err(SkewRefused { ahead_ms: ahead });
        }
        self.raise(seen);
        Ok(())
    }

    /// Moves the clock to at least `held`, a version already stored here (no skew check: it was accepted when it
    /// was stored). A clock made fresh after a restart is raised this way, so it never makes a version at or
    /// before one already held.
    pub fn raise(&mut self, held: &Version) {
        if held.wall_ms > self.last_wall {
            self.last_wall = held.wall_ms;
            self.last_counter = held.counter;
        } else if held.wall_ms == self.last_wall && held.counter > self.last_counter {
            self.last_counter = held.counter;
        }
    }
}
