// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Incremental JSON (iJSON): path-based operations on JSON data.
//! Port of Go's pkg/ijson/ijson.go.

#![allow(dead_code)]
//!
//! iJSON allows expressing JSON updates as a series of commands:
//! - `set`: Set a value at a path
//! - `del`: Delete a value at a path
//! - `append`: Append a value to an array at a path
//!
//! Paths are arrays of string keys and integer indices, e.g.:
//! `["users", 0, "name"]` refers to `data.users[0].name`

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---- Path type ----

/// A JSON path is a sequence of string keys and integer indices.
/// Example: `["users", 0, "name"]` → `data.users[0].name`
pub type Path = Vec<PathElement>;

/// Element of a JSON path: either a string key or integer index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathElement {
    Key(String),
    Index(usize),
}

impl PathElement {
    pub fn key(s: &str) -> Self {
        PathElement::Key(s.to_string())
    }

    pub fn index(i: usize) -> Self {
        PathElement::Index(i)
    }
}

impl std::fmt::Display for PathElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathElement::Key(k) => write!(f, ".{k}"),
            PathElement::Index(i) => write!(f, "[{i}]"),
        }
    }
}

// ---- Command types ----

/// Command type constants.
pub const CMD_SET: &str = "set";
pub const CMD_DEL: &str = "del";
pub const CMD_APPEND: &str = "append";

/// An iJSON command: set, delete, or append.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// Command type: "set", "del", or "append".
    #[serde(rename = "type")]
    pub cmd_type: String,
    /// Path to target element.
    pub path: Vec<Value>,
    /// Data payload (for set and append commands).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Command {
    /// Create a set command.
    pub fn set(path: Path, data: Value) -> Self {
        Self {
            cmd_type: CMD_SET.to_string(),
            path: path_to_values(&path),
            data: Some(data),
        }
    }

    /// Create a delete command.
    pub fn del(path: Path) -> Self {
        Self {
            cmd_type: CMD_DEL.to_string(),
            path: path_to_values(&path),
            data: None,
        }
    }

    /// Create an append command.
    pub fn append(path: Path, data: Value) -> Self {
        Self {
            cmd_type: CMD_APPEND.to_string(),
            path: path_to_values(&path),
            data: Some(data),
        }
    }

    /// Parse the path from JSON values to PathElements.
    pub fn parsed_path(&self) -> Result<Path, String> {
        values_to_path(&self.path)
    }
}

// ---- Options ----

/// Options for set_path operations.
#[derive(Debug, Clone, Default)]
pub struct SetPathOpts {
    /// Allocation budget (0 = unlimited, negative = fail immediately).
    pub budget: i64,
    /// Force: clobber incompatible types at path nodes.
    pub force: bool,
    /// Remove: delete the value at path instead of setting it.
    pub remove: bool,
}

// ---- Errors ----

/// Error type for iJSON operations.
#[derive(Debug, Clone)]
pub enum IJsonError {
    PathError(String),
    TypeError(String),
    BudgetExceeded,
    IndexOutOfBounds { index: usize, len: usize },
}

impl std::fmt::Display for IJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IJsonError::PathError(msg) => write!(f, "path error: {msg}"),
            IJsonError::TypeError(msg) => write!(f, "type error: {msg}"),
            IJsonError::BudgetExceeded => write!(f, "budget exceeded"),
            IJsonError::IndexOutOfBounds { index, len } => {
                write!(f, "index {index} out of bounds (len {len})")
            }
        }
    }
}

impl std::error::Error for IJsonError {}

impl From<String> for IJsonError {
    fn from(s: String) -> Self {
        IJsonError::PathError(s)
    }
}

// ---- Core operations ----

/// Get a value at the specified path within JSON data.
/// Returns `None` if the path doesn't exist (not an error).
pub fn get_path(data: &Value, path: &Path) -> Result<Option<Value>, IJsonError> {
    let mut current = data;
    for elem in path {
        match elem {
            PathElement::Key(key) => match current {
                Value::Object(map) => match map.get(key) {
                    Some(v) => current = v,
                    None => return Ok(None),
                },
                _ => return Ok(None),
            },
            PathElement::Index(idx) => match current {
                Value::Array(arr) => match arr.get(*idx) {
                    Some(v) => current = v,
                    None => return Ok(None),
                },
                _ => return Ok(None),
            },
        }
    }
    Ok(Some(current.clone()))
}

