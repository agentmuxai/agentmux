// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Configuration type definitions: settings, themes, widgets, connections, bookmarks.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::obj::MetaMapType;

// ---- Serde helpers (used by skip_serializing_if attributes) ----

pub(crate) fn is_false(v: &bool) -> bool {
    !v
}

pub(crate) fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

pub(crate) fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

pub(crate) fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

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

    /// Maximum runtime in hours before the watchdog kills an agent pane.
    /// 0 (default) disables the limit.
    #[serde(rename = "term:agentmaxruntimehours", default, skip_serializing_if = "is_zero_f64")]
    pub term_agent_max_runtime_hours: f64,

    /// Minutes of PTY silence before the watchdog kills an idle agent pane.
    /// 0 (default) disables the limit.
    #[serde(rename = "term:agentidletimeoutmins", default, skip_serializing_if = "is_zero_f64")]
    pub term_agent_idle_timeout_mins: f64,

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

    #[serde(rename = "widget:icononly", default, skip_serializing_if = "Option::is_none")]
    pub widget_icon_only: Option<bool>,

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

    #[serde(rename = "telemetry:interval", default, skip_serializing_if = "is_zero_f64")]
    pub telemetry_interval: f64,

    #[serde(rename = "telemetry:numpoints", default, skip_serializing_if = "Option::is_none")]
    pub telemetry_numpoints: Option<i64>,

    // -- Connection settings --
    #[serde(rename = "conn:*", default, skip_serializing_if = "is_false")]
    pub conn_clear: bool,

    #[serde(rename = "conn:askbeforewshinstall", default, skip_serializing_if = "Option::is_none")]
    pub conn_ask_before_wsh_install: Option<bool>,

    #[serde(rename = "conn:wshenabled", default, skip_serializing_if = "is_false")]
    pub conn_wsh_enabled: bool,

    // -- Network settings --
    #[serde(rename = "network:lan_discovery", default, skip_serializing_if = "is_false")]
    pub network_lan_discovery: bool,

    /// Catch-all for unknown/dynamic keys (e.g. `widget:hidden@defwidget@sysinfo`).
    /// These pass through serde unchanged so the frontend can access them as flat settings keys.
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
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

    /// Whether this widget is pinned to the action bar by default on new installs.
    /// Once the user has a `widget:pinned` setting this field is ignored.
    #[serde(rename = "display:pinned", default, skip_serializing_if = "is_false")]
    pub display_pinned: bool,

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
