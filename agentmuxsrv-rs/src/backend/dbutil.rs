// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Database utility helpers for converting between SQL row maps and Rust types.
//! Port of Go's `pkg/util/dbutil/dbutil.go`.

#![allow(dead_code)]
//!
//! Provides "QuickSet" functions for hydrating struct fields from database result
//! maps, and "QuickJson" functions for serializing values to JSON strings/bytes
//! suitable for database storage.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A row from the database: column-name → dynamic value.
pub type DbRow = HashMap<String, Value>;

// ---- QuickSet Functions ----

/// Set a string field from a DB row map.
/// Handles both string and integer (formatted as string) values.
pub fn quick_set_str(target: &mut String, row: &DbRow, name: &str) {
    if let Some(v) = row.get(name) {
        match v {
            Value::String(s) => *target = s.clone(),
            Value::Number(n) => *target = n.to_string(),
            _ => {}
        }
    }
}

/// Set an i32 field from a DB row map.
pub fn quick_set_int(target: &mut i32, row: &DbRow, name: &str) {
    if let Some(v) = row.get(name) {
        if let Some(n) = v.as_i64() {
            *target = n as i32;
        }
    }
}

/// Set an i64 field from a DB row map.
pub fn quick_set_int64(target: &mut i64, row: &DbRow, name: &str) {
    if let Some(v) = row.get(name) {
        if let Some(n) = v.as_i64() {
            *target = n;
        }
    }
}

/// Set a nullable i64 field from a DB row map.
pub fn quick_set_nullable_int64(target: &mut Option<i64>, row: &DbRow, name: &str) {
    if let Some(v) = row.get(name) {
        if v.is_null() {
            *target = None;
        } else if let Some(n) = v.as_i64() {
            *target = Some(n);
        }
    }
}

/// Set a bool field from a DB row map.
/// Handles integer values (> 0 = true) and boolean values.
pub fn quick_set_bool(target: &mut bool, row: &DbRow, name: &str) {
    if let Some(v) = row.get(name) {
        match v {
            Value::Bool(b) => *target = *b,
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    *target = i > 0;
                }
            }
            _ => {}
        }
    }
}

/// Set a byte vector from a DB row map (from string value).
pub fn quick_set_bytes(target: &mut Vec<u8>, row: &DbRow, name: &str) {
    if let Some(v) = row.get(name) {
        if let Some(s) = v.as_str() {
            *target = s.as_bytes().to_vec();
        }
    }
}

/// Deserialize a JSON value from a DB row into a target.
/// Defaults to `{}` (empty object) if the value is missing or empty.
pub fn quick_set_json<T: DeserializeOwned + Default>(target: &mut T, row: &DbRow, name: &str) {
    let bytes = get_json_bytes(row, name, "{}");
    if let Ok(v) = serde_json::from_slice::<T>(&bytes) {
        *target = v;
    }
}

/// Deserialize a nullable JSON value from a DB row.
/// Defaults to `null` if the value is missing or empty.
pub fn quick_set_nullable_json<T: DeserializeOwned>(
    target: &mut Option<T>,
    row: &DbRow,
    name: &str,
) {
    let bytes = get_json_bytes(row, name, "null");
    if let Ok(v) = serde_json::from_slice::<Option<T>>(&bytes) {
        *target = v;
    }
}

/// Deserialize a JSON array from a DB row.
/// Defaults to `[]` if the value is missing or empty.
pub fn quick_set_json_arr<T: DeserializeOwned>(target: &mut Vec<T>, row: &DbRow, name: &str) {
    let bytes = get_json_bytes(row, name, "[]");
    if let Ok(v) = serde_json::from_slice::<Vec<T>>(&bytes) {
        *target = v;
    }
}

// ---- QuickJson Functions ----

/// Serialize a value to a JSON string, returning `"{}"` if None.
pub fn quick_json<T: Serialize>(v: Option<&T>) -> String {
    match v {
        Some(val) => serde_json::to_string(val).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    }
}

/// Serialize a value to JSON bytes, returning `b"{}"` if None.
pub fn quick_json_bytes<T: Serialize>(v: Option<&T>) -> Vec<u8> {
    match v {
        Some(val) => serde_json::to_vec(val).unwrap_or_else(|_| b"{}".to_vec()),
        None => b"{}".to_vec(),
    }
}