/// Set a value at the specified path within JSON data.
/// Creates intermediate objects/arrays as needed.
/// Returns the modified data.
pub fn set_path(
    data: Value,
    path: &Path,
    value: Value,
    opts: &SetPathOpts,
) -> Result<Value, IJsonError> {
    if opts.budget < 0 {
        return Err(IJsonError::BudgetExceeded);
    }

    if path.is_empty() {
        if opts.remove {
            return Ok(Value::Null);
        }
        return Ok(value);
    }

    set_path_recursive(data, path, 0, value, opts)
}

fn set_path_recursive(
    data: Value,
    path: &Path,
    depth: usize,
    value: Value,
    opts: &SetPathOpts,
) -> Result<Value, IJsonError> {
    if depth >= path.len() {
        if opts.remove {
            return Ok(Value::Null);
        }
        return Ok(value);
    }

    let elem = &path[depth];
    let is_last = depth == path.len() - 1;

    match elem {
        PathElement::Key(key) => {
            let mut map = match data {
                Value::Object(m) => m,
                Value::Null if opts.force || depth > 0 => serde_json::Map::new(),
                _ if opts.force => serde_json::Map::new(),
                _ => {
                    return Err(IJsonError::TypeError(format!(
                        "expected object at path depth {depth}, got {}",
                        value_type_name(&data)
                    )));
                }
            };

            if is_last && opts.remove {
                map.remove(key);
            } else if is_last {
                map.insert(key.clone(), value);
            } else {
                let child = map.remove(key).unwrap_or(Value::Null);
                let new_child = set_path_recursive(child, path, depth + 1, value, opts)?;
                map.insert(key.clone(), new_child);
            }

            Ok(Value::Object(map))
        }
        PathElement::Index(idx) => {
            let mut arr = match data {
                Value::Array(a) => a,
                Value::Null if opts.force || depth > 0 => Vec::new(),
                _ if opts.force => Vec::new(),
                _ => {
                    return Err(IJsonError::TypeError(format!(
                        "expected array at path depth {depth}, got {}",
                        value_type_name(&data)
                    )));
                }
            };

            // Extend array if needed
            while arr.len() <= *idx {
                arr.push(Value::Null);
            }

            if is_last && opts.remove {
                if *idx < arr.len() {
                    arr.remove(*idx);
                }
            } else if is_last {
                arr[*idx] = value;
            } else {
                let child = std::mem::replace(&mut arr[*idx], Value::Null);
                let new_child = set_path_recursive(child, path, depth + 1, value, opts)?;
                arr[*idx] = new_child;
            }

            Ok(Value::Array(arr))
        }
    }
}

/// Apply a single iJSON command to data.
pub fn apply_command(data: Value, cmd: &Command) -> Result<Value, IJsonError> {
    let path = cmd.parsed_path()?;

    match cmd.cmd_type.as_str() {
        CMD_SET => {
            let value = cmd.data.clone().unwrap_or(Value::Null);
            set_path(
                data,
                &path,
                value,
                &SetPathOpts {
                    force: true,
                    ..Default::default()
                },
            )
        }
        CMD_DEL => set_path(
            data,
            &path,
            Value::Null,
            &SetPathOpts {
                remove: true,
                force: true,
                ..Default::default()
            },
        ),
        CMD_APPEND => {
            let value = cmd.data.clone().unwrap_or(Value::Null);
            // Get current array at path, append, set back
            let current = get_path(&data, &path)?;
            let mut arr = match current {
                Some(Value::Array(a)) => a,
                Some(Value::Null) | None => Vec::new(),
                Some(_) => {
                    return Err(IJsonError::TypeError(
                        "append target is not an array".to_string(),
                    ))
                }
            };
            arr.push(value);
            set_path(
                data,
                &path,
                Value::Array(arr),
                &SetPathOpts {
                    force: true,
                    ..Default::default()
                },
            )
        }
        other => Err(IJsonError::PathError(format!(
            "unknown command type: {other}"
        ))),
    }
}

