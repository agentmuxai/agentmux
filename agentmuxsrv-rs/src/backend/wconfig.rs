// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Configuration system: settings, themes, widgets, bookmarks, connections.
//! Port of Go's pkg/wconfig/.
//!
//! Provides the full configuration type hierarchy and a thread-safe
//! config watcher. The actual file system watching is deferred until
//! integrated with the Tauri event loop.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::waveobj::MetaMapType;

// ---- Config file constants ----

pub const SETTINGS_FILE: &str = "settings.json";
pub const CONNECTIONS_FILE: &str = "connections.json";
pub const PROFILES_FILE: &str = "profiles.json";

// ---- SettingsType ----

/// Application settings. Matches Go's `wconfig.SettingsType` JSON tags.
/// Fields use pointer-like `Option` for nullable booleans/numbers
/// to distinguish "not set" from "false/0".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsType {
    // -- App settings --
    #[serde(rename = "app:*", default, skip_serializing_if = "is_false")]
    pub app_clear: bool,

    #[serde(rename = "app:globalhotkey", default, skip_serializing_if = "String::is_empty")]
    pub app_global_hotkey: String,

    #[serde(rename = "app:dismissarchitecturewarning", default, skip_serializing_if = "is_false")]
    pub app_dismiss_architecture_warning: bool,

    #[serde(rename = "app:defaultnewblock", default, skip_serializing_if = "String::is_empty")]
    pub app_default_new_block: String,

    #[serde(rename = "app:showoverlayblocknums", default, skip_serializing_if = "Option::is_none")]
    pub app_show_overlay_block_nums: Option<bool>,

    // -- AI settings --
    #[serde(rename = "ai:*", default, skip_serializing_if = "is_false")]
    pub ai_clear: bool,

    #[serde(rename = "ai:preset", default, skip_serializing_if = "String::is_empty")]
    pub ai_preset: String,

    #[serde(rename = "ai:apitype", default, skip_serializing_if = "String::is_empty")]
    pub ai_api_type: String,

    #[serde(rename = "ai:baseurl", default, skip_serializing_if = "String::is_empty")]
    pub ai_base_url: String,

    #[serde(rename = "ai:apitoken", default, skip_serializing_if = "String::is_empty")]
    pub ai_api_token: String,

    #[serde(rename = "ai:name", default, skip_serializing_if = "String::is_empty")]
    pub ai_name: String,

    #[serde(rename = "ai:model", default, skip_serializing_if = "String::is_empty")]
    pub ai_model: String,

    #[serde(rename = "ai:orgid", default, skip_serializing_if = "String::is_empty")]
    pub ai_org_id: String,

    #[serde(rename = "ai:apiversion", default, skip_serializing_if = "String::is_empty")]
    pub ai_api_version: String,

    #[serde(rename = "ai:maxtokens", default, skip_serializing_if = "is_zero_f64")]
    pub ai_max_tokens: f64,

    #[serde(rename = "ai:timeoutms", default, skip_serializing_if = "is_zero_f64")]
    pub ai_timeout_ms: f64,

    #[serde(rename = "ai:proxyurl", default, skip_serializing_if = "String::is_empty")]
    pub ai_proxy_url: String,

    #[serde(rename = "ai:fontsize", default, skip_serializing_if = "is_zero_f64")]
    pub ai_font_size: f64,

    #[serde(rename = "ai:fixedfontsize", default, skip_serializing_if = "is_zero_f64")]
    pub ai_fixed_font_size: f64,

    // -- Terminal settings --
    #[serde(rename = "term:*", default, skip_serializing_if = "is_false")]
    pub term_clear: bool,

    #[serde(rename = "term:fontsize", default, skip_serializing_if = "is_zero_f64")]
    pub term_font_size: f64,

    #[serde(rename = "term:fontfamily", default, skip_serializing_if = "String::is_empty")]
    pub term_font_family: String,

    #[serde(rename = "term:theme", default, skip_serializing_if = "String::is_empty")]
    pub term_theme: String,

    #[serde(rename = "term:disablewebgl", default, skip_serializing_if = "is_false")]
    pub term_disable_web_gl: bool,

    #[serde(rename = "term:localshellpath", default, skip_serializing_if = "String::is_empty")]
    pub term_local_shell_path: String,

    #[serde(rename = "term:localshellopts", default, skip_serializing_if = "Vec::is_empty")]
    pub term_local_shell_opts: Vec<String>,

    #[serde(rename = "term:scrollback", default, skip_serializing_if = "Option::is_none")]
    pub term_scrollback: Option<i64>,

    #[serde(rename = "term:copyonselect", default, skip_serializing_if = "Option::is_none")]
    pub term_copy_on_select: Option<bool>,

    #[serde(rename = "term:transparency", default, skip_serializing_if = "Option::is_none")]
    pub term_transparency: Option<f64>,

    #[serde(rename = "term:allowbracketedpaste", default, skip_serializing_if = "Option::is_none")]
    pub term_allow_bracketed_paste: Option<bool>,

    #[serde(rename = "term:shiftenternewline", default, skip_serializing_if = "Option::is_none")]
    pub term_shift_enter_newline: Option<bool>,

    // -- Command settings --
    #[serde(rename = "cmd:env", default, skip_serializing_if = "HashMap::is_empty")]
    pub cmd_env: HashMap<String, String>,

    // -- Editor settings --
    #[serde(rename = "editor:minimapenabled", default, skip_serializing_if = "is_false")]
    pub editor_minimap_enabled: bool,

    #[serde(rename = "editor:stickyscrollenabled", default, skip_serializing_if = "is_false")]
    pub editor_sticky_scroll_enabled: bool,

    #[serde(rename = "editor:wordwrap", default, skip_serializing_if = "is_false")]
    pub editor_word_wrap: bool,

    #[serde(rename = "editor:fontsize", default, skip_serializing_if = "is_zero_f64")]
    pub editor_font_size: f64,

    // -- Block header settings --
    #[serde(rename = "blockheader:*", default, skip_serializing_if = "is_false")]
    pub block_header_clear: bool,

    #[serde(rename = "blockheader:showblockids", default, skip_serializing_if = "is_false")]
    pub block_header_show_block_ids: bool,

    // -- Auto-update settings --
    #[serde(rename = "autoupdate:*", default, skip_serializing_if = "is_false")]
    pub auto_update_clear: bool,

    #[serde(rename = "autoupdate:enabled", default, skip_serializing_if = "is_false")]
    pub auto_update_enabled: bool,

    #[serde(rename = "autoupdate:intervalms", default, skip_serializing_if = "is_zero_f64")]
    pub auto_update_interval_ms: f64,

    #[serde(rename = "autoupdate:installonquit", default, skip_serializing_if = "is_false")]
    pub auto_update_install_on_quit: bool,

    #[serde(rename = "autoupdate:channel", default, skip_serializing_if = "String::is_empty")]
    pub auto_update_channel: String,

    // -- Markdown settings --
    #[serde(rename = "markdown:fontsize", default, skip_serializing_if = "is_zero_f64")]
    pub markdown_font_size: f64,

    #[serde(rename = "markdown:fixedfontsize", default, skip_serializing_if = "is_zero_f64")]
    pub markdown_fixed_font_size: f64,

    // -- Preview settings --
    #[serde(rename = "preview:showhiddenfiles", default, skip_serializing_if = "Option::is_none")]
    pub preview_show_hidden_files: Option<bool>,

    // -- Tab settings --
    #[serde(rename = "tab:preset", default, skip_serializing_if = "String::is_empty")]
    pub tab_preset: String,

    // -- Widget settings --
    #[serde(rename = "widget:*", default, skip_serializing_if = "is_false")]
    pub widget_clear: bool,

    #[serde(rename = "widget:showhelp", default, skip_serializing_if = "Option::is_none")]
    pub widget_show_help: Option<bool>,

    // -- Window settings --
    #[serde(rename = "window:*", default, skip_serializing_if = "is_false")]
    pub window_clear: bool,

    #[serde(rename = "window:transparent", default, skip_serializing_if = "is_false")]
    pub window_transparent: bool,

    #[serde(rename = "window:blur", default, skip_serializing_if = "is_false")]
    pub window_blur: bool,

    #[serde(rename = "window:opacity", default, skip_serializing_if = "Option::is_none")]
    pub window_opacity: Option<f64>,

    #[serde(rename = "window:bgcolor", default, skip_serializing_if = "String::is_empty")]
    pub window_bg_color: String,

    #[serde(rename = "window:reducedmotion", default, skip_serializing_if = "is_false")]
    pub window_reduced_motion: bool,

    #[serde(rename = "window:tilegapsize", default, skip_serializing_if = "Option::is_none")]
    pub window_tile_gap_size: Option<i64>,

    #[serde(rename = "window:showmenubar", default, skip_serializing_if = "is_false")]
    pub window_show_menu_bar: bool,

    #[serde(rename = "window:nativetitlebar", default, skip_serializing_if = "is_false")]
    pub window_native_title_bar: bool,

    #[serde(rename = "window:disablehardwareacceleration", default, skip_serializing_if = "is_false")]
    pub window_disable_hardware_acceleration: bool,

    #[serde(rename = "window:maxtabcachesize", default, skip_serializing_if = "is_zero_i32")]
    pub window_max_tab_cache_size: i32,

    #[serde(rename = "window:magnifiedblockopacity", default, skip_serializing_if = "Option::is_none")]
    pub window_magnified_block_opacity: Option<f64>,

    #[serde(rename = "window:magnifiedblocksize", default, skip_serializing_if = "Option::is_none")]
    pub window_magnified_block_size: Option<f64>,

    #[serde(rename = "window:magnifiedblockblurprimarypx", default, skip_serializing_if = "Option::is_none")]
    pub window_magnified_block_blur_primary_px: Option<i64>,

    #[serde(rename = "window:magnifiedblockblursecondarypx", default, skip_serializing_if = "Option::is_none")]
    pub window_magnified_block_blur_secondary_px: Option<i64>,

    #[serde(rename = "window:confirmclose", default, skip_serializing_if = "is_false")]
    pub window_confirm_close: bool,

    #[serde(rename = "window:savelastwindow", default, skip_serializing_if = "is_false")]
    pub window_save_last_window: bool,

    #[serde(rename = "window:dimensions", default, skip_serializing_if = "String::is_empty")]
    pub window_dimensions: String,

    #[serde(rename = "window:zoom", default, skip_serializing_if = "Option::is_none")]
    pub window_zoom: Option<f64>,

    // -- Telemetry settings --
    #[serde(rename = "telemetry:*", default, skip_serializing_if = "is_false")]
    pub telemetry_clear: bool,

    #[serde(rename = "telemetry:enabled", default, skip_serializing_if = "is_false")]
    pub telemetry_enabled: bool,

    // -- Connection settings --
    #[serde(rename = "conn:*", default, skip_serializing_if = "is_false")]
    pub conn_clear: bool,

    #[serde(rename = "conn:askbeforewshinstall", default, skip_serializing_if = "Option::is_none")]
    pub conn_ask_before_wsh_install: Option<bool>,

    #[serde(rename = "conn:wshenabled", default, skip_serializing_if = "is_false")]
    pub conn_wsh_enabled: bool,
}

