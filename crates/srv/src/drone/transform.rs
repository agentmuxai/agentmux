// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Transform node's evaluator: shape lists and objects with a restricted
//! expression language and no code execution
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9).
//!
//! **Module A of the engine.** The types, the bounds and [`taint_out`] are
//! fixed here; [`evaluate`] is the module's job. Its acceptance tests are
//! `transform_acceptance_tests.rs`. The language itself (the tokenizer, the
//! parser and the evaluator) is in `expr.rs`, the operations are in `ops.rs`,
//! and the module's own unit tests are in `tests.rs`; all three are private to
//! this module.
//!
//! # Expression grammar
//!
//! Lowest precedence first; all binary operators are left associative.
//!
//! ```text
//! expr    := or
//! or      := and ( "||" and )*
//! and     := not ( "&&" not )*
//! not     := "!" not | cmp
//! cmp     := sum ( ( "==" | "!=" | "<" | "<=" | ">" | ">=" | "contains" | "matches" | "in_set" ) sum )?
//! sum     := prod ( ( "+" | "-" ) prod )*
//! prod    := unary ( ( "*" | "/" | "%" ) unary )*
//! unary   := "-" unary | atom
//! atom    := number | string | "true" | "false" | "null" | list | call | path | "(" expr ")"
//! list    := "[" ( expr ( "," expr )* )? "]"
//! call    := name "(" ( expr ( "," expr )* )? ")"      // lower, trunc, default, exists
//! path    := name ( "." name )*                         // item, item.user.login, index
//! string  := 'text' or "text"                           // no escapes beyond \' \" \\
//! ```
//!
//! * `item` is the current element and `index` its position (0-based) inside
//!   `map`, `filter` and the key expressions (`by`). A missing path is `null`,
//!   never an error.
//! * `a contains b`: a string contains a substring, or a list holds an element.
//!   `a matches 're'`: the Rust `regex` crate (linear time) on a string. The
//!   pattern must be a string literal, compiled once when the chain is read, so
//!   no element pays for a compile.
//!   `a in_set [..]`: the list holds `a` (JSON equality).
//! * `lower(s)`, `trunc(s, n)` (at most `n` characters), `default(x, d)` (`d`
//!   when `x` is null), `exists(path)` (the path is not null). **No other
//!   function exists**, no variable, no assignment, no loop, no recursion, no
//!   call to the host and no network: an unknown name is [`TransformError::Expr`].
//! * Arithmetic is on numbers only; `/` by zero and `%` by zero are errors; an
//!   integer result stays an integer (`1 + 2` is `3`, not `3.0`).
//! * Comparing values of different types is `false` for `==`, `true` for `!=`,
//!   and an error for `<`, `<=`, `>`, `>=`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::value::{join, Tv};

mod condition;
mod expr;
mod ops;

pub use condition::evaluate_condition;
use expr::State;
use ops::apply;

/// An input of at most this many items (§3.9).
pub const MAX_ITEMS: usize = 10_000;
/// An evaluation budget per node: one unit per element an operation visits and
/// per expression node evaluated (§3.9).
pub const MAX_OPERATIONS: u64 = 100_000;
/// An output of at most the state cap, in bytes of JSON (§3.2).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
/// A compiled `matches` pattern may be at most this big: a pattern that would
/// compile to more is an expression error, so a hostile one cannot make the
/// compiler work without bound.
const MAX_REGEX_BYTES: usize = 1024 * 1024;

/// One step of a chain. A chain reads top to bottom, each result feeding the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TransformOp {
    /// A list in, a list out: `expr` per element.
    Map { expr: String },
    /// Keep the elements for which `expr` is true.
    Filter { expr: String },
    /// Keep only these top-level keys of an object, or of every object in a list.
    Pick { fields: Vec<String> },
    /// The opposite of `pick`.
    Omit { fields: Vec<String> },
    /// Stable sort of a list by the value of `by` (an expression from `item`, such as `item.age`).
    Sort {
        by: String,
        #[serde(default)]
        desc: bool,
    },
    /// An object from the key of `by` (as a string) to the list of elements with it, in input order.
    Group { by: String },
    /// Drop repeats, keeping the first: by whole value, or by the value of `by`.
    Unique {
        #[serde(default)]
        by: Option<String>,
    },
    /// `start..end` of a list (`end` absent: to the end); out-of-range ends are clamped.
    Slice {
        start: usize,
        #[serde(default)]
        end: Option<usize>,
    },
    /// The first element, or `null` for an empty list.
    First,
    /// The last element, or `null` for an empty list.
    Last,
    /// One level of lists flattened; non-list elements stay.
    Flatten,
    /// A list of strings or numbers as one string.
    Join { sep: String },
    /// A string as a list of strings.
    Split { sep: String },
    /// The input object with `with` merged over it (shallow; `with` wins).
    Merge { with: Value },
    /// The number of elements of a list, of keys of an object, or characters of a string.
    Count,
    /// The sum of a list of numbers, or of `by` over a list of objects.
    Sum {
        #[serde(default)]
        by: Option<String>,
    },
    /// The smallest, by the same rule as `sum`; `null` for an empty list.
    Min {
        #[serde(default)]
        by: Option<String>,
    },
    /// The largest, by the same rule as `sum`; `null` for an empty list.
    Max {
        #[serde(default)]
        by: Option<String>,
    },
    /// Whether a list has an element for which `expr` is true (any element when `expr` is absent), or an object has a non-null path `expr` (a dotted path from the object itself, with no `item`).
    Exists {
        #[serde(default)]
        expr: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub ops: Vec<TransformOp>,
}

