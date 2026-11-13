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
//! taint bits of the result (the same rule for every reducer: none clears a bit).
//!
//! | Reducer | First write | Next write `w` onto current `c` | Type error when |
//! |---|---|---|---|
//! | `replace` | `w` | `w` | never |
//! | `append` | `[w]` | `c` with `w` pushed as one element (a list `w` is one element) | `c` is not a list |
//! | `merge` | `w` | shallow merge, `w`'s keys win | `c` or `w` is not an object |
//! | `add` | `w` | `c + w`; integers stay integers (`1 + 2` is `3`) | `c` or `w` is not a number |

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::value::{join, Tv};

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

/// The bits a write leaves on a variable: **no reducer clears a bit** (spec
/// 4.1 "a reducer cannot clear taint", 4.2 "tainted if any contributing write
/// is tainted"). With nothing there yet they are the write's; after that they
/// are [`join`] of the current value and the write, `replace` included: a clean
/// write over a tainted value leaves it tainted, because a person cannot tell
/// from the variable which text is still in it.
pub fn merged_bits(current: Option<&Tv>, write: &Tv) -> (bool, bool) {
    match current {
        None => (write.tainted, write.derived),
        Some(c) => join(&[c, write]),
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
    pub fn declare(&mut self, name: &str, reducer: Reducer) -> Result<(), StateError> {
        check_name(name)?;
        if self.snapshot.vars.contains_key(name) && self.reducer_of(name) != reducer {
            return Err(StateError::Type {
                name: name.to_string(),
                reducer,
                needs: "the reducer it was written under",
                got: "a different reducer",
            });
        }
        self.snapshot.reducers.insert(name.to_string(), reducer);
        Ok(())
    }

    /// Apply a write under the variable's reducer (see the table above). On an
    /// error nothing changes. After a write the state is at most [`STATE_CAP_BYTES`].
    pub fn write(&mut self, name: &str, value: Tv) -> Result<(), StateError> {
        check_name(name)?;
        let reducer = self.reducer_of(name);
        let current = self.snapshot.vars.get(name);
        let new_value = match reducer {
            Reducer::Replace => value.v.clone(),
            Reducer::Append => append_value(name, current, &value)?,
            Reducer::Merge => merge_value(name, current, &value)?,
            Reducer::Add => add_value(name, current, &value)?,
        };
        let (tainted, derived) = merged_bits(current, &value);
        // Apply to a copy and measure before committing, so an over-cap write
        // leaves the state exactly as it was.
        let mut candidate = self.snapshot.clone();
        candidate.vars.insert(name.to_string(), Tv { v: new_value, tainted, derived });
        let bytes = snapshot_len(&candidate);
        if bytes > STATE_CAP_BYTES {
            return Err(StateError::TooLarge { bytes, cap: STATE_CAP_BYTES });
        }
        self.snapshot = candidate;
        Ok(())
    }

    /// The size of the state as JSON, the number [`STATE_CAP_BYTES`] bounds:
    /// the length of `serde_json::to_string(&snapshot())`.
    pub fn size_bytes(&self) -> usize {
        snapshot_len(&self.snapshot)
    }
}

/// Refuse a name that cannot be a variable: empty, with a dot, or with whitespace.
fn check_name(name: &str) -> Result<(), StateError> {
    if name.is_empty() || name.contains('.') || name.chars().any(char::is_whitespace) {
        return Err(StateError::Name(name.to_string()));
    }
    Ok(())
}

/// What a value is, for the `got` half of a [`StateError::Type`].
fn kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "a list",
        serde_json::Value::Object(_) => "an object",
    }
}

fn type_error(name: &str, reducer: Reducer, needs: &'static str, got: &serde_json::Value) -> StateError {
    StateError::Type { name: name.to_string(), reducer, needs, got: kind(got) }
}

