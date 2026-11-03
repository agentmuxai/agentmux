// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading `settings.json` one key at a time: a value that is the wrong type or outside its range
//! uses that key's default and is reported, and every other key applies
//! (docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md §5.5).
//!
//! Before, one bad value failed the whole file: a reload kept the old settings and a first load
//! ran on the defaults. Only a file that is not one JSON object at all is still unreadable.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use serde_json::Value;

use super::loader::jsonc_to_json;
use super::types::{ConfigError, SettingsType};

/// What `settings.json` gave.
pub struct SettingsRead {
    /// The settings that read. A key with a bad value is left out, so it has its default.
    pub settings: SettingsType,
    /// What is wrong with the file, each with its line when it is known.
    pub errors: Vec<ConfigError>,
    /// False when the file is not one JSON object (or cannot be read): `settings` is then the
    /// defaults, and the caller keeps the settings it has. True with `errors` means some keys were
    /// left out and every other key applied.
    pub readable: bool,
}

impl SettingsRead {
    fn unreadable(path: &Path, err: String, line: Option<u32>, column: Option<u32>) -> Self {
        SettingsRead {
            settings: SettingsType::default(),
            errors: vec![ConfigError { file: file_name(path), err, line, column }],
            readable: false,
        }
    }
}

fn file_name(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// What `schema/settings.json` says a key may hold, beyond what `SettingsType` can read.
#[derive(Default)]
struct KeyRules {
    types: Vec<String>,
    allowed: Vec<Value>,
    minimum: Option<f64>,
    maximum: Option<f64>,
}

fn schema_rules() -> &'static HashMap<String, KeyRules> {
    static RULES: OnceLock<HashMap<String, KeyRules>> = OnceLock::new();
    RULES.get_or_init(|| {
        const SCHEMA: &str = include_str!("../../../../../schema/settings.json");
        let schema: Value = serde_json::from_str(SCHEMA).unwrap_or_default();
        let Some(props) = schema.pointer("/$defs/SettingsType/properties").and_then(Value::as_object) else {
            return HashMap::new();
        };
        props
            .iter()
            .map(|(key, def)| {
                let types = match def.get("type") {
                    Some(Value::String(t)) => vec![t.clone()],
                    Some(Value::Array(list)) => list.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect(),
                    _ => Vec::new(),
                };
                let rules = KeyRules {
                    types,
                    allowed: def.get("enum").and_then(Value::as_array).cloned().unwrap_or_default(),
                    minimum: def.get("minimum").and_then(Value::as_f64),
                    maximum: def.get("maximum").and_then(Value::as_f64),
                };
                (key.clone(), rules)
            })
            .collect()
    })
}

fn describe(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 40 {
        format!("{}…", text.chars().take(40).collect::<String>())
    } else {
        text
    }
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    }
}

/// Why `value` is not acceptable for `key`, or `None`.
fn problem_with(key: &str, value: &Value) -> Option<String> {
    // What the typed settings can read. A `null` is fine where the setting can be unset.
    let typed = match serde_json::from_value::<SettingsType>(serde_json::json!({ key: value })) {
        Ok(typed) => typed,
        Err(e) => return Some(format!("{key}: {e}")),
    };
    if value.is_null() {
        return None;
    }
    let rules = schema_rules().get(key)?;
    // A key the typed settings have no field for is read as it is written, so the schema's type is
    // the only check on it. For a typed field the type is already checked, and its spelling in the
    // schema may differ from what serde reads.
    if typed.extra.contains_key(key)
        && !rules.types.is_empty()
        && !rules.types.iter().any(|t| type_matches(t, value))
    {
        return Some(format!("{key}: expected {}, got {}", rules.types.join(" or "), describe(value)));
    }
    if !rules.allowed.is_empty() && !rules.allowed.contains(value) {
        let list: Vec<String> = rules.allowed.iter().map(describe).collect();
        return Some(format!("{key}: expected one of {}, got {}", list.join(", "), describe(value)));
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = rules.minimum.filter(|min| n < *min) {
            return Some(format!("{key}: {} is below the lowest allowed, {min}", describe(value)));
        }
        if let Some(max) = rules.maximum.filter(|max| n > *max) {
            return Some(format!("{key}: {} is above the highest allowed, {max}", describe(value)));
        }
    }
    None
}

/// Read `settings.json`: JSONC, with or without a byte-order mark. A missing file is the defaults.
pub fn read_settings_file(path: &Path) -> SettingsRead {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SettingsRead { settings: SettingsType::default(), errors: Vec::new(), readable: true };
        }
        Err(e) => return SettingsRead::unreadable(path, format!("cannot read file: {e}"), None, None),
    };
    read_settings_text(path, &content)
}