/// Serialize a nullable value to a JSON string, returning `"null"` if None.
pub fn quick_nullable_json<T: Serialize>(v: Option<&T>) -> String {
    match v {
        Some(val) => serde_json::to_string(val).unwrap_or_else(|_| "null".to_string()),
        None => "null".to_string(),
    }
}

/// Serialize a value to a JSON array string, returning `"[]"` if None.
pub fn quick_json_arr<T: Serialize>(v: Option<&T>) -> String {
    match v {
        Some(val) => serde_json::to_string(val).unwrap_or_else(|_| "[]".to_string()),
        None => "[]".to_string(),
    }
}

/// Serialize a value to JSON array bytes, returning `b"[]"` if None.
pub fn quick_json_arr_bytes<T: Serialize>(v: Option<&T>) -> Vec<u8> {
    match v {
        Some(val) => serde_json::to_vec(val).unwrap_or_else(|_| b"[]".to_vec()),
        None => b"[]".to_vec(),
    }
}

// ---- Parse Functions ----

/// Parse a JSON string into a HashMap.
/// If `force_make` is true, always returns a map (empty on error/empty input).
/// If `force_make` is false, returns None on empty input or parse error.
pub fn parse_json_map(val: &str, force_make: bool) -> Option<HashMap<String, Value>> {
    if val.is_empty() {
        return if force_make { Some(HashMap::new()) } else { None };
    }
    match serde_json::from_str(val) {
        Ok(map) => Some(map),
        Err(_) if force_make => Some(HashMap::new()),
        Err(_) => None,
    }
}

/// Parse a JSON string into a Vec. Returns empty vec on error.
pub fn parse_json_arr<T: DeserializeOwned>(val: &str) -> Vec<T> {
    if val.is_empty() {
        return Vec::new();
    }
    serde_json::from_str(val).unwrap_or_default()
}

// ---- Internal Helpers ----