// ---- AI settings subset ----

/// AI-specific settings (used for presets).
/// Matches Go's `wconfig.AiSettingsType`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiSettingsType {
    #[serde(rename = "ai:*", default, skip_serializing_if = "is_false")]
    pub ai_clear: bool,

    #[serde(rename = "ai:preset", default, skip_serializing_if = "String::is_empty")]
    pub ai_preset: String,

    #[serde(rename = "ai:apitype", default, skip_serializing_if = "String::is_empty")]
    pub ai_api_type: String,

    #[serde(rename = "ai:baseurl", default, skip_serializing_if = "String::is_empty")]
    pub ai_base_url: String,

    #[serde(rename = "ai:apitoken", default, skip_serializing_if = "String::is_empty")]
    pub ai_api_token: String,

    #[serde(rename = "ai:name", default, skip_serializing_if = "String::is_empty")]
    pub ai_name: String,

    #[serde(rename = "ai:model", default, skip_serializing_if = "String::is_empty")]
    pub ai_model: String,

    #[serde(rename = "ai:orgid", default, skip_serializing_if = "String::is_empty")]
    pub ai_org_id: String,

    #[serde(rename = "ai:apiversion", default, skip_serializing_if = "String::is_empty")]
    pub ai_api_version: String,

    #[serde(rename = "ai:maxtokens", default, skip_serializing_if = "is_zero_f64")]
    pub ai_max_tokens: f64,

    #[serde(rename = "ai:timeoutms", default, skip_serializing_if = "is_zero_f64")]
    pub ai_timeout_ms: f64,

    #[serde(rename = "ai:proxyurl", default, skip_serializing_if = "String::is_empty")]
    pub ai_proxy_url: String,

    #[serde(rename = "ai:fontsize", default, skip_serializing_if = "is_zero_f64")]
    pub ai_font_size: f64,

    #[serde(rename = "ai:fixedfontsize", default, skip_serializing_if = "is_zero_f64")]
    pub ai_fixed_font_size: f64,

    #[serde(rename = "display:name", default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,

    #[serde(rename = "display:order", default, skip_serializing_if = "is_zero_f64")]
    pub display_order: f64,
}