pub(super) fn read_settings_text(path: &Path, content: &str) -> SettingsRead {
    let json = jsonc_to_json(content);
    let mut map = match serde_json::from_str::<Value>(&json) {
        Ok(Value::Object(map)) => map,
        Ok(_) => {
            return SettingsRead::unreadable(path, "the file must be one JSON object".to_string(), None, None);
        }
        Err(e) => {
            return SettingsRead::unreadable(
                path,
                format!("JSON parse error: {e}"),
                Some(e.line() as u32),
                Some(e.column() as u32),
            );
        }
    };

    let lines = agentmux_common::settings_file::key_lines(content).unwrap_or_default();
    let mut errors = Vec::new();
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        if let Some(err) = problem_with(&key, &map[&key]) {
            map.remove(&key);
            errors.push(ConfigError {
                file: file_name(path),
                err: format!("{err}; using its default"),
                line: lines.get(&key).copied(),
                column: None,
            });
        }
    }

    match serde_json::from_value::<SettingsType>(Value::Object(map)) {
        Ok(settings) => SettingsRead { settings, errors, readable: true },
        Err(e) => SettingsRead::unreadable(path, format!("JSON parse error: {e}"), None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> SettingsRead {
        read_settings_text(Path::new("settings.json"), text)
    }

    #[test]
    fn one_bad_key_among_good_ones_leaves_the_rest_applied_and_one_error() {
        let text = r#"{
            "term:fontsize": 18,
            "term:theme": "dracula",
            "term:scrollback": 5000,
            "term:copyonselect": true,
            "term:disablewebgl": true,
            "app:defaultnewblock": "term",
            "term:durable": "yes",
            "term:fontfamily": "Fira Code",
            "term:transparency": 0.5
        }"#;
        let read = read(text);
        assert!(read.readable);
        assert_eq!(read.errors.len(), 1, "{:?}", read.errors);
        assert!(read.errors[0].err.starts_with("term:durable:"), "{}", read.errors[0].err);
        assert_eq!(read.settings.term_durable, None);
        assert_eq!(read.errors[0].line, Some(8));
        assert_eq!(read.settings.term_font_size, 18.0);
        assert_eq!(read.settings.term_theme, "dracula");
        assert_eq!(read.settings.term_scrollback, Some(5000));
        assert_eq!(read.settings.term_font_family, "Fira Code");
    }

    #[test]
    fn a_wrong_type_uses_the_default_and_names_the_key_and_line() {
        let read = read("{\n  // my size\n  \"term:fontsize\": \"big\",\n  \"term:theme\": \"nord\"\n}");
        assert!(read.readable);
        assert_eq!(read.errors.len(), 1);
        assert!(read.errors[0].err.starts_with("term:fontsize:"), "{}", read.errors[0].err);
        assert!(read.errors[0].err.ends_with("using its default"));
        assert_eq!(read.errors[0].line, Some(3));
        assert_eq!(read.settings.term_font_size, SettingsType::default().term_font_size);
        assert_eq!(read.settings.term_theme, "nord");
    }

    #[test]
    fn a_value_outside_the_schemas_range_uses_the_default() {
        // term:scrollsensitivity is 0.1 to 10 in the schema.
        let low = read(r#"{ "term:scrollsensitivity": 0 }"#);
        assert_eq!(low.errors.len(), 1);
        assert!(low.errors[0].err.contains("below the lowest allowed"), "{}", low.errors[0].err);
        let high = read(r#"{ "term:scrollsensitivity": 11 }"#);
        assert!(high.errors[0].err.contains("above the highest allowed"), "{}", high.errors[0].err);
        let fine = read(r#"{ "term:scrollsensitivity": 2.5 }"#);
        assert!(fine.errors.is_empty(), "{:?}", fine.errors);
    }

    #[test]
    fn a_value_outside_the_schemas_list_uses_the_default() {
        let wobbly = read(r#"{ "animation:preset": "wobbly" }"#);
        assert_eq!(wobbly.errors.len(), 1);
        assert!(wobbly.errors[0].err.contains("expected one of"), "{}", wobbly.errors[0].err);
        assert!(read(r#"{ "animation:preset": "quick" }"#).errors.is_empty());
    }

    #[test]
    fn a_null_is_fine_where_a_setting_can_be_unset_and_reported_where_it_cannot() {
        // term:scrollback can be unset; term:theme is a plain string.
        let read = read(r#"{ "term:scrollback": null, "term:theme": null, "term:fontfamily": "Fira Code" }"#);
        assert!(read.readable, "a null never makes the file unreadable");
        assert_eq!(read.errors.len(), 1, "{:?}", read.errors);
        assert!(read.errors[0].err.starts_with("term:theme:"), "{}", read.errors[0].err);
        assert_eq!(read.settings.term_font_family, "Fira Code");
    }

    #[test]
    fn an_unknown_key_is_kept_and_not_an_error() {
        let read = read(r#"{ "my:own:note": [1, 2], "term:theme": "nord" }"#);
        assert!(read.readable);
        assert!(read.errors.is_empty());
        assert!(read.settings.extra.contains_key("my:own:note"));
    }

    #[test]
    fn a_file_that_is_not_json_is_unreadable_with_its_line_and_column() {
        let read = read("{\n  \"term:fontsize\": 14\n  \"term:theme\": \"nord\"\n}");
        assert!(!read.readable);
        assert_eq!(read.errors.len(), 1);
        assert_eq!(read.errors[0].line, Some(3));
        assert!(read.errors[0].column.is_some());
    }

    #[test]
    fn a_root_that_is_not_an_object_is_unreadable() {
        for text in ["[1, 2]", "42", "\"text\""] {
            let read = read(text);
            assert!(!read.readable, "{text}");
            assert_eq!(read.errors[0].err, "the file must be one JSON object");
        }
    }

    #[test]
    fn a_byte_order_mark_comments_and_trailing_commas_read_as_before() {
        let read = read("\u{feff}{\n  // size\n  \"term:fontsize\": \"big\", /* bad */\n  \"term:theme\": \"nord\",\n}");
        assert!(read.readable);
        assert_eq!(read.errors.len(), 1);
        assert_eq!(read.errors[0].line, Some(3));
        assert_eq!(read.settings.term_theme, "nord");
    }

    #[test]
    fn a_missing_file_is_the_defaults() {
        let read = read_settings_file(Path::new("/no/such/dir/settings.json"));
        assert!(read.readable);
        assert!(read.errors.is_empty());
    }

    #[test]
    fn the_template_reads_without_a_single_error() {
        let read = read(super::super::SETTINGS_TEMPLATE);
        assert!(read.readable);
        assert!(read.errors.is_empty(), "{:?}", read.errors);
    }
}
