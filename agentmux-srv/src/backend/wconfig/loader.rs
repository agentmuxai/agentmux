// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Config file loading, parsing, template merging, and environment expansion.

use std::collections::HashMap;
use std::path::PathBuf;

use super::types::{ConfigError, FullConfigType, WidgetConfigType};

// ---- Default config builder ----

/// Build the initial default configuration with embedded default assets.
///
/// Loads the bundled `widgets.json` (from `config/`) at compile time
/// and populates `FullConfigType.widgets` so the frontend widget bar is populated on startup.
pub fn build_default_config() -> FullConfigType {
    let mut config = FullConfigType::default();

    // Embed widgets.json at compile time (equivalent to Go's //go:embed)
    const WIDGETS_JSON: &str =
        include_str!("../../config/widgets.json");

    match serde_json::from_str::<HashMap<String, WidgetConfigType>>(WIDGETS_JSON) {
        Ok(widgets) => {
            validate_widget_configs(&widgets);
            config.widgets = widgets;
        }
        Err(e) => {
            eprintln!("wconfig: failed to parse embedded widgets.json: {}", e);
        }
    }

    config
}

/// Soft validation for widget entries loaded from `widgets.json`: a widget
/// must be either a leaf (non-empty `blockdef.meta`) or a parent (non-empty
/// `children`), never neither, never both. Logged as a startup warning, not
/// a hard failure, matching how other config issues are handled.
fn validate_widget_configs(widgets: &HashMap<String, WidgetConfigType>) {
    for (key, widget) in widgets {
        let has_blockdef = !widget.block_def.meta.is_empty();
        let has_children = !widget.children.is_empty();
        if has_blockdef && has_children {
            eprintln!(
                "wconfig: widget \"{}\" has both a blockdef and children — children (parent behavior) takes precedence",
                key
            );
        } else if !has_blockdef && !has_children {
            eprintln!(
                "wconfig: widget \"{}\" has neither a blockdef nor children — it can't open a pane or expand a submenu",
                key
            );
        }
    }
}

// ---- Config loading helpers ----

/// Read a JSON config file, returning default on missing/error.
pub fn read_config_file<T: serde::de::DeserializeOwned + Default>(
    path: &PathBuf,
) -> (T, Vec<ConfigError>) {
    let mut errors = Vec::new();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (T::default(), errors),
        Err(e) => {
            errors.push(ConfigError {
                file: path.to_string_lossy().to_string(),
                err: format!("cannot read file: {}", e),
            });
            return (T::default(), errors);
        }
    };

    // Strip // and /* */ comments so users can annotate settings.json
    let stripped = json_comments::StripComments::new(content.as_bytes());
    let mut json_bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::BufReader::new(stripped), &mut json_bytes)
        .unwrap_or_default();

    // Strip trailing commas before } or ] (common when commented-out lines follow values)
    let json_str = strip_trailing_commas(&String::from_utf8_lossy(&json_bytes));
    let clean: Result<T, _> = serde_json::from_str(&json_str);

    match clean {
        Ok(parsed) => (parsed, errors),
        Err(e) => {
            errors.push(ConfigError {
                file: path.to_string_lossy().to_string(),
                err: format!("JSON parse error: {}", e),
            });
            (T::default(), errors)
        }
    }
}

/// Read `settings.json` as a raw `serde_json::Value::Object`, stripping JSONC comments
/// and trailing commas. Returns an empty object if the file doesn't exist or can't be parsed.
pub fn read_settings_raw_jsonc(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    if !path.exists() {
        return serde_json::Map::new();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => parse_jsonc_to_map(&content),
        Err(_) => serde_json::Map::new(),
    }
}

/// Parse a JSONC string (with // comments and trailing commas) into a flat JSON map.
pub fn parse_jsonc_to_map(content: &str) -> serde_json::Map<String, serde_json::Value> {
    let stripped_comments = json_comments::StripComments::new(content.as_bytes());
    let mut json_bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::BufReader::new(stripped_comments), &mut json_bytes)
        .unwrap_or_default();
    let json_str = strip_trailing_commas(&String::from_utf8_lossy(&json_bytes));
    match serde_json::from_str::<serde_json::Value>(&json_str) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// Merge user settings into a JSONC template string.