// ---- Supporting config types ----

/// MIME type display configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MimeTypeConfigType {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

/// File definition for block widgets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileDef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub meta: HashMap<String, Value>,
}

/// Block definition for widgets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockDef {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub files: HashMap<String, FileDef>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub meta: MetaMapType,
}

/// Widget configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WidgetConfigType {
    #[serde(rename = "display:order", default, skip_serializing_if = "is_zero_f64")]
    pub display_order: f64,

    #[serde(rename = "display:hidden", default, skip_serializing_if = "is_false")]
    pub display_hidden: bool,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    #[serde(default, skip_serializing_if = "is_false")]
    pub magnified: bool,

    #[serde(rename = "blockdef", default)]
    pub block_def: BlockDef,
}

/// Terminal color theme.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TermThemeType {
    #[serde(rename = "display:name", default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,

    #[serde(rename = "display:order", default, skip_serializing_if = "is_zero_f64")]
    pub display_order: f64,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub black: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub red: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub green: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub yellow: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blue: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magenta: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cyan: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub white: String,

    #[serde(rename = "brightBlack", default, skip_serializing_if = "String::is_empty")]
    pub bright_black: String,
    #[serde(rename = "brightRed", default, skip_serializing_if = "String::is_empty")]
    pub bright_red: String,
    #[serde(rename = "brightGreen", default, skip_serializing_if = "String::is_empty")]
    pub bright_green: String,
    #[serde(rename = "brightYellow", default, skip_serializing_if = "String::is_empty")]
    pub bright_yellow: String,
    #[serde(rename = "brightBlue", default, skip_serializing_if = "String::is_empty")]
    pub bright_blue: String,
    #[serde(rename = "brightMagenta", default, skip_serializing_if = "String::is_empty")]
    pub bright_magenta: String,
    #[serde(rename = "brightCyan", default, skip_serializing_if = "String::is_empty")]
    pub bright_cyan: String,
    #[serde(rename = "brightWhite", default, skip_serializing_if = "String::is_empty")]
    pub bright_white: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gray: String,
    #[serde(rename = "cmdtext", default, skip_serializing_if = "String::is_empty")]
    pub cmd_text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub foreground: String,
    #[serde(rename = "selectionBackground", default, skip_serializing_if = "String::is_empty")]
    pub selection_background: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub background: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cursor: String,
}