/// Apply a sequence of iJSON commands to data.
pub fn apply_commands(data: Value, commands: &[Command]) -> Result<Value, IJsonError> {
    let mut result = data;
    for cmd in commands {
        result = apply_command(result, cmd)?;
    }
    Ok(result)
}

// ---- Path formatting ----

/// Format a path for display: `$users[0].name`
pub fn format_path(path: &Path) -> String {
    let mut result = String::from("$");
    for elem in path {
        match elem {
            PathElement::Key(k) => {
                // Use bracket notation for keys with special chars
                if k.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !k.is_empty()
                    && !k.chars().next().unwrap().is_ascii_digit()
                {
                    result.push('.');
                    result.push_str(k);
                } else {
                    result.push_str(&format!("[\"{k}\"]"));
                }
            }
            PathElement::Index(i) => {
                result.push_str(&format!("[{i}]"));
            }
        }
    }
    result
}

/// Parse a simple dot-separated path string.
/// "users.0.name" → [Key("users"), Index(0), Key("name")]
pub fn parse_simple_path(s: &str) -> Path {
    if s.is_empty() {
        return vec![];
    }
    s.split('.')
        .map(|part| {
            if let Ok(idx) = part.parse::<usize>() {
                PathElement::Index(idx)
            } else {
                PathElement::Key(part.to_string())
            }
        })
        .collect()
}

// ---- Helpers ----

/// Convert Path to JSON values for serialization.
fn path_to_values(path: &Path) -> Vec<Value> {
    path.iter()
        .map(|elem| match elem {
            PathElement::Key(k) => Value::String(k.clone()),
            PathElement::Index(i) => Value::Number((*i as u64).into()),
        })
        .collect()
}

/// Convert JSON values to Path.
fn values_to_path(values: &[Value]) -> Result<Path, String> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| match v {
            Value::String(s) => Ok(PathElement::Key(s.clone())),
            Value::Number(n) => {
                if let Some(idx) = n.as_u64() {
                    Ok(PathElement::Index(idx as usize))
                } else {
                    Err(format!("path element {i}: expected non-negative integer"))
                }
            }
            _ => Err(format!(
                "path element {i}: expected string or number, got {}",
                value_type_name(v)
            )),
        })
        .collect::<Result<Path, String>>()
        .map_err(|e| e.to_string())
}

/// Get a human-readable type name for a JSON value.
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Parse newline-delimited iJSON commands.
pub fn parse_ijson(data: &str) -> Result<Vec<Command>, String> {
    let mut commands = Vec::new();
    for (i, line) in data.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cmd: Command = serde_json::from_str(line)
            .map_err(|e| format!("line {}: {e}", i + 1))?;
        commands.push(cmd);
    }
    Ok(commands)
}

