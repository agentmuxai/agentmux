// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Configuration system: settings, themes, widgets, bookmarks, connections.
//! Port of Go's pkg/wconfig/.
//!
//! Provides the full configuration type hierarchy and a thread-safe
//! in-memory config holder (`ConfigState`).
//!
//! Note `ConfigState` does NOT watch the filesystem, despite the name this
//! module's type carried until 2026-09-06. Filesystem watching lives in
//! `backend::config_watcher_fs`, on top of the shared `fs_watch::pool`. Two
//! similarly-named modules where only one watched cost real debugging time;
//! see REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md §7.

mod loader;
pub mod redact;
pub mod types;
mod state;

// Re-export all public APIs so callers can continue using `wconfig::Type`.
pub use loader::*;
pub use redact::redact_full_config_for_renderer;
pub use types::*;
pub use state::*;

// ---- Config file constants ----

pub const SETTINGS_FILE: &str = "settings.json";
pub const SETTINGS_TEMPLATE: &str = include_str!("../../../../settings-template.jsonc");
#[allow(dead_code)]
pub const CONNECTIONS_FILE: &str = "connections.json";
#[allow(dead_code)]
pub const PROFILES_FILE: &str = "profiles.json";

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -- SettingsType serde --

    #[test]
    fn test_settings_default_empty() {
        let s = SettingsType::default();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_settings_terminal_fields() {
        let s = SettingsType {
            term_font_size: 14.0,
            term_theme: "dracula".to_string(),
            term_scrollback: Some(10000),
            term_copy_on_select: Some(true),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"term:fontsize\":14.0"));
        assert!(json.contains("\"term:theme\":\"dracula\""));
        assert!(json.contains("\"term:scrollback\":10000"));
        assert!(json.contains("\"term:copyonselect\":true"));
    }

    #[test]
    fn test_settings_window_fields() {
        let s = SettingsType {
            window_transparent: true,
            window_opacity: Some(0.9),
            window_zoom: Some(1.5),
            window_dimensions: "1920x1080".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"window:transparent\":true"));
        assert!(json.contains("\"window:opacity\":0.9"));
        assert!(json.contains("\"window:zoom\":1.5"));
        assert!(json.contains("\"window:dimensions\":\"1920x1080\""));
    }

    #[test]
    fn test_settings_from_json() {
        let json_str = r#"{
            "term:fontsize": 13,
            "term:theme": "solarized-dark",
            "term:scrollback": 5000,
            "window:transparent": true,
            "window:opacity": 0.85,
            "telemetry:enabled": true
        }"#;
        let s: SettingsType = serde_json::from_str(json_str).unwrap();
        assert_eq!(s.term_font_size, 13.0);
        assert_eq!(s.term_theme, "solarized-dark");
        assert_eq!(s.term_scrollback, Some(5000));
        assert!(s.window_transparent);
        assert_eq!(s.window_opacity, Some(0.85));
        assert!(s.telemetry_enabled);
    }

    #[test]
    fn test_settings_roundtrip() {
        let s = SettingsType {
            term_font_size: 14.0,
            term_theme: "dracula".to_string(),
            window_opacity: Some(0.95),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let parsed: SettingsType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.term_font_size, 14.0);
        assert_eq!(parsed.term_theme, "dracula");
        assert_eq!(parsed.window_opacity, Some(0.95));
    }

    #[test]
    fn test_settings_unknown_keys_passthrough() {
        // Legacy keys (ai:*, autoupdate:*, editor:*, markdown:*) removed from
        // SettingsType should land in `extra` via the flatten catch-all so
        // existing user settings.json files don't error.
        let json_str = r#"{
            "ai:model": "claude-3-opus",
            "autoupdate:enabled": true,
            "editor:wordwrap": true,
            "term:fontsize": 14
        }"#;
        let s: SettingsType = serde_json::from_str(json_str).unwrap();
        assert_eq!(s.term_font_size, 14.0);
        assert!(s.extra.contains_key("ai:model"));
        assert!(s.extra.contains_key("autoupdate:enabled"));
        assert!(s.extra.contains_key("editor:wordwrap"));
    }

    // -- TermThemeType serde --

    #[test]
    fn test_term_theme_serde() {
        let theme = TermThemeType {
            display_name: "Dracula".to_string(),
            black: "#282a36".to_string(),
            red: "#ff5555".to_string(),
            foreground: "#f8f8f2".to_string(),
            background: "#282a36".to_string(),
            cursor: "#f8f8f2".to_string(),
            bright_red: "#ff6e6e".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&theme).unwrap();
        assert!(json.contains(r#""display:name":"Dracula""#));
        assert!(json.contains(r##""brightRed":"#ff6e6e""##));
        assert!(json.contains(r##""foreground":"#f8f8f2""##));

        let parsed: TermThemeType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.display_name, "Dracula");
        assert_eq!(parsed.bright_red, "#ff6e6e");
    }

    #[test]
    fn test_term_theme_from_go_json() {
        let go_json = r##"{
            "display:name": "Solarized Dark",
            "display:order": 1.0,
            "black": "#073642",
            "red": "#dc322f",
            "green": "#859900",
            "yellow": "#b58900",
            "blue": "#268bd2",
            "magenta": "#d33682",
            "cyan": "#2aa198",
            "white": "#eee8d5",
            "brightBlack": "#002b36",
            "brightRed": "#cb4b16",
            "brightGreen": "#586e75",
            "brightYellow": "#657b83",
            "brightBlue": "#839496",
            "brightMagenta": "#6c71c4",
            "brightCyan": "#93a1a1",
            "brightWhite": "#fdf6e3",
            "foreground": "#839496",
            "background": "#002b36",
            "cursor": "#839496",
            "selectionBackground": "#073642"
        }"##;
        let parsed: TermThemeType = serde_json::from_str(go_json).unwrap();
        assert_eq!(parsed.display_name, "Solarized Dark");
        assert_eq!(parsed.display_order, 1.0);
        assert_eq!(parsed.black, "#073642");
        assert_eq!(parsed.bright_cyan, "#93a1a1");
        assert_eq!(parsed.selection_background, "#073642");
    }

    // -- WebBookmark serde --

    #[test]
    fn test_web_bookmark_serde() {
        let bm = WebBookmark {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            icon_url: "https://example.com/favicon.ico".to_string(),
            display_order: 1.0,
            ..Default::default()
        };
        let json = serde_json::to_string(&bm).unwrap();
        assert!(json.contains("\"iconurl\":\"https://example.com/favicon.ico\""));
        assert!(json.contains("\"display:order\":1.0"));

        let parsed: WebBookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.url, "https://example.com");
    }

    // -- ConnKeywords serde --

    #[test]
    fn test_conn_keywords_serde() {
        let kw = ConnKeywords {
            ssh_user: Some("admin".to_string()),
            ssh_hostname: Some("server.example.com".to_string()),
            ssh_port: Some("2222".to_string()),
            ssh_identity_file: vec!["~/.ssh/id_rsa".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&kw).unwrap();
        assert!(json.contains("\"ssh:user\":\"admin\""));
        assert!(json.contains("\"ssh:hostname\":\"server.example.com\""));
        assert!(json.contains("\"ssh:port\":\"2222\""));
        assert!(json.contains("\"ssh:identityfile\":[\"~/.ssh/id_rsa\"]"));
    }

    #[test]
    fn test_conn_keywords_from_go_json() {
        let go_json = r#"{
            "ssh:user": "deploy",
            "ssh:hostname": "prod.example.com",
            "ssh:port": "22",
            "ssh:identityfile": ["~/.ssh/deploy_key", "~/.ssh/id_ed25519"],
            "ssh:pubkeyauthentication": true,
            "ssh:proxyjump": ["bastion.example.com"],
            "term:theme": "monokai",
            "cmd:env": {"NODE_ENV": "production"}
        }"#;
        let parsed: ConnKeywords = serde_json::from_str(go_json).unwrap();
        assert_eq!(parsed.ssh_user, Some("deploy".to_string()));
        assert_eq!(parsed.ssh_identity_file.len(), 2);
        assert_eq!(parsed.ssh_pubkey_authentication, Some(true));
        assert_eq!(parsed.ssh_proxy_jump, vec!["bastion.example.com"]);
        assert_eq!(parsed.term_theme, "monokai");
        assert_eq!(parsed.cmd_env.get("NODE_ENV").unwrap(), "production");
    }

    // -- WidgetConfigType serde --

    #[test]
    fn test_widget_config_serde() {
        let w = WidgetConfigType {
            display_order: 1.5,
            icon: "terminal".to_string(),
            label: "Shell".to_string(),
            description: "Terminal emulator".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("\"display:order\":1.5"));
        assert!(json.contains("\"label\":\"Shell\""));
    }

    // -- MimeTypeConfigType serde --

    #[test]
    fn test_mime_type_config_serde() {
        let mt = MimeTypeConfigType {
            icon: "file-code".to_string(),
            color: "#e06c75".to_string(),
        };
        let json = serde_json::to_string(&mt).unwrap();
        assert!(json.contains("\"icon\":\"file-code\""));
        assert!(json.contains(r##""color":"#e06c75""##));
    }

    // -- FullConfigType serde --

    #[test]
    fn test_full_config_default() {
        let config = FullConfigType::default();
        let json = serde_json::to_string(&config).unwrap();
        // Should have all top-level keys even if empty
        assert!(json.contains("\"settings\""));
        assert!(json.contains("\"mimetypes\""));
        assert!(json.contains("\"termthemes\""));
    }

    #[test]
    fn test_full_config_with_data() {
        let mut config = FullConfigType::default();
        config.settings.term_theme = "dracula".to_string();
        config.term_themes.insert(
            "test".to_string(),
            TermThemeType {
                display_name: "Test Theme".to_string(),
                ..Default::default()
            },
        );
        config.bookmarks.insert(
            "example".to_string(),
            WebBookmark {
                url: "https://example.com".to_string(),
                ..Default::default()
            },
        );

        let json = serde_json::to_string(&config).unwrap();
        let parsed: FullConfigType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.settings.term_theme, "dracula");
        assert_eq!(parsed.term_themes.len(), 1);
        assert_eq!(
            parsed.term_themes.get("test").unwrap().display_name,
            "Test Theme"
        );
        assert_eq!(parsed.bookmarks.len(), 1);
    }

    #[test]
    fn test_full_config_from_json() {
        let json = r##"{
            "settings": {
                "term:theme": "solarized-dark",
                "term:fontsize": 14,
                "window:transparent": true
            },
            "mimetypes": {
                "text/rust": {"icon": "rust", "color": "#dea584"}
            },
            "termthemes": {
                "dracula": {
                    "display:name": "Dracula",
                    "black": "#282a36",
                    "foreground": "#f8f8f2"
                }
            },
            "bookmarks": {
                "docs": {"url": "https://docs.rs", "title": "Rust Docs"}
            },
            "connections": {
                "prod-server": {
                    "ssh:user": "admin",
                    "ssh:hostname": "prod.example.com"
                }
            },
            "widgets": {},
            "defaultwidgets": {},
            "presets": {}
        }"##;
        let parsed: FullConfigType = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.settings.term_theme, "solarized-dark");
        assert_eq!(parsed.settings.term_font_size, 14.0);
        assert!(parsed.settings.window_transparent);
        assert_eq!(parsed.mime_types.len(), 1);
        assert_eq!(parsed.term_themes.get("dracula").unwrap().display_name, "Dracula");
        assert_eq!(parsed.bookmarks.get("docs").unwrap().title, "Rust Docs");
        assert_eq!(
            parsed.connections.get("prod-server").unwrap().ssh_user,
            Some("admin".to_string())
        );
    }

    // -- ConfigError serde --

    #[test]
    fn test_config_error_serde() {
        let err = ConfigError {
            file: "settings.json".to_string(),
            err: "unexpected token at line 5".to_string(),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""file":"settings.json""#));
        assert!(json.contains(r#""err":"unexpected token at line 5""#));
    }

    // -- WebhookConfigType serde --

    #[test]
    fn test_webhook_config_serde() {
        let wh = WebhookConfigType {
            version: "1".to_string(),
            workspace_id: "ws-123".to_string(),
            auth_token: "tok-abc".to_string(),
            cloud_endpoint: "wss://cloud.example.com".to_string(),
            enabled: true,
            terminals: vec!["term-1".to_string()],
        };
        let json = serde_json::to_string(&wh).unwrap();
        assert!(json.contains("\"workspaceId\":\"ws-123\""));
        assert!(json.contains("\"authToken\":\"tok-abc\""));
        assert!(json.contains("\"cloudEndpoint\":\"wss://cloud.example.com\""));
    }

    // -- ConfigState --

    #[test]
    fn test_config_watcher_default() {
        let watcher = ConfigState::new();
        let config = watcher.get_full_config();
        assert!(config.settings.term_theme.is_empty());
    }

    #[test]
    fn test_config_watcher_with_initial() {
        let mut config = FullConfigType::default();
        config.settings.term_theme = "dracula".to_string();
        let watcher = ConfigState::with_config(config);
        assert_eq!(watcher.get_settings().term_theme, "dracula");
    }

    #[test]
    fn test_config_watcher_set_config() {
        let watcher = ConfigState::new();
        let mut config = FullConfigType::default();
        config.settings.term_font_size = 16.0;
        watcher.set_config(config);
        assert_eq!(watcher.get_settings().term_font_size, 16.0);
    }

    #[test]
    fn test_config_watcher_update_settings() {
        let watcher = ConfigState::new();
        let settings = SettingsType {
            term_theme: "solarized-dark".to_string(),
            ..Default::default()
        };
        watcher.update_settings(settings);
        assert_eq!(watcher.get_settings().term_theme, "solarized-dark");
    }

    #[test]
    fn test_config_watcher_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let watcher = Arc::new(ConfigState::new());
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let w = watcher.clone();
                thread::spawn(move || {
                    let s = SettingsType {
                        term_theme: format!("theme-{}", i),
                        ..Default::default()
                    };
                    w.update_settings(s);
                    let _ = w.get_full_config();
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
        // Should not panic — proves thread safety
        let _ = watcher.get_settings();
    }

    // -- expand_env_vars --

    #[test]
    fn test_expand_env_vars_no_vars() {
        assert_eq!(expand_env_vars("hello world"), "hello world");
    }

    #[test]
    fn test_expand_env_vars_with_var() {
        std::env::set_var("TEST_WCONFIG_VAR", "replaced");
        let result = expand_env_vars("prefix $ENV:TEST_WCONFIG_VAR suffix");
        assert_eq!(result, "prefix replaced suffix");
        std::env::remove_var("TEST_WCONFIG_VAR");
    }

    #[test]
    fn test_expand_env_vars_with_fallback() {
        let result = expand_env_vars("$ENV:NONEXISTENT_VAR_12345:fallback_value");
        assert_eq!(result, "fallback_value");
    }

    #[test]
    fn test_expand_env_vars_missing_no_fallback() {
        let result = expand_env_vars("$ENV:NONEXISTENT_VAR_99999");
        assert_eq!(result, "");
    }

    // -- read_config_file --

    #[test]
    fn test_read_config_file_missing() {
        let path = PathBuf::from("/nonexistent/settings.json");
        let (config, errors): (SettingsType, _) = read_config_file(&path);
        assert!(errors.is_empty()); // Missing file is not an error
        assert!(config.term_theme.is_empty());
    }

    #[test]
    fn test_read_config_file_with_comments() {
        let dir = std::env::temp_dir().join("agentmux_test_jsonc");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings_comments.json");
        std::fs::write(
            &path,
            r#"
// Terminal settings
{
    "term:fontsize": 16.0,
    /* theme */
    "term:theme": "dracula" // inline comment
}
"#,
        )
        .unwrap();

        let (config, errors): (SettingsType, _) = read_config_file(&path);
        assert!(errors.is_empty(), "errors: {:?}", errors);
        assert_eq!(config.term_font_size, 16.0);
        assert_eq!(config.term_theme, "dracula");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_read_config_file_trailing_commas() {
        let dir = std::env::temp_dir().join("agentmux_test_trailing");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings_trailing.json");
        std::fs::write(
            &path,
            r#"{
    // "term:fontsize": 12,
     "window:opacity": 0.7,
    // "window:bgcolor": ""
}"#,
        )
        .unwrap();

        let (config, errors): (SettingsType, _) = read_config_file(&path);
        assert!(errors.is_empty(), "trailing comma should be tolerated: {:?}", errors);
        assert_eq!(config.window_opacity, Some(0.7));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_strip_trailing_commas_basic() {
        assert_eq!(loader::strip_trailing_commas(r#"{"a": 1,}"#), r#"{"a": 1 }"#);
        assert_eq!(loader::strip_trailing_commas(r#"[1, 2,]"#), r#"[1, 2 ]"#);
        assert_eq!(loader::strip_trailing_commas(r#"{"a": "b,}"}"#), r#"{"a": "b,}"}"#);
        assert_eq!(loader::strip_trailing_commas(r#"{"a": 1, "b": 2}"#), r#"{"a": 1, "b": 2}"#);
    }

    // -- merge_into_template --

    #[test]
    fn test_merge_into_template_empty_settings() {
        let template = "// header\n{\n    // \"foo:bar\": 1,\n}\n";
        let settings = serde_json::Map::new();
        let result = merge_into_template(template, &settings);
        assert_eq!(result, template);
    }

    #[test]
    fn test_merge_into_template_uncomments_known_key() {
        let template = "{\n    // \"window:transparent\":       false,\n}\n";
        let mut settings = serde_json::Map::new();
        settings.insert("window:transparent".to_string(), serde_json::Value::Bool(true));
        let result = merge_into_template(template, &settings);
        assert!(result.contains("    \"window:transparent\": true,"));
        assert!(!result.contains("//"));
    }

    #[test]
    fn test_merge_into_template_appends_unknown_key() {
        let template = "{\n    // \"term:fontsize\":            12,\n}\n";
        let mut settings = serde_json::Map::new();
        settings.insert(
            "widget:order".to_string(),
            serde_json::json!(["agent", "settings"]),
        );
        let result = merge_into_template(template, &settings);
        assert!(result.contains("// -- User Overrides --"));
        assert!(result.contains("\"widget:order\": [\"agent\",\"settings\"]"));
        // Template line should still be commented
        assert!(result.contains("// \"term:fontsize\""));
    }

    #[test]
    fn test_merge_into_template_mixed() {
        let template = "{\n    // \"window:blur\":              false,\n    // \"term:fontsize\":            12,\n}\n";
        let mut settings = serde_json::Map::new();
        settings.insert("window:blur".to_string(), serde_json::Value::Bool(true));
        settings.insert("custom:key".to_string(), serde_json::json!("hello"));
        let result = merge_into_template(template, &settings);
        // Known key uncommented
        assert!(result.contains("    \"window:blur\": true,"));
        // Other known key still commented
        assert!(result.contains("// \"term:fontsize\""));
        // Unknown key appended
        assert!(result.contains("\"custom:key\": \"hello\""));
    }

    #[test]
    fn test_merge_into_template_idempotent() {
        let template = SETTINGS_TEMPLATE;
        let mut settings = serde_json::Map::new();
        settings.insert("window:transparent".to_string(), serde_json::Value::Bool(true));
        settings.insert("widget:order".to_string(), serde_json::json!(["a", "b"]));

        let first = merge_into_template(template, &settings);
        // Parse the result back and merge again
        let parsed = parse_jsonc_to_map(&first);
        let second = merge_into_template(template, &parsed);
        assert_eq!(first, second, "merge_into_template should be idempotent");
    }

    #[test]
    fn test_merge_into_template_preserves_indentation() {
        let template = "{\n        // \"deep:key\":   42,\n}\n";
        let mut settings = serde_json::Map::new();
        settings.insert("deep:key".to_string(), serde_json::json!(99));
        let result = merge_into_template(template, &settings);
        assert!(result.contains("        \"deep:key\": 99,"));
    }

    #[test]
    fn test_parse_jsonc_to_map() {
        let content = r#"// comment
{
    // "commented": true,
    "active": 42,
    "name": "test",
}
"#;
        let map = parse_jsonc_to_map(content);
        assert_eq!(map.get("active"), Some(&serde_json::json!(42)));
        assert_eq!(map.get("name"), Some(&serde_json::json!("test")));
        assert!(map.get("commented").is_none());
    }
}