/// Web bookmark.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebBookmark {
    #[serde(default)]
    pub url: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,

    #[serde(rename = "iconcolor", default, skip_serializing_if = "String::is_empty")]
    pub icon_color: String,

    #[serde(rename = "iconurl", default, skip_serializing_if = "String::is_empty")]
    pub icon_url: String,

    #[serde(rename = "display:order", default, skip_serializing_if = "is_zero_f64")]
    pub display_order: f64,
}

/// Per-connection configuration keywords.
/// Matches Go's `wconfig.ConnKeywords`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnKeywords {
    // -- Connection settings --
    #[serde(rename = "conn:wshenabled", default, skip_serializing_if = "Option::is_none")]
    pub conn_wsh_enabled: Option<bool>,

    #[serde(rename = "conn:askbeforewshinstall", default, skip_serializing_if = "Option::is_none")]
    pub conn_ask_before_wsh_install: Option<bool>,

    #[serde(rename = "conn:wshpath", default, skip_serializing_if = "String::is_empty")]
    pub conn_wsh_path: String,

    #[serde(rename = "conn:shellpath", default, skip_serializing_if = "String::is_empty")]
    pub conn_shell_path: String,

    #[serde(rename = "conn:ignoresshconfig", default, skip_serializing_if = "Option::is_none")]
    pub conn_ignore_ssh_config: Option<bool>,

    // -- Display settings --
    #[serde(rename = "display:hidden", default, skip_serializing_if = "Option::is_none")]
    pub display_hidden: Option<bool>,

    #[serde(rename = "display:order", default, skip_serializing_if = "is_zero_f32")]
    pub display_order: f32,

    // -- Terminal settings --
    #[serde(rename = "term:*", default, skip_serializing_if = "is_false")]
    pub term_clear: bool,

    #[serde(rename = "term:fontsize", default, skip_serializing_if = "is_zero_f64")]
    pub term_font_size: f64,

    #[serde(rename = "term:fontfamily", default, skip_serializing_if = "String::is_empty")]
    pub term_font_family: String,

    #[serde(rename = "term:theme", default, skip_serializing_if = "String::is_empty")]
    pub term_theme: String,

    // -- Command settings --
    #[serde(rename = "cmd:env", default, skip_serializing_if = "HashMap::is_empty")]
    pub cmd_env: HashMap<String, String>,

    #[serde(rename = "cmd:initscript", default, skip_serializing_if = "String::is_empty")]
    pub cmd_init_script: String,

    #[serde(rename = "cmd:initscript.sh", default, skip_serializing_if = "String::is_empty")]
    pub cmd_init_script_sh: String,

    #[serde(rename = "cmd:initscript.bash", default, skip_serializing_if = "String::is_empty")]
    pub cmd_init_script_bash: String,

    #[serde(rename = "cmd:initscript.zsh", default, skip_serializing_if = "String::is_empty")]
    pub cmd_init_script_zsh: String,

    #[serde(rename = "cmd:initscript.pwsh", default, skip_serializing_if = "String::is_empty")]
    pub cmd_init_script_pwsh: String,

    #[serde(rename = "cmd:initscript.fish", default, skip_serializing_if = "String::is_empty")]
    pub cmd_init_script_fish: String,

    // -- SSH settings --
    #[serde(rename = "ssh:user", default, skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,

    #[serde(rename = "ssh:hostname", default, skip_serializing_if = "Option::is_none")]
    pub ssh_hostname: Option<String>,

    #[serde(rename = "ssh:port", default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<String>,

    #[serde(rename = "ssh:identityfile", default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_identity_file: Vec<String>,

    #[serde(rename = "ssh:batchmode", default, skip_serializing_if = "Option::is_none")]
    pub ssh_batch_mode: Option<bool>,

    #[serde(rename = "ssh:pubkeyauthentication", default, skip_serializing_if = "Option::is_none")]
    pub ssh_pubkey_authentication: Option<bool>,

    #[serde(rename = "ssh:passwordauthentication", default, skip_serializing_if = "Option::is_none")]
    pub ssh_password_authentication: Option<bool>,

    #[serde(rename = "ssh:kbdinteractiveauthentication", default, skip_serializing_if = "Option::is_none")]
    pub ssh_kbd_interactive_authentication: Option<bool>,

    #[serde(rename = "ssh:preferredauthentications", default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_preferred_authentications: Vec<String>,

    #[serde(rename = "ssh:addkeystoagent", default, skip_serializing_if = "Option::is_none")]
    pub ssh_add_keys_to_agent: Option<bool>,

    #[serde(rename = "ssh:identityagent", default, skip_serializing_if = "Option::is_none")]
    pub ssh_identity_agent: Option<String>,

    #[serde(rename = "ssh:identitiesonly", default, skip_serializing_if = "Option::is_none")]
    pub ssh_identities_only: Option<bool>,

    #[serde(rename = "ssh:proxyjump", default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_proxy_jump: Vec<String>,

    #[serde(rename = "ssh:userknownhostsfile", default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_user_known_hosts_file: Vec<String>,

    #[serde(rename = "ssh:globalknownhostsfile", default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_global_known_hosts_file: Vec<String>,
}