/// Extract JSON bytes from a DB row value, with a default.
fn get_json_bytes(row: &DbRow, name: &str, default: &str) -> Vec<u8> {
    match row.get(name) {
        Some(Value::String(s)) => {
            if s.is_empty() {
                default.as_bytes().to_vec()
            } else {
                s.as_bytes().to_vec()
            }
        }
        Some(v) if !v.is_null() => serde_json::to_vec(v).unwrap_or_else(|_| default.as_bytes().to_vec()),
        _ => default.as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(pairs: &[(&str, Value)]) -> DbRow {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    // ---- quick_set_str ----

    #[test]
    fn test_quick_set_str_from_string() {
        let row = make_row(&[("name", Value::String("hello".into()))]);
        let mut s = String::new();
        quick_set_str(&mut s, &row, "name");
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_quick_set_str_from_number() {
        let row = make_row(&[("id", serde_json::json!(42))]);
        let mut s = String::new();
        quick_set_str(&mut s, &row, "id");
        assert_eq!(s, "42");
    }

    #[test]
    fn test_quick_set_str_missing() {
        let row = make_row(&[]);
        let mut s = "default".to_string();
        quick_set_str(&mut s, &row, "missing");
        assert_eq!(s, "default");
    }

    // ---- quick_set_int ----

    #[test]
    fn test_quick_set_int() {
        let row = make_row(&[("count", serde_json::json!(42))]);
        let mut v = 0;
        quick_set_int(&mut v, &row, "count");
        assert_eq!(v, 42);
    }

    // ---- quick_set_int64 ----

    #[test]
    fn test_quick_set_int64() {
        let row = make_row(&[("ts", serde_json::json!(1700000000i64))]);
        let mut v = 0i64;
        quick_set_int64(&mut v, &row, "ts");
        assert_eq!(v, 1700000000);
    }

    // ---- quick_set_nullable_int64 ----

    #[test]
    fn test_quick_set_nullable_int64_some() {
        let row = make_row(&[("val", serde_json::json!(100))]);
        let mut v: Option<i64> = None;
        quick_set_nullable_int64(&mut v, &row, "val");
        assert_eq!(v, Some(100));
    }

    #[test]
    fn test_quick_set_nullable_int64_null() {
        let row = make_row(&[("val", Value::Null)]);
        let mut v: Option<i64> = Some(42);
        quick_set_nullable_int64(&mut v, &row, "val");
        assert_eq!(v, None);
    }

    // ---- quick_set_bool ----

    #[test]
    fn test_quick_set_bool_from_int() {
        let row = make_row(&[("active", serde_json::json!(1))]);
        let mut v = false;
        quick_set_bool(&mut v, &row, "active");
        assert!(v);
    }

    #[test]
    fn test_quick_set_bool_from_zero() {
        let row = make_row(&[("active", serde_json::json!(0))]);
        let mut v = true;
        quick_set_bool(&mut v, &row, "active");
        assert!(!v);
    }

    #[test]
    fn test_quick_set_bool_from_bool() {
        let row = make_row(&[("flag", Value::Bool(true))]);
        let mut v = false;
        quick_set_bool(&mut v, &row, "flag");
        assert!(v);
    }

    // ---- quick_set_json ----

    #[test]
    fn test_quick_set_json() {
        let row = make_row(&[("meta", Value::String(r#"{"key":"val"}"#.into()))]);
        let mut m: HashMap<String, String> = HashMap::new();
        quick_set_json(&mut m, &row, "meta");
        assert_eq!(m.get("key").unwrap(), "val");
    }

    #[test]
    fn test_quick_set_json_empty_defaults() {
        let row = make_row(&[("meta", Value::String("".into()))]);
        let mut m: HashMap<String, Value> = HashMap::new();
        quick_set_json(&mut m, &row, "meta");
        assert!(m.is_empty()); // "{}" deserializes to empty map
    }

    // ---- quick_set_json_arr ----

    #[test]
    fn test_quick_set_json_arr() {
        let row = make_row(&[("tags", Value::String(r#"["a","b","c"]"#.into()))]);
        let mut arr: Vec<String> = Vec::new();
        quick_set_json_arr(&mut arr, &row, "tags");
        assert_eq!(arr, vec!["a", "b", "c"]);
    }

    // ---- quick_json ----

    #[test]
    fn test_quick_json_some() {
        let m: HashMap<String, i32> = [("a".into(), 1)].into();
        let json = quick_json(Some(&m));
        assert!(json.contains("\"a\":1"));
    }

    #[test]
    fn test_quick_json_none() {
        let json = quick_json::<HashMap<String, i32>>(None);
        assert_eq!(json, "{}");
    }

    // ---- quick_nullable_json ----

    #[test]
    fn test_quick_nullable_json_some() {
        let val = 42;
        let json = quick_nullable_json(Some(&val));
        assert_eq!(json, "42");
    }

    #[test]
    fn test_quick_nullable_json_none() {
        let json = quick_nullable_json::<i32>(None);
        assert_eq!(json, "null");
    }

    // ---- quick_json_arr ----

    #[test]
    fn test_quick_json_arr_some() {
        let arr = vec![1, 2, 3];
        let json = quick_json_arr(Some(&arr));
        assert_eq!(json, "[1,2,3]");
    }

    #[test]
    fn test_quick_json_arr_none() {
        let json = quick_json_arr::<Vec<i32>>(None);
        assert_eq!(json, "[]");
    }

    // ---- parse_json_map ----

    #[test]
    fn test_parse_json_map() {
        let m = parse_json_map(r#"{"key":"value","num":42}"#, false).unwrap();
        assert_eq!(m.get("key").unwrap().as_str().unwrap(), "value");
        assert_eq!(m.get("num").unwrap().as_i64().unwrap(), 42);
    }

    #[test]
    fn test_parse_json_map_empty() {
        let m = parse_json_map("", true).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn test_parse_json_map_empty_no_force() {
        assert!(parse_json_map("", false).is_none());
    }

    #[test]
    fn test_parse_json_map_invalid() {
        assert!(parse_json_map("not json", false).is_none());
    }

    #[test]
    fn test_parse_json_map_invalid_force() {
        let m = parse_json_map("not json", true).unwrap();
        assert!(m.is_empty());
    }

    // ---- parse_json_arr ----

    #[test]
    fn test_parse_json_arr() {
        let arr: Vec<String> = parse_json_arr(r#"["a","b"]"#);
        assert_eq!(arr, vec!["a", "b"]);
    }

    #[test]
    fn test_parse_json_arr_empty() {
        let arr: Vec<String> = parse_json_arr("");
        assert!(arr.is_empty());
    }
}
