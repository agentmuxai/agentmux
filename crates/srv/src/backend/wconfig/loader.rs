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
        Ok(mut widgets) => {
            validate_widget_configs(&widgets);
            widgets.values_mut().for_each(WidgetConfigType::resolve_section);
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
pub(crate) fn validate_widget_configs(widgets: &HashMap<String, WidgetConfigType>) {
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

/// A config file's text as `T`: JSONC (comments and trailing commas allowed), with or without a
/// byte-order mark. What `read_config_file` parses, so a caller can check bytes it holds the same way.
pub fn parse_config_text<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, serde_json::Error> {
    // An editor may have saved the file with a byte-order mark.
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    // Strip // and /* */ comments so users can annotate settings.json
    let stripped = json_comments::StripComments::new(content.as_bytes());
    let mut json_bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::BufReader::new(stripped), &mut json_bytes)
        .unwrap_or_default();

    // Strip trailing commas before } or ] (common when commented-out lines follow values)
    let json_str = strip_trailing_commas(&String::from_utf8_lossy(&json_bytes));
    serde_json::from_str(&json_str)
}

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

    match parse_config_text::<T>(&content) {
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
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
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

pub use agentmux_common::settings_file::merge_into_template;

pub(super) use agentmux_common::settings_jsonc::strip_trailing_commas;

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
    /// Pane Tab contract Phase 6: a third-party widget's `module` must survive
    /// parsing (serde drops unknown fields) and reach the frontend unchanged.
    #[test]
    fn test_widget_module_survives_parse_and_serialize() {
        let raw = r#"{"label":"Hello","module":"hello/index.js","blockdef":{"meta":{"view":"ext:hello"}}}"#;
        let widget: WidgetConfigType = serde_json::from_str(raw).expect("parses");
        assert_eq!(widget.module, "hello/index.js");
        let back = serde_json::to_value(&widget).expect("serializes");
        assert_eq!(back["module"], "hello/index.js");
        // A built-in widget without one serializes without the key.
        let plain = serde_json::to_value(leaf_widget()).expect("serializes");
        assert!(plain.get("module").is_none());
    }

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
    fn a_section_entry_opens_its_pane_on_that_section() {
        let config = build_default_config();
        let kanban = &config.widgets["defwidget@kanban"].block_def.meta;
        // The Kanban is a pane of its own: no section, and nothing left of the Swarm's.
        assert_eq!(kanban.get("view").and_then(|v| v.as_str()), Some("kanban"));
        assert!(kanban.get("swarm:section").is_none());
        assert!(kanban.get("kanban:section").is_none());
        assert!(config.widgets["defwidget@kanban"].section.is_empty());

        // The Vault entry is whatever widgets.json holds; whichever pane that is, it carries no
        // section of Warden's once resolved.
        let vault = &config.widgets["defwidget@vault"].block_def.meta;
        assert!(vault.get("warden:section").is_none());

        // A section of a section: the Vault pane on its Cards. Warden is no longer in the way.
        let mut cards: WidgetConfigType = serde_json::from_str(
            r#"{"label":"Cards","section":"vault/cards","blockdef":{"meta":{"view":"warden"}}}"#,
        )
        .unwrap();
        cards.resolve_section();
        assert_eq!(cards.block_def.meta.get("view").and_then(|v| v.as_str()), Some("vault"));
        assert_eq!(cards.block_def.meta.get("vault:section").and_then(|v| v.as_str()), Some("cards"));
        assert!(cards.block_def.meta.get("warden:section").is_none());
        assert!(cards.section.is_empty());

        // An entry that still names the Kanban as a section of Warden or of the Swarm (a user's own, or a
        // pin from before the moves) opens the Kanban pane.
        for was in ["warden", "swarm"] {
            let mut old: WidgetConfigType = serde_json::from_str(&format!(
                r#"{{"label":"Kanban","section":"kanban","blockdef":{{"meta":{{"view":"{was}"}}}}}}"#
            ))
            .unwrap();
            old.resolve_section();
            assert_eq!(old.block_def.meta.get("view").and_then(|v| v.as_str()), Some("kanban"), "from {was}");
            assert!(old.block_def.meta.get("swarm:section").is_none());
            assert!(old.block_def.meta.get("warden:section").is_none());
            assert!(old.section.is_empty());
        }

        // The Vault, History and Audit are panes of their own; Host is the Swarm.
        for (section, view) in [("vault", "vault"), ("history", "history"), ("audit", "audit"), ("host", "swarm")] {
            let mut old: WidgetConfigType = serde_json::from_str(&format!(
                r#"{{"label":"X","section":"{section}","blockdef":{{"meta":{{"view":"warden","warden:section":"{section}"}}}}}}"#
            ))
            .unwrap();
            old.resolve_section();
            assert_eq!(old.block_def.meta.get("view").and_then(|v| v.as_str()), Some(view), "from {section}");
            assert!(old.block_def.meta.get("warden:section").is_none(), "from {section}");
            assert!(old.block_def.meta.get(&format!("{view}:section")).is_none(), "from {section}");
            assert!(old.section.is_empty());
        }

        // A section with no pane any more is cleared and the entry stays on Warden.
        for retired in ["lan", "internet", "ambient"] {
            let mut old: WidgetConfigType = serde_json::from_str(&format!(
                r#"{{"label":"X","section":"{retired}","blockdef":{{"meta":{{"view":"warden"}}}}}}"#
            ))
            .unwrap();
            old.resolve_section();
            assert_eq!(old.block_def.meta.get("view").and_then(|v| v.as_str()), Some("warden"), "from {retired}");
            assert!(old.block_def.meta.get("warden:section").is_none(), "from {retired}");
            assert!(old.section.is_empty());
        }

        // A section Warden still has is written as before.
        let mut jobs: WidgetConfigType =
            serde_json::from_str(r#"{"label":"Jobs","section":"jobs","blockdef":{"meta":{"view":"warden"}}}"#).unwrap();
        jobs.resolve_section();
        assert_eq!(jobs.block_def.meta.get("view").and_then(|v| v.as_str()), Some("warden"));
        assert_eq!(jobs.block_def.meta.get("warden:section").and_then(|v| v.as_str()), Some("jobs"));

        // No view: nothing to key it by, so the meta is left alone.
        let mut orphan: WidgetConfigType = serde_json::from_str(r#"{"section":"cards"}"#).unwrap();
        orphan.resolve_section();
        assert!(orphan.block_def.meta.is_empty());
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
            assert!(child.display_grouponly, "{key} should be display:grouponly now that it's grouped");
            assert!(child.children.is_empty(), "{key} is a leaf, not itself a parent");
            assert!(!child.block_def.meta.is_empty(), "{key} should still carry its own blockdef");
        }
    }

    #[test]
    fn a_widget_saved_with_the_old_display_hidden_name_still_reads_as_group_only() {
        let old: WidgetConfigType = serde_json::from_str(r#"{ "display:hidden": true }"#).unwrap();
        assert!(old.display_grouponly);
        let new: WidgetConfigType = serde_json::from_str(r#"{ "display:grouponly": true }"#).unwrap();
        assert!(new.display_grouponly);
        // Written back under the new name.
        assert_eq!(serde_json::to_value(&old).unwrap()["display:grouponly"], true);
    }
}