///
/// For each user key:
/// - If the key exists as a commented-out line in the template (`// "key": ...`),
///   that line is replaced with the uncommented user value.
/// - If the key is NOT in the template, it is appended before the closing `}`.
///
/// The result is always a valid JSONC file with the full template structure intact.
pub fn merge_into_template(
    template: &str,
    user_settings: &serde_json::Map<String, serde_json::Value>,
) -> String {
    if user_settings.is_empty() {
        return template.to_string();
    }

    let mut remaining: std::collections::HashMap<&str, &serde_json::Value> =
        user_settings.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut lines: Vec<String> = Vec::new();

    for line in template.lines() {
        if let Some(key) = extract_commented_setting_key(line) {
            if let Some(value) = remaining.remove(key) {
                // Preserve the original indentation
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                let val_str = serde_json::to_string(value).unwrap_or_default();
                lines.push(format!("{}\"{}\": {},", indent, key, val_str));
                continue;
            }
        }
        lines.push(line.to_string());
    }

    // Append any remaining user settings not found in the template
    if !remaining.is_empty() {
        // Find the last `}` and insert before it
        if let Some(brace_pos) = lines.iter().rposition(|l| l.trim() == "}") {
            let mut extra: Vec<String> = Vec::new();
            extra.push(String::new());
            extra.push("    // -- User Overrides --".to_string());
            let mut sorted_keys: Vec<&&str> = remaining.keys().collect();
            sorted_keys.sort();
            for key in sorted_keys {
                let value = remaining[*key];
                let val_str = serde_json::to_string(value).unwrap_or_default();
                extra.push(format!("    \"{}\": {},", key, val_str));
            }
            for (i, line) in extra.into_iter().enumerate() {
                lines.insert(brace_pos + i, line);
            }
        }
    }

    let mut result = lines.join("\n");
    // Ensure file ends with newline
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Extract the settings key from a commented-out template line.
/// Matches lines like: `    // "some:key":   value,`
/// Returns `Some("some:key")` or `None`.
fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("//")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

pub(super) fn strip_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut last_comma_pos: Option<usize> = None;

    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if ch == '\\' {
                if let Some(&next) = chars.peek() {
                    result.push(next);
                    chars.next();
                }
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => {
                    in_string = true;
                    last_comma_pos = None;
                    result.push(ch);
                }
                ',' => {
                    last_comma_pos = Some(result.len());
                    result.push(ch);
                }
                '}' | ']' => {
                    if let Some(pos) = last_comma_pos {
                        result.replace_range(pos..pos + 1, " ");
                    }
                    last_comma_pos = None;
                    result.push(ch);
                }
                c if c.is_whitespace() => {
                    result.push(ch);
                }
                _ => {
                    last_comma_pos = None;
                    result.push(ch);
                }
            }
        }
    }
    result
}

/// Replace `$ENV:VAR_NAME` and `$ENV:VAR_NAME:fallback` in a string.
#[allow(dead_code)]
pub fn expand_env_vars(s: &str) -> String {
    let mut result = s.to_string();
    let mut start = 0;

    while let Some(idx) = result[start..].find("$ENV:") {
        let abs_idx = start + idx;
        let rest = &result[abs_idx + 5..];

        // Find the end of the variable reference
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':')
            .unwrap_or(rest.len());

        let var_spec = &rest[..end];

        // Split on first colon for fallback
        let (var_name, fallback) = if let Some(colon_idx) = var_spec.find(':') {
            (&var_spec[..colon_idx], Some(&var_spec[colon_idx + 1..]))
        } else {
            (var_spec, None)
        };

        let value = std::env::var(var_name).unwrap_or_else(|_| {
            fallback.unwrap_or("").to_string()
        });

        let full_pattern = format!("$ENV:{}", var_spec);
        result = result.replacen(&full_pattern, &value, 1);
        start = abs_idx + value.len();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_widget() -> WidgetConfigType {
        WidgetConfigType {
            block_def: super::super::types::BlockDef {
                meta: std::collections::HashMap::from([("view".to_string(), serde_json::json!("browser"))]),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn parent_widget(children: &[&str]) -> WidgetConfigType {
        WidgetConfigType {
            children: children.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    // validate_widget_configs only logs (eprintln) — these tests just confirm
    // it doesn't panic for every combination, since there's no return value
    // to assert on. The real coverage of "parent xor leaf" is enforced by the
    // embedded widgets.json actually loading without triggering a warning,
    // which build_default_config's own tests below check indirectly by
    // asserting on the real parsed shape.
    #[test]
    fn test_validate_widget_configs_does_not_panic() {
        let widgets: HashMap<String, WidgetConfigType> = HashMap::from([
            ("defwidget@leaf".to_string(), leaf_widget()),
            ("defwidget@parent".to_string(), parent_widget(&["leaf"])),
            ("defwidget@neither".to_string(), WidgetConfigType::default()),
            (
                "defwidget@both".to_string(),
                WidgetConfigType {
                    children: vec!["leaf".to_string()],
                    ..leaf_widget()
                },
            ),
        ]);
        validate_widget_configs(&widgets);
    }

    #[test]
    fn test_build_default_config_loads_messengers_group() {
        let config = build_default_config();
        let messengers = config
            .widgets
            .get("defwidget@messengers")
            .expect("defwidget@messengers should be in the embedded widgets.json");
        assert_eq!(
            messengers.children,
            vec!["discord", "slack", "telegram", "whatsapp", "teams"]
        );
        assert!(
            messengers.block_def.meta.is_empty(),
            "a parent widget shouldn't carry a blockdef"
        );

        for short_name in ["discord", "slack", "telegram", "whatsapp", "teams"] {
            let key = format!("defwidget@{short_name}");
            let child = config
                .widgets
                .get(&key)
                .unwrap_or_else(|| panic!("{key} should still be in widgets.json"));
            assert!(child.display_hidden, "{key} should be display:hidden now that it's grouped");
            assert!(child.children.is_empty(), "{key} is a leaf, not itself a parent");
            assert!(!child.block_def.meta.is_empty(), "{key} should still carry its own blockdef");
        }
    }
}
