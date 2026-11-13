// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Transform operations: one function per [`TransformOp`], and the chain
//! runner (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9).

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Number, Value};

use super::expr::{
    as_list, charge, cmp_same_class, eval_at, parse_object_path, parse_opt, parse_or, value_class,
    State,
};
use super::{expr_step, op_step, TransformError, TransformOp};

// ---------------------------------------------------------------------------
// The operations
// ---------------------------------------------------------------------------

pub(super) fn apply(
    op: &TransformOp,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    match op {
        TransformOp::Map { expr } => op_map(expr, input, step, state),
        TransformOp::Filter { expr } => op_filter(expr, input, step, state),
        TransformOp::Pick { fields } => op_select(fields, false, input, step, state),
        TransformOp::Omit { fields } => op_select(fields, true, input, step, state),
        TransformOp::Sort { by, desc } => op_sort(by, *desc, input, step, state),
        TransformOp::Group { by } => op_group(by, input, step, state),
        TransformOp::Unique { by } => op_unique(by.as_deref(), input, step, state),
        TransformOp::Slice { start, end } => op_slice(*start, *end, input, step),
        TransformOp::First => op_first(false, input, step, state),
        TransformOp::Last => op_first(true, input, step, state),
        TransformOp::Flatten => op_flatten(input, step, state),
        TransformOp::Join { sep } => op_join(sep, input, step, state),
        TransformOp::Split { sep } => op_split(sep, input, step),
        TransformOp::Merge { with } => op_merge(with, input, step),
        TransformOp::Count => op_count(input, step, state),
        TransformOp::Sum { by } => op_sum(by.as_deref(), input, step, state),
        TransformOp::Min { by } => op_extreme(by.as_deref(), false, input, step, state),
        TransformOp::Max { by } => op_extreme(by.as_deref(), true, input, step, state),
        TransformOp::Exists { expr } => op_exists(expr.as_deref(), input, step, state),
    }
}

fn op_map(
    source: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "map")?;
    let ast = parse_or(source, step)?;
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        out.push(eval_at(&ast, item, index, step, state)?);
    }
    Ok(Value::Array(out))
}

fn op_filter(
    source: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "filter")?;
    let ast = parse_or(source, step)?;
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        match eval_at(&ast, item, index, step, state)? {
            Value::Bool(true) => out.push(item.clone()),
            Value::Bool(false) => {}
            other => {
                return Err(expr_step(
                    step,
                    format!("filter needs true or false, not {other}"),
                ))
            }
        }
    }
    Ok(Value::Array(out))
}

fn op_select(
    fields: &[String],
    drop: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    match input {
        Value::Object(map) => Ok(Value::Object(select_fields(map, fields, drop))),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                charge(state)?;
                match item {
                    Value::Object(map) => out.push(Value::Object(select_fields(map, fields, drop))),
                    _ => return Err(op_step(step, "pick and omit need objects in the list")),
                }
            }
            Ok(Value::Array(out))
        }
        _ => Err(op_step(
            step,
            "pick and omit need an object or a list of objects",
        )),
    }
}

fn select_fields(map: &Map<String, Value>, fields: &[String], drop: bool) -> Map<String, Value> {
    map.iter()
        .filter(|(key, _)| fields.iter().any(|field| field == *key) != drop)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn op_sort(
    by: &str,
    desc: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "sort")?;
    let ast = parse_or(by, step)?;
    let mut keyed = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let key = eval_at(&ast, item, index, step, state)?;
        keyed.push((key, item.clone()));
    }
    if let Some((first, _)) = keyed.first() {
        let class = value_class(first);
        let orderable = matches!(class, "number" | "string" | "bool" | "null");
        if !orderable || keyed.iter().any(|(key, _)| value_class(key) != class) {
            return Err(expr_step(
                step,
                "sort needs keys of one orderable type".to_string(),
            ));
        }
    }
    // The keys are of one class by now, so the comparison cannot fail and the
    // sort stays stable for equal keys.
    keyed.sort_by(|(left, _), (right, _)| {
        let ordering = cmp_same_class(left, right);
        if desc {
            ordering.reverse()
        } else {
            ordering
        }
    });
    Ok(Value::Array(
        keyed.into_iter().map(|(_, item)| item).collect(),
    ))
}

fn op_group(
    by: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "group")?;
    let ast = parse_or(by, step)?;
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let key = eval_at(&ast, item, index, step, state)?;
        let key = key_as_string(&key).map_err(|message| expr_step(step, message))?;
        groups.entry(key).or_default().push(item.clone());
    }
    let mut out = Map::new();
    for (key, group) in groups {
        out.insert(key, Value::Array(group));
    }
    Ok(Value::Object(out))
}

fn key_as_string(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Null => Ok("null".to_string()),
        _ => Err("a group key must be a string, a number or a boolean".to_string()),
    }
}

fn op_unique(
    by: Option<&str>,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "unique")?;
    let ast = parse_opt(by, step)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let key = match &ast {
            Some(ast) => eval_at(ast, item, index, step, state)?,
            None => item.clone(),
        };
        let canonical = serde_json::to_string(&key)
            .map_err(|error| op_step(step, &format!("a value that cannot be compared: {error}")))?;
        if seen.insert(canonical) {
            out.push(item.clone());
        }
    }
    Ok(Value::Array(out))
}

fn op_slice(
    start: usize,
    end: Option<usize>,
    input: &Value,
    step: usize,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "slice")?;
    let end = end.unwrap_or(items.len()).min(items.len());
    let start = start.min(end);
    Ok(Value::Array(items[start..end].to_vec()))
}