/// Configuration error from parsing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigError {
    pub file: String,
    pub err: String,
}

/// Webhook integration configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfigType {
    #[serde(default)]
    pub version: String,

    #[serde(rename = "workspaceId", default)]
    pub workspace_id: String,

    #[serde(rename = "authToken", default)]
    pub auth_token: String,

    #[serde(rename = "cloudEndpoint", default)]
    pub cloud_endpoint: String,

    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub terminals: Vec<String>,
}

// ---- Full config container ----

/// Complete application configuration.
/// Matches Go's `wconfig.FullConfigType`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FullConfigType {
    #[serde(default)]
    pub settings: SettingsType,

    #[serde(rename = "mimetypes", default)]
    pub mime_types: HashMap<String, MimeTypeConfigType>,

    #[serde(rename = "defaultwidgets", default)]
    pub default_widgets: HashMap<String, WidgetConfigType>,

    #[serde(default)]
    pub widgets: HashMap<String, WidgetConfigType>,

    #[serde(default)]
    pub presets: HashMap<String, MetaMapType>,

    #[serde(rename = "termthemes", default)]
    pub term_themes: HashMap<String, TermThemeType>,

    #[serde(default)]
    pub connections: HashMap<String, ConnKeywords>,

    #[serde(default)]
    pub bookmarks: HashMap<String, WebBookmark>,

    #[serde(rename = "configerrors", default, skip_serializing_if = "Vec::is_empty")]
    pub config_errors: Vec<ConfigError>,
}

