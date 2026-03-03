// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Environment variable utilities: null-delimited encoding/decoding and validation.
//! Port of Go's pkg/util/envutil/envutil.go.

#![allow(dead_code)]
//!
//! Environment strings use null-byte (`\0`) as delimiter between `KEY=VALUE` entries.

use std::collections::HashMap;

/// Maximum environment string size (1MB).
pub const MAX_ENV_SIZE: usize = 1024 * 1024;

/// Parse a null-delimited environment string into a map.
///
/// Each entry has format `KEY=VALUE`, separated by `\0`.
/// Entries without `=` are skipped.
pub fn env_to_map(env_str: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if env_str.is_empty() {
        return map;
    }
    for entry in env_str.split('\0') {
        if entry.is_empty() {
            continue;
        }
        if let Some(eq_pos) = entry.find('=') {
            let key = &entry[..eq_pos];
            let value = &entry[eq_pos + 1..];
            if !key.is_empty() {
                map.insert(key.to_string(), value.to_string());
            }
        }
    }
    map
}

/// Convert a map to a null-delimited environment string.
///
/// Each entry is `KEY=VALUE`, separated by `\0`.
/// Keys are sorted for deterministic output.
pub fn map_to_env(map: &HashMap<String, String>) -> String {
    if map.is_empty() {
        return String::new();
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let entries: Vec<String> = keys
        .into_iter()
        .map(|k| format!("{}={}", k, map[k]))
        .collect();
    entries.join("\0")
}

/// Validate an environment variable name.
///
/// Rules: must not be empty, must not contain `=` or `\0`.
pub fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('=') && !name.contains('\0')
}

/// Get a single environment variable from a null-delimited string.
pub fn get_env(env_str: &str, key: &str) -> Option<String> {
    let map = env_to_map(env_str);
    map.get(key).cloned()
}

/// Set an environment variable in a null-delimited string, returning the updated string.
///
/// Returns `Err` if the key is invalid (empty, contains `=` or `\0`).
pub fn set_env(env_str: &str, key: &str, value: &str) -> Result<String, String> {
    if !is_valid_env_name(key) {
        return Err(format!(
            "invalid environment variable name: {:?}",
            key
        ));
    }
    let mut map = env_to_map(env_str);
    map.insert(key.to_string(), value.to_string());
    Ok(map_to_env(&map))
}

/// Remove an environment variable from a null-delimited string, returning the updated string.
pub fn rm_env(env_str: &str, key: &str) -> String {
    let mut map = env_to_map(env_str);
    map.remove(key);
    map_to_env(&map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_to_map_empty() {
        let map = env_to_map("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_env_to_map_single() {
        let map = env_to_map("FOO=bar");
        assert_eq!(map.get("FOO").unwrap(), "bar");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_env_to_map_multiple() {
        let map = env_to_map("FOO=bar\0BAZ=qux\0HOME=/home/user");
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("FOO").unwrap(), "bar");
        assert_eq!(map.get("BAZ").unwrap(), "qux");
        assert_eq!(map.get("HOME").unwrap(), "/home/user");
    }

    #[test]
    fn test_env_to_map_value_with_equals() {
        // Values can contain '='
        let map = env_to_map("CMD=echo foo=bar");
        assert_eq!(map.get("CMD").unwrap(), "echo foo=bar");
    }

    #[test]
    fn test_env_to_map_empty_value() {
        let map = env_to_map("EMPTY=");
        assert_eq!(map.get("EMPTY").unwrap(), "");
    }

    #[test]
    fn test_env_to_map_skips_no_equals() {
        let map = env_to_map("VALID=yes\0noequalssign\0ALSO_VALID=true");
        assert_eq!(map.len(), 2);
        assert!(map.get("VALID").is_some());
        assert!(map.get("ALSO_VALID").is_some());
    }

    #[test]
    fn test_env_to_map_trailing_null() {
        let map = env_to_map("FOO=bar\0");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn test_map_to_env_empty() {
        let map = HashMap::new();
        assert_eq!(map_to_env(&map), "");
    }

    #[test]
    fn test_map_to_env_single() {
        let mut map = HashMap::new();
        map.insert("FOO".to_string(), "bar".to_string());
        assert_eq!(map_to_env(&map), "FOO=bar");
    }

    #[test]
    fn test_map_to_env_sorted() {
        let mut map = HashMap::new();
        map.insert("ZZZ".to_string(), "last".to_string());
        map.insert("AAA".to_string(), "first".to_string());
        map.insert("MMM".to_string(), "middle".to_string());
        let result = map_to_env(&map);
        assert_eq!(result, "AAA=first\0MMM=middle\0ZZZ=last");
    }

    #[test]
    fn test_roundtrip() {
        let mut map = HashMap::new();
        map.insert("PATH".to_string(), "/usr/bin:/usr/local/bin".to_string());
        map.insert("HOME".to_string(), "/home/user".to_string());
        map.insert("EMPTY".to_string(), String::new());
        let env = map_to_env(&map);
        let parsed = env_to_map(&env);
        assert_eq!(map, parsed);
    }

    #[test]
    fn test_is_valid_env_name() {
        assert!(is_valid_env_name("FOO"));
        assert!(is_valid_env_name("PATH"));
        assert!(is_valid_env_name("MY_VAR_123"));
        assert!(!is_valid_env_name(""));
        assert!(!is_valid_env_name("FOO=BAR"));
        assert!(!is_valid_env_name("FOO\0BAR"));
    }

    #[test]
    fn test_get_env() {
        let env = "FOO=bar\0BAZ=qux";
        assert_eq!(get_env(env, "FOO"), Some("bar".to_string()));
        assert_eq!(get_env(env, "BAZ"), Some("qux".to_string()));
        assert_eq!(get_env(env, "NOPE"), None);
    }

    #[test]
    fn test_set_env_new() {
        let env = "FOO=bar";
        let result = set_env(env, "BAZ", "qux").unwrap();
        let map = env_to_map(&result);
        assert_eq!(map.get("FOO").unwrap(), "bar");
        assert_eq!(map.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn test_set_env_overwrite() {
        let env = "FOO=bar";
        let result = set_env(env, "FOO", "baz").unwrap();
        let map = env_to_map(&result);
        assert_eq!(map.get("FOO").unwrap(), "baz");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_set_env_invalid_name() {
        assert!(set_env("", "FOO=BAR", "val").is_err());
        assert!(set_env("", "", "val").is_err());
    }

    #[test]
    fn test_rm_env() {
        let env = "FOO=bar\0BAZ=qux";
        let result = rm_env(env, "FOO");
        let map = env_to_map(&result);
        assert!(map.get("FOO").is_none());
        assert_eq!(map.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn test_rm_env_nonexistent() {
        let env = "FOO=bar";
        let result = rm_env(env, "NOPE");
        assert_eq!(result, "FOO=bar");
    }
}