/// Compact iJSON: apply all commands and produce a single set command.
pub fn compact_ijson(data: &str) -> Result<String, String> {
    let commands = parse_ijson(data)?;
    if commands.is_empty() {
        return Ok(String::new());
    }
    let result = apply_commands(Value::Null, &commands)
        .map_err(|e| format!("apply error: {e}"))?;
    let compact_cmd = Command::set(vec![], result);
    serde_json::to_string(&compact_cmd).map_err(|e| format!("serialize error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_path_element_display() {
        assert_eq!(PathElement::key("name").to_string(), ".name");
        assert_eq!(PathElement::index(0).to_string(), "[0]");
    }

    #[test]
    fn test_format_path() {
        let path = vec![
            PathElement::key("users"),
            PathElement::index(0),
            PathElement::key("name"),
        ];
        assert_eq!(format_path(&path), "$.users[0].name");
    }

    #[test]
    fn test_format_path_empty() {
        assert_eq!(format_path(&vec![]), "$");
    }

    #[test]
    fn test_format_path_special_key() {
        let path = vec![PathElement::key("my-key")];
        assert_eq!(format_path(&path), "$[\"my-key\"]");
    }

    #[test]
    fn test_parse_simple_path() {
        let path = parse_simple_path("users.0.name");
        assert_eq!(
            path,
            vec![
                PathElement::key("users"),
                PathElement::index(0),
                PathElement::key("name"),
            ]
        );
    }

    #[test]
    fn test_parse_simple_path_empty() {
        assert!(parse_simple_path("").is_empty());
    }

    #[test]
    fn test_get_path_object() {
        let data = json!({"users": [{"name": "Alice"}, {"name": "Bob"}]});
        let path = vec![
            PathElement::key("users"),
            PathElement::index(0),
            PathElement::key("name"),
        ];
        let result = get_path(&data, &path).unwrap();
        assert_eq!(result, Some(json!("Alice")));
    }

    #[test]
    fn test_get_path_missing() {
        let data = json!({"a": 1});
        let path = vec![PathElement::key("b")];
        let result = get_path(&data, &path).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_path_empty() {
        let data = json!({"a": 1});
        let result = get_path(&data, &vec![]).unwrap();
        assert_eq!(result, Some(json!({"a": 1})));
    }

    #[test]
    fn test_get_path_deep_missing() {
        let data = json!({"a": {"b": 1}});
        let path = vec![PathElement::key("a"), PathElement::key("c")];
        let result = get_path(&data, &path).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_set_path_simple() {
        let data = json!({});
        let path = vec![PathElement::key("name")];
        let result = set_path(data, &path, json!("Alice"), &SetPathOpts::default()).unwrap();
        assert_eq!(result, json!({"name": "Alice"}));
    }

    #[test]
    fn test_set_path_nested() {
        let data = json!({});
        let path = vec![PathElement::key("a"), PathElement::key("b")];
        let result = set_path(
            data,
            &path,
            json!(42),
            &SetPathOpts {
                force: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result, json!({"a": {"b": 42}}));
    }

    #[test]
    fn test_set_path_array() {
        let data = json!({"items": [1, 2, 3]});
        let path = vec![PathElement::key("items"), PathElement::index(1)];
        let result = set_path(data, &path, json!(99), &SetPathOpts::default()).unwrap();
        assert_eq!(result, json!({"items": [1, 99, 3]}));
    }

    #[test]
    fn test_set_path_array_extend() {
        let data = json!({"items": []});
        let path = vec![PathElement::key("items"), PathElement::index(2)];
        let result = set_path(data, &path, json!("new"), &SetPathOpts::default()).unwrap();
        assert_eq!(result, json!({"items": [null, null, "new"]}));
    }

    #[test]
    fn test_set_path_remove() {
        let data = json!({"a": 1, "b": 2});
        let path = vec![PathElement::key("a")];
        let result = set_path(
            data,
            &path,
            Value::Null,
            &SetPathOpts {
                remove: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result, json!({"b": 2}));
    }

    #[test]
    fn test_set_path_empty_path() {
        let data = json!({"old": true});
        let result =
            set_path(data, &vec![], json!({"new": true}), &SetPathOpts::default()).unwrap();
        assert_eq!(result, json!({"new": true}));
    }

    #[test]
    fn test_set_path_budget_exceeded() {
        let data = json!({});
        let path = vec![PathElement::key("a")];
        let result = set_path(
            data,
            &path,
            json!(1),
            &SetPathOpts {
                budget: -1,
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(IJsonError::BudgetExceeded)));
    }

    #[test]
    fn test_command_set() {
        let cmd = Command::set(
            vec![PathElement::key("name")],
            json!("Alice"),
        );
        assert_eq!(cmd.cmd_type, CMD_SET);
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"set\""));
    }

    #[test]
    fn test_command_del() {
        let cmd = Command::del(vec![PathElement::key("name")]);
        assert_eq!(cmd.cmd_type, CMD_DEL);
        assert!(cmd.data.is_none());
    }

    #[test]
    fn test_command_append() {
        let cmd = Command::append(vec![PathElement::key("items")], json!(42));
        assert_eq!(cmd.cmd_type, CMD_APPEND);
    }

    #[test]
    fn test_apply_command_set() {
        let data = json!({});
        let cmd = Command::set(vec![PathElement::key("x")], json!(1));
        let result = apply_command(data, &cmd).unwrap();
        assert_eq!(result, json!({"x": 1}));
    }

    #[test]
    fn test_apply_command_del() {
        let data = json!({"x": 1, "y": 2});
        let cmd = Command::del(vec![PathElement::key("x")]);
        let result = apply_command(data, &cmd).unwrap();
        assert_eq!(result, json!({"y": 2}));
    }

    #[test]
    fn test_apply_command_append() {
        let data = json!({"items": [1, 2]});
        let cmd = Command::append(vec![PathElement::key("items")], json!(3));
        let result = apply_command(data, &cmd).unwrap();
        assert_eq!(result, json!({"items": [1, 2, 3]}));
    }

    #[test]
    fn test_apply_command_append_create() {
        let data = json!({});
        let cmd = Command::append(vec![PathElement::key("items")], json!(1));
        let result = apply_command(data, &cmd).unwrap();
        assert_eq!(result, json!({"items": [1]}));
    }

    #[test]
    fn test_apply_commands_sequence() {
        let commands = vec![
            Command::set(vec![], json!({})),
            Command::set(vec![PathElement::key("name")], json!("Alice")),
            Command::set(vec![PathElement::key("age")], json!(30)),
        ];
        let result = apply_commands(Value::Null, &commands).unwrap();
        assert_eq!(result, json!({"name": "Alice", "age": 30}));
    }

    #[test]
    fn test_parse_ijson() {
        let input = r#"{"type":"set","path":[],"data":{}}
{"type":"set","path":["x"],"data":1}"#;
        let commands = parse_ijson(input).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].cmd_type, CMD_SET);
        assert_eq!(commands[1].cmd_type, CMD_SET);
    }

    #[test]
    fn test_parse_ijson_empty_lines() {
        let input = "\n\n{\"type\":\"set\",\"path\":[],\"data\":1}\n\n";
        let commands = parse_ijson(input).unwrap();
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn test_compact_ijson() {
        let input = r#"{"type":"set","path":[],"data":{}}
{"type":"set","path":["x"],"data":1}
{"type":"set","path":["y"],"data":2}"#;
        let result = compact_ijson(input).unwrap();
        let cmd: Command = serde_json::from_str(&result).unwrap();
        assert_eq!(cmd.cmd_type, CMD_SET);
        let data = cmd.data.unwrap();
        assert_eq!(data, json!({"x": 1, "y": 2}));
    }

    #[test]
    fn test_command_serde_roundtrip() {
        let cmd = Command::set(
            vec![PathElement::key("a"), PathElement::index(0)],
            json!({"nested": true}),
        );
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.cmd_type, CMD_SET);
        assert_eq!(parsed.path.len(), 2);
    }

    #[test]
    fn test_path_element_serde() {
        let path = vec![PathElement::key("users"), PathElement::index(0)];
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(json, r#"["users",0]"#);
        let parsed: Path = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, path);
    }

    #[test]
    fn test_ijson_error_display() {
        assert_eq!(
            IJsonError::PathError("bad path".to_string()).to_string(),
            "path error: bad path"
        );
        assert_eq!(IJsonError::BudgetExceeded.to_string(), "budget exceeded");
        assert_eq!(
            IJsonError::IndexOutOfBounds { index: 5, len: 3 }.to_string(),
            "index 5 out of bounds (len 3)"
        );
    }

    #[test]
    fn test_set_path_force_clobber() {
        // Setting a nested path through a non-object value with force=true
        let data = json!({"a": "string_value"});
        let path = vec![PathElement::key("a"), PathElement::key("b")];
        let result = set_path(
            data,
            &path,
            json!(1),
            &SetPathOpts {
                force: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result, json!({"a": {"b": 1}}));
    }

    #[test]
    fn test_value_type_name() {
        assert_eq!(value_type_name(&Value::Null), "null");
        assert_eq!(value_type_name(&json!(true)), "bool");
        assert_eq!(value_type_name(&json!(42)), "number");
        assert_eq!(value_type_name(&json!("hi")), "string");
        assert_eq!(value_type_name(&json!([])), "array");
        assert_eq!(value_type_name(&json!({})), "object");
    }
}