/// The JSON length of a snapshot, the measure [`STATE_CAP_BYTES`] bounds.
fn snapshot_len(snapshot: &StateSnapshot) -> usize {
    serde_json::to_string(snapshot).map(|text| text.len()).unwrap_or(usize::MAX)
}

/// `append`: the first write makes `[w]`; after that `w` is one more element.
fn append_value(name: &str, current: Option<&Tv>, write: &Tv) -> Result<serde_json::Value, StateError> {
    let Some(current) = current else {
        return Ok(serde_json::Value::Array(vec![write.v.clone()]));
    };
    let serde_json::Value::Array(items) = &current.v else {
        return Err(type_error(name, Reducer::Append, "a list", &current.v));
    };
    let mut next = items.clone();
    next.push(write.v.clone());
    Ok(serde_json::Value::Array(next))
}

/// `merge`: a shallow object merge, `w`'s keys winning. A merge channel holds
/// objects, so the first write must be one too.
fn merge_value(name: &str, current: Option<&Tv>, write: &Tv) -> Result<serde_json::Value, StateError> {
    let Some(write_object) = write.v.as_object() else {
        return Err(type_error(name, Reducer::Merge, "an object", &write.v));
    };
    let Some(current) = current else {
        return Ok(serde_json::Value::Object(write_object.clone()));
    };
    let Some(current_object) = current.v.as_object() else {
        return Err(type_error(name, Reducer::Merge, "an object", &current.v));
    };
    let mut merged = current_object.clone();
    for (key, value) in write_object {
        merged.insert(key.clone(), value.clone());
    }
    Ok(serde_json::Value::Object(merged))
}

/// `add`: numbers only; two integers stay an integer, any float makes a float.
fn add_value(name: &str, current: Option<&Tv>, write: &Tv) -> Result<serde_json::Value, StateError> {
    let Some(write_number) = write.v.as_number() else {
        return Err(type_error(name, Reducer::Add, "a number", &write.v));
    };
    let Some(current) = current else {
        return Ok(write.v.clone());
    };
    let Some(current_number) = current.v.as_number() else {
        return Err(type_error(name, Reducer::Add, "a number", &current.v));
    };
    let sum = add_numbers(current_number, write_number)
        .ok_or_else(|| type_error(name, Reducer::Add, "a number", &write.v))?;
    Ok(serde_json::Value::Number(sum))
}

/// The sum of two JSON numbers, keeping integers integers where it can.
fn add_numbers(a: &serde_json::Number, b: &serde_json::Number) -> Option<serde_json::Number> {
    if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
        if let Some(sum) = x.checked_add(y) {
            return Some(serde_json::Number::from(sum));
        }
    }
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        if let Some(sum) = x.checked_add(y) {
            return Some(serde_json::Number::from(sum));
        }
    }
    serde_json::Number::from_f64(a.as_f64()? + b.as_f64()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_bits_a_write_leaves_and_no_reducer_clears_one() {
        let (clean, tainted, derived) = (Tv::clean(json!(1)), Tv::tainted(json!("x")), Tv::derived(json!(2)));
        assert_eq!(merged_bits(None, &tainted), (true, false));
        assert_eq!(merged_bits(None, &clean), (false, false));
        assert_eq!(merged_bits(Some(&tainted), &clean), (true, false), "a clean write over a tainted value");
        assert_eq!(merged_bits(Some(&clean), &tainted), (true, false));
        assert_eq!(merged_bits(Some(&clean), &derived), (false, true));
        assert_eq!(merged_bits(Some(&derived), &clean), (false, true), "a derived value stays derived");
        assert_eq!(merged_bits(Some(&tainted), &derived), (true, false), "tainted is the stronger claim");
        assert_eq!(merged_bits(Some(&clean), &clean), (false, false));
    }

    #[test]
    fn a_reducer_is_written_in_snake_case() {
        assert_eq!(serde_json::to_value(Reducer::Append).unwrap(), json!("append"));
        assert_eq!(Reducer::default(), Reducer::Replace);
    }
}
