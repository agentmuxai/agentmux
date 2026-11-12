// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Run state: the named values a graph writes, and how writes combine
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.2).
//!
//! **Module B of the engine**, with `checkpoint.rs`. The types and the cap are
//! fixed here; [`RunState::write`] and the rest are the module's job. Its
//! acceptance tests are `state_acceptance_tests.rs`.
//!
//! # Reducers
//!
//! A variable declares how its writes combine. A name with no declaration is
//! `replace`. The reducer decides the value, and [`merged_bits`] decides the
//! taint bits of the result (the same rule for every reducer).
//!
//! | Reducer | First write | Next write `w` onto current `c` | Type error when |
//! |---|---|---|---|
//! | `replace` | `w` | `w` | never |
//! | `append` | `[w]` | `c` with `w` pushed as one element (a list `w` is one element) | `c` is not a list |
//! | `merge` | `w` | shallow merge, `w`'s keys win | `c` or `w` is not an object |
//! | `add` | `w` | `c + w`; integers stay integers (`1 + 2` is `3`) | `c` or `w` is not a number |

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::value::Tv;

/// The state of a run is at most this many bytes of JSON (§3.2).
pub const STATE_CAP_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reducer {
    #[default]
    Replace,
    Append,
    Merge,
    Add,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// The write does not fit the reducer (`append` onto a number, `add` of text, `merge` of a list).
    #[error("state: {name}: {reducer:?} needs {needs}, got {got}")]
    Type { name: String, reducer: Reducer, needs: &'static str, got: &'static str },
    /// The state would pass [`STATE_CAP_BYTES`]. The write is not applied.
    #[error("state: over the limit of {cap} bytes (would be {bytes})")]
    TooLarge { bytes: usize, cap: usize },
    /// A name that cannot be a variable (empty, or holding a dot or whitespace).
    #[error("state: {0:?} is not a variable name")]
    Name(String),
}

/// What a checkpoint stores of the state: every value with its bits, and the
/// reducer each name declared.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub vars: BTreeMap<String, Tv>,
    pub reducers: BTreeMap<String, Reducer>,
}

/// The bits a write leaves on a variable: tainted if the write or the current
/// value is (a reducer other than `replace` keeps what was there), derived if
/// either is and the result is not tainted. For `replace` the new value's bits
/// are the result: the old value is gone.
pub fn merged_bits(reducer: Reducer, current: Option<&Tv>, write: &Tv) -> (bool, bool) {
    match (reducer, current) {
        (Reducer::Replace, _) | (_, None) => (write.tainted, write.derived),
        (_, Some(c)) => super::value::join(&[c, write]),
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunState {
    snapshot: StateSnapshot,
}

impl RunState {
    pub fn new() -> Self {
        Self::default()
    }

    /// A state restored from a checkpoint. The bits come back exactly as saved.
    pub fn from_snapshot(snapshot: StateSnapshot) -> Self {
        Self { snapshot }
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot.clone()
    }

    pub fn reducer_of(&self, name: &str) -> Reducer {
        self.snapshot.reducers.get(name).copied().unwrap_or_default()
    }

    pub fn get(&self, name: &str) -> Option<&Tv> {
        self.snapshot.vars.get(name)
    }

    /// Declare how `name`'s writes combine. Declaring the same reducer again is
    /// fine. Once the name has a value its reducer cannot change: another one is
    /// a [`StateError::Type`] with `needs` "the reducer it was written under".
    /// A bad name is a [`StateError::Name`].
    ///
    /// **Module B.** Not written yet.
    pub fn declare(&mut self, name: &str, reducer: Reducer) -> Result<(), StateError> {
        let _ = (name, reducer);
        todo!("module B: RunState::declare")
    }

    /// Apply a write under the variable's reducer (see the table above). On an
    /// error nothing changes. After a write the state is at most [`STATE_CAP_BYTES`].
    ///
    /// **Module B.** Not written yet.
    pub fn write(&mut self, name: &str, value: Tv) -> Result<(), StateError> {
        let _ = (name, value);
        todo!("module B: RunState::write")
    }

    /// The size of the state as JSON, the number [`STATE_CAP_BYTES`] bounds:
    /// the length of `serde_json::to_string(&snapshot())`.
    ///
    /// **Module B.** Not written yet.
    pub fn size_bytes(&self) -> usize {
        todo!("module B: RunState::size_bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_bits_a_write_leaves() {
        let (clean, tainted, derived) = (Tv::clean(json!(1)), Tv::tainted(json!("x")), Tv::derived(json!(2)));
        // replace: the old value is gone, so only the new one counts.
        assert_eq!(merged_bits(Reducer::Replace, Some(&tainted), &clean), (false, false));
        assert_eq!(merged_bits(Reducer::Replace, None, &tainted), (true, false));
        // the others keep what was there: tainted stays tainted.
        for r in [Reducer::Append, Reducer::Merge, Reducer::Add] {
            assert_eq!(merged_bits(r, Some(&tainted), &clean), (true, false), "{r:?}");
            assert_eq!(merged_bits(r, Some(&clean), &tainted), (true, false), "{r:?}");
            assert_eq!(merged_bits(r, Some(&clean), &derived), (false, true), "{r:?}");
            assert_eq!(merged_bits(r, Some(&clean), &clean), (false, false), "{r:?}");
            assert_eq!(merged_bits(r, None, &clean), (false, false), "{r:?}");
        }
    }

    #[test]
    fn a_reducer_is_written_in_snake_case() {
        assert_eq!(serde_json::to_value(Reducer::Append).unwrap(), json!("append"));
        assert_eq!(Reducer::default(), Reducer::Replace);
    }
}