/// Which bound a node went over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundKind {
    Items,
    Operations,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransformError {
    /// A bound of §3.9 was passed. The node fails naming it.
    #[error("transform: over the limit of {limit} {kind:?}")]
    Bound { kind: BoundKind, limit: u64 },
    /// An operation could not be applied (a map on a number, a sum of text...). `index` is its place in the chain.
    #[error("transform: step {index}: {message}")]
    Op { index: usize, message: String },
    /// An expression did not parse or cannot be evaluated (an unknown function, a bad regex, a division by zero).
    #[error("transform: step {index}: expression: {message}")]
    Expr { index: usize, message: String },
}

/// What an operation does to the taint bits of the value it reads (the table
/// of §3.9 and the derived-value rule of §4.5). Returns `(tainted, derived)`.
///
/// * Operations that can carry text (everything but the next line) pass the
///   input's bits on: tainted in, tainted out.
/// * `count`, `sum`, `min`, `max` and `exists` cannot carry text: over tainted
///   input they give a clean value that is **`derived`**, so it is clean as
///   data and still attacker-chosen as control or as a numeric argument.
/// * Over clean input they stay clean; over derived input they stay derived.
pub fn taint_out(op: &TransformOp, input: &Tv) -> (bool, bool) {
    let carries_no_text = matches!(
        op,
        TransformOp::Count
            | TransformOp::Sum { .. }
            | TransformOp::Min { .. }
            | TransformOp::Max { .. }
            | TransformOp::Exists { .. }
    );
    let (tainted, derived) = join(&[input]);
    if carries_no_text && tainted {
        (false, true)
    } else {
        (tainted, derived)
    }
}

/// The bits of a chain's output: [`taint_out`] applied step by step, starting
/// from the input's.
pub fn chain_taint(ops: &[TransformOp], input: &Tv) -> (bool, bool) {
    let mut bits = (input.tainted, input.derived);
    for op in ops {
        let probe = Tv {
            v: Value::Null,
            tainted: bits.0,
            derived: bits.1,
        };
        bits = taint_out(op, &probe);
    }
    bits
}

/// Run `t` over `input`. The output's bits are [`chain_taint`] of the chain.
/// Every bound of §3.9 is checked and named in [`TransformError::Bound`].
pub fn evaluate(t: &Transform, input: &Tv) -> Result<Tv, TransformError> {
    let mut value = input.v.clone();
    check_items(&value)?;
    let mut state = State::default();
    for (step, op) in t.ops.iter().enumerate() {
        value = apply(op, &value, step, &mut state)?;
        check_items(&value)?;
        check_output(&value, step)?;
    }
    let (tainted, derived) = chain_taint(&t.ops, input);
    Ok(Tv {
        v: value,
        tainted,
        derived,
    })
}

/// The item bound: an input list, or a list a step makes, of more than
/// [`MAX_ITEMS`].
fn check_items(value: &Value) -> Result<(), TransformError> {
    match value {
        Value::Array(items) if items.len() > MAX_ITEMS => Err(TransformError::Bound {
            kind: BoundKind::Items,
            limit: MAX_ITEMS as u64,
        }),
        _ => Ok(()),
    }
}

/// The output bound, checked after every step so a runaway intermediate stops
/// early rather than filling memory first.
fn check_output(value: &Value, step: usize) -> Result<(), TransformError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| op_step(step, &format!("the output cannot be measured: {error}")))?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        Err(TransformError::Bound {
            kind: BoundKind::Output,
            limit: MAX_OUTPUT_BYTES as u64,
        })
    } else {
        Ok(())
    }
}

fn op_step(step: usize, message: &str) -> TransformError {
    TransformError::Op {
        index: step,
        message: message.to_string(),
    }
}

fn expr_step(step: usize, message: String) -> TransformError {
    TransformError::Expr {
        index: step,
        message,
    }
}

/// The expressions an op carries, for the validator and the canvas.
pub fn expressions_of(op: &TransformOp) -> Vec<&str> {
    match op {
        TransformOp::Map { expr } | TransformOp::Filter { expr } => vec![expr.as_str()],
        TransformOp::Sort { by, .. } | TransformOp::Group { by } => vec![by.as_str()],
        TransformOp::Unique { by }
        | TransformOp::Sum { by }
        | TransformOp::Min { by }
        | TransformOp::Max { by } => by.iter().map(String::as_str).collect(),
        TransformOp::Exists { expr } => expr.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Every op as it is written in a graph, with a few words for the canvas.
pub fn op_names() -> BTreeMap<&'static str, &'static str> {
    [
        ("map", "an expression per element"),
        ("filter", "keep where true"),
        ("pick", "keep fields"),
        ("omit", "drop fields"),
        ("sort", "by an expression"),
        ("group", "by an expression"),
        ("unique", "drop repeats"),
        ("slice", "start to end"),
        ("first", "first element"),
        ("last", "last element"),
        ("flatten", "one level"),
        ("join", "list to string"),
        ("split", "string to list"),
        ("merge", "object over object"),
        ("count", "how many"),
        ("sum", "total"),
        ("min", "smallest"),
        ("max", "largest"),
        ("exists", "any, or has a path"),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests;