fn op_first(
    last: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, if last { "last" } else { "first" })?;
    charge(state)?;
    let found = if last { items.last() } else { items.first() };
    Ok(found.cloned().unwrap_or(Value::Null))
}

fn op_flatten(input: &Value, step: usize, state: &mut State) -> Result<Value, TransformError> {
    let items = as_list(input, step, "flatten")?;
    let mut out = Vec::new();
    for item in items {
        charge(state)?;
        match item {
            Value::Array(inner) => out.extend(inner.iter().cloned()),
            other => out.push(other.clone()),
        }
    }
    Ok(Value::Array(out))
}

fn op_join(
    sep: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "join")?;
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        charge(state)?;
        match item {
            Value::String(text) => parts.push(text.clone()),
            Value::Number(number) => parts.push(number.to_string()),
            _ => return Err(op_step(step, "join needs strings or numbers")),
        }
    }
    Ok(Value::String(parts.join(sep)))
}

fn op_split(sep: &str, input: &Value, step: usize) -> Result<Value, TransformError> {
    let text = input
        .as_str()
        .ok_or_else(|| op_step(step, "split needs a string"))?;
    if sep.is_empty() {
        return Err(op_step(
            step,
            "split needs a separator of at least one character",
        ));
    }
    Ok(Value::Array(
        text.split(sep)
            .map(|part| Value::String(part.to_string()))
            .collect(),
    ))
}

fn op_merge(with: &Value, input: &Value, step: usize) -> Result<Value, TransformError> {
    let base = input
        .as_object()
        .ok_or_else(|| op_step(step, "merge needs an object"))?;
    let extra = with
        .as_object()
        .ok_or_else(|| op_step(step, "merge needs an object to merge"))?;
    let mut out = base.clone();
    for (key, value) in extra {
        out.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(out))
}

fn op_count(input: &Value, step: usize, state: &mut State) -> Result<Value, TransformError> {
    match input {
        Value::Array(items) => {
            for _ in items {
                charge(state)?;
            }
            Ok(Value::Number(Number::from(items.len() as u64)))
        }
        Value::Object(map) => {
            for _ in map {
                charge(state)?;
            }
            Ok(Value::Number(Number::from(map.len() as u64)))
        }
        Value::String(text) => Ok(Value::Number(Number::from(text.chars().count() as u64))),
        _ => Err(op_step(step, "count needs a list, an object or a string")),
    }
}

fn op_sum(
    by: Option<&str>,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "sum")?;
    let ast = parse_opt(by, step)?;
    let mut whole: i128 = 0;
    let mut decimal = 0.0f64;
    let mut all_whole = true;
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let value = match &ast {
            Some(ast) => eval_at(ast, item, index, step, state)?,
            None => item.clone(),
        };
        let number = match &value {
            Value::Number(number) => number,
            _ => return Err(op_step(step, "sum needs numbers")),
        };
        match number.as_i64() {
            Some(integer) => {
                whole += integer as i128;
                decimal += integer as f64;
            }
            None => {
                all_whole = false;
                decimal += number
                    .as_f64()
                    .ok_or_else(|| op_step(step, "sum needs numbers"))?;
            }
        }
    }
    if all_whole && whole >= i64::MIN as i128 && whole <= i64::MAX as i128 {
        Ok(Value::Number(Number::from(whole as i64)))
    } else {
        Number::from_f64(decimal)
            .map(Value::Number)
            .ok_or_else(|| op_step(step, "the sum is not finite"))
    }
}

fn op_extreme(
    by: Option<&str>,
    largest: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, if largest { "max" } else { "min" })?;
    let ast = parse_opt(by, step)?;
    let mut best: Option<(f64, Value)> = None;
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let value = match &ast {
            Some(ast) => eval_at(ast, item, index, step, state)?,
            None => item.clone(),
        };
        let number = value
            .as_f64()
            .ok_or_else(|| op_step(step, "min and max need numbers"))?;
        let better = match &best {
            None => true,
            Some((chosen, _)) => {
                if largest {
                    number > *chosen
                } else {
                    number < *chosen
                }
            }
        };
        if better {
            best = Some((number, value));
        }
    }
    Ok(best.map(|(_, value)| value).unwrap_or(Value::Null))
}

fn op_exists(
    source: Option<&str>,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    match input {
        Value::Array(items) => match source {
            None => Ok(Value::Bool(!items.is_empty())),
            Some(source) => {
                let ast = parse_or(source, step)?;
                for (index, item) in items.iter().enumerate() {
                    charge(state)?;
                    match eval_at(&ast, item, index, step, state)? {
                        Value::Bool(true) => return Ok(Value::Bool(true)),
                        Value::Bool(false) => {}
                        other => {
                            return Err(expr_step(
                                step,
                                format!("exists needs true or false, not {other}"),
                            ))
                        }
                    }
                }
                Ok(Value::Bool(false))
            }
        },
        Value::Object(_) => {
            let source =
                source.ok_or_else(|| op_step(step, "exists needs a path for an object"))?;
            let path = parse_object_path(source).map_err(|message| expr_step(step, message))?;
            Ok(Value::Bool(
                walk_path(input, &path).is_some_and(|value| !value.is_null()),
            ))
        }
        _ => Err(op_step(step, "exists needs a list or an object")),
    }
}

fn walk_path<'a>(root: &'a Value, segments: &[String]) -> Option<&'a Value> {
    let mut current = root;
    for segment in segments {
        current = current.get(segment.as_str())?;
    }
    Some(current)
}