// ---- Config watcher ----

/// Thread-safe configuration holder with change notification.
/// The actual file system watching will be integrated with Tauri's
/// event loop in a later phase.
pub struct ConfigWatcher {
    config: RwLock<Arc<FullConfigType>>,
}

impl ConfigWatcher {
    /// Create a new config watcher with default config.
    pub fn new() -> Self {
        Self {
            config: RwLock::new(Arc::new(FullConfigType::default())),
        }
    }

    /// Create a new config watcher with initial config.
    pub fn with_config(config: FullConfigType) -> Self {
        Self {
            config: RwLock::new(Arc::new(config)),
        }
    }

    /// Get a snapshot of the current config.
    pub fn get_full_config(&self) -> Arc<FullConfigType> {
        self.config.read().unwrap().clone()
    }

    /// Get just the settings.
    pub fn get_settings(&self) -> SettingsType {
        self.config.read().unwrap().settings.clone()
    }

    /// Update the full config (called when files change).
    pub fn set_config(&self, config: FullConfigType) {
        let mut current = self.config.write().unwrap();
        *current = Arc::new(config);
    }

    /// Update just the settings portion.
    pub fn update_settings(&self, settings: SettingsType) {
        let mut current = self.config.write().unwrap();
        let mut new_config = (**current).clone();
        new_config.settings = settings;
        *current = Arc::new(new_config);
    }
}

impl Default for ConfigWatcher {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Default config builder ----

/// Build the initial default configuration with embedded default assets.
///
/// Loads the bundled `widgets.json` (from `pkg/wconfig/defaultconfig/`) at compile time
/// and populates `FullConfigType.widgets` so the frontend widget bar is populated on startup.
pub fn build_default_config() -> FullConfigType {
    let mut config = FullConfigType::default();

    // Embed widgets.json at compile time (equivalent to Go's //go:embed)
    const WIDGETS_JSON: &str =
        include_str!("../config/widgets.json");

    match serde_json::from_str::<HashMap<String, WidgetConfigType>>(WIDGETS_JSON) {
        Ok(widgets) => {
            config.widgets = widgets;
        }
        Err(e) => {
            eprintln!("wconfig: failed to parse embedded widgets.json: {}", e);
        }
    }

    config
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

    match serde_json::from_str(&content) {
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

/// Replace `$ENV:VAR_NAME` and `$ENV:VAR_NAME:fallback` in a string.
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

// ---- Serde helpers ----

fn is_false(v: &bool) -> bool {
    !v
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    // -- SettingsType serde --

    #[test]
    fn test_settings_default_empty() {
        let s = SettingsType::default();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_settings_ai_fields() {
        let s = SettingsType {
            ai_api_type: "anthropic".to_string(),
            ai_model: "claude-3-opus".to_string(),
            ai_max_tokens: 4096.0,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"ai:apitype\":\"anthropic\""));
        assert!(json.contains("\"ai:model\":\"claude-3-opus\""));
        assert!(json.contains("\"ai:maxtokens\":4096.0"));
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
    fn test_settings_from_go_json() {
        let go_json = r#"{
            "ai:apitype": "openai",
            "ai:model": "gpt-4",
            "ai:maxtokens": 2048,
            "term:fontsize": 13,
            "term:theme": "solarized-dark",
            "term:scrollback": 5000,
            "window:transparent": true,
            "window:opacity": 0.85,
            "telemetry:enabled": true
        }"#;
        let s: SettingsType = serde_json::from_str(go_json).unwrap();
        assert_eq!(s.ai_api_type, "openai");
        assert_eq!(s.ai_model, "gpt-4");
        assert_eq!(s.ai_max_tokens, 2048.0);
        assert_eq!(s.term_font_size, 13.0);
        assert_eq!(s.term_scrollback, Some(5000));
        assert!(s.window_transparent);
        assert_eq!(s.window_opacity, Some(0.85));
        assert!(s.telemetry_enabled);
    }

    #[test]
    fn test_settings_roundtrip() {
        let s = SettingsType {
            ai_api_type: "anthropic".to_string(),
            term_font_size: 14.0,
            window_opacity: Some(0.95),
            conn_wsh_enabled: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let parsed: SettingsType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.ai_api_type, "anthropic");
        assert_eq!(parsed.term_font_size, 14.0);
        assert_eq!(parsed.window_opacity, Some(0.95));
        assert!(parsed.conn_wsh_enabled);
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
            conn_wsh_enabled: Some(true),
            ..Default::default()
        };
        let json = serde_json::to_string(&kw).unwrap();
        assert!(json.contains("\"ssh:user\":\"admin\""));
        assert!(json.contains("\"ssh:hostname\":\"server.example.com\""));
        assert!(json.contains("\"ssh:port\":\"2222\""));
        assert!(json.contains("\"ssh:identityfile\":[\"~/.ssh/id_rsa\"]"));
        assert!(json.contains("\"conn:wshenabled\":true"));
    }

    #[test]
    fn test_conn_keywords_from_go_json() {
        let go_json = r#"{
            "conn:wshenabled": true,
            "conn:askbeforewshinstall": false,
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
        assert_eq!(parsed.conn_wsh_enabled, Some(true));
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
        config.settings.ai_model = "claude-3".to_string();
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
        assert_eq!(parsed.settings.ai_model, "claude-3");
        assert_eq!(parsed.term_themes.len(), 1);
        assert_eq!(
            parsed.term_themes.get("test").unwrap().display_name,
            "Test Theme"
        );
        assert_eq!(parsed.bookmarks.len(), 1);
    }

    #[test]
    fn test_full_config_from_go_json() {
        let go_json = r##"{
            "settings": {
                "ai:apitype": "anthropic",
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
        let parsed: FullConfigType = serde_json::from_str(go_json).unwrap();
        assert_eq!(parsed.settings.ai_api_type, "anthropic");
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

    // -- ConfigWatcher --

    #[test]
    fn test_config_watcher_default() {
        let watcher = ConfigWatcher::new();
        let config = watcher.get_full_config();
        assert!(config.settings.ai_model.is_empty());
    }

    #[test]
    fn test_config_watcher_with_initial() {
        let mut config = FullConfigType::default();
        config.settings.ai_model = "test-model".to_string();
        let watcher = ConfigWatcher::with_config(config);
        assert_eq!(watcher.get_settings().ai_model, "test-model");
    }

    #[test]
    fn test_config_watcher_set_config() {
        let watcher = ConfigWatcher::new();
        let mut config = FullConfigType::default();
        config.settings.term_font_size = 16.0;
        watcher.set_config(config);
        assert_eq!(watcher.get_settings().term_font_size, 16.0);
    }

    #[test]
    fn test_config_watcher_update_settings() {
        let watcher = ConfigWatcher::new();
        let settings = SettingsType {
            ai_api_type: "openai".to_string(),
            ..Default::default()
        };
        watcher.update_settings(settings);
        assert_eq!(watcher.get_settings().ai_api_type, "openai");
    }

    #[test]
    fn test_config_watcher_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let watcher = Arc::new(ConfigWatcher::new());
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let w = watcher.clone();
                thread::spawn(move || {
                    let s = SettingsType {
                        ai_model: format!("model-{}", i),
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
        assert!(config.ai_model.is_empty());
    }

    // -- AiSettingsType serde --

    #[test]
    fn test_ai_settings_type_serde() {
        let ai = AiSettingsType {
            ai_api_type: "anthropic".to_string(),
            ai_model: "claude-3-opus".to_string(),
            ai_max_tokens: 4096.0,
            display_name: "Claude Opus".to_string(),
            display_order: 1.0,
            ..Default::default()
        };
        let json = serde_json::to_string(&ai).unwrap();
        assert!(json.contains("\"ai:apitype\":\"anthropic\""));
        assert!(json.contains("\"display:name\":\"Claude Opus\""));
        assert!(json.contains("\"display:order\":1.0"));
    }
}
