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

    // -- Block header settings --
    #[serde(rename = "blockheader:*", default, skip_serializing_if = "is_false")]
    pub block_header_clear: bool,

    #[serde(rename = "blockheader:showblockids", default, skip_serializing_if = "is_false")]
    pub block_header_show_block_ids: bool,

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

    // -- Network settings --
    #[serde(rename = "network:lan_discovery", default, skip_serializing_if = "is_false")]
    pub network_lan_discovery: bool,

    // -- Voice settings --
    //
    // `voice:enabled` globally controls whether the per-pane microphone
    // button is rendered. The default at the UX layer is "enabled" — the
    // frontend treats `undefined`/absent as enabled, so we only need to
    // model the explicit-disable case here. Users set this to `false` in
    // `settings.json` to fully hide the buttons across all panes.
    //
    // Spec: docs/specs/SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §7 Phase 3.
    #[serde(rename = "voice:enabled", default, skip_serializing_if = "Option::is_none")]
    pub voice_enabled: Option<bool>,

    // Speech-to-text engine: "whisper" (capture audio → server STT, the default
    // and only engine that works in CEF) or "webspeech" (browser API — dev /
    // real-Chromium only). Absent ⇒ whisper.
    // Spec: docs/specs/SPEC_VOICE_STT_ENGINE_2026_06_20.md.
    #[serde(rename = "voice:engine", default, skip_serializing_if = "Option::is_none")]
    pub voice_engine: Option<String>,

    // Groq API key for the hosted Whisper backend. Read server-side only; never
    // sent to the renderer. The AGENTMUX_GROQ_API_KEY env var takes precedence.
    #[serde(rename = "voice:groqApiKey", default, skip_serializing_if = "Option::is_none")]
    pub voice_groq_api_key: Option<String>,

    // Local whisper.cpp backend (voice:engine = "whisper-local"). Both are
    // user-provided paths in v1 (auto-download is a follow-up). Env overrides:
    // AGENTMUX_WHISPER_CLI / AGENTMUX_WHISPER_MODEL.
    #[serde(rename = "voice:whisperCliPath", default, skip_serializing_if = "Option::is_none")]
    pub voice_whisper_cli_path: Option<String>,
    // GGML model name to auto-download on first use (default "base.en"). The
    // explicit-path override below skips the download.
    #[serde(rename = "voice:whisperModel", default, skip_serializing_if = "Option::is_none")]
    pub voice_whisper_model: Option<String>,
    #[serde(rename = "voice:whisperModelPath", default, skip_serializing_if = "Option::is_none")]
    pub voice_whisper_model_path: Option<String>,

    // -- Notification sounds --
    //
    // Spec: docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §5.1.
    //
    // Master enable. Default at the UX layer is "on" — the frontend treats
    // `undefined`/absent as enabled, so absence means sounds play. Users
    // set this to `false` to fully silence the app.
    #[serde(rename = "notify:sounds:enabled", default, skip_serializing_if = "Option::is_none")]
    pub notify_sounds_enabled: Option<bool>,

    // 0.0 to 1.0; default 0.6 if unset.
    #[serde(rename = "notify:sounds:volume", default, skip_serializing_if = "Option::is_none")]
    pub notify_sounds_volume: Option<f32>,

    // Suppress a pane's sound when that pane is focused AND the window
    // is in foreground. Default: true.
    #[serde(rename = "notify:sounds:suppresswhenfocused", default, skip_serializing_if = "Option::is_none")]
    pub notify_sounds_suppress_when_focused: Option<bool>,

    // Per-event opt-out. Absence = on (default behavior).
    #[serde(rename = "notify:sound:agent.turn.complete", default, skip_serializing_if = "Option::is_none")]
    pub notify_sound_agent_turn_complete: Option<bool>,

    #[serde(rename = "notify:sound:agent.turn.error", default, skip_serializing_if = "Option::is_none")]
    pub notify_sound_agent_turn_error: Option<bool>,

    #[serde(rename = "notify:sound:agent.turn.interrupted", default, skip_serializing_if = "Option::is_none")]
    pub notify_sound_agent_turn_interrupted: Option<bool>,

    #[serde(rename = "notify:sound:agent.message.accepted", default, skip_serializing_if = "Option::is_none")]
    pub notify_sound_agent_message_accepted: Option<bool>,

    #[serde(rename = "notify:sound:agent.message.rejected", default, skip_serializing_if = "Option::is_none")]
    pub notify_sound_agent_message_rejected: Option<bool>,

    // -- Tool-call tones (subliminal per-tool "voice") --
    //
    // Spec: docs/specs/SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md.
    //
    // Master enable. Default at the UX layer is "on" — absence is on
    // by design (see spec §7).
    #[serde(rename = "notify:tooltones:enabled", default, skip_serializing_if = "Option::is_none")]
    pub notify_tooltones_enabled: Option<bool>,

    // Independent gain (0.0–1.0; default 0.25). Layered below the
    // shared master gain so master kill-switches both subsystems.
    #[serde(rename = "notify:tooltones:volume", default, skip_serializing_if = "Option::is_none")]
    pub notify_tooltones_volume: Option<f32>,

    // Scope: "all" (default) plays for every pane in every window;
    // "focused" plays only for the focused pane in the focused window.
    // The intermediate "window" mode is reserved for v1.5.
    #[serde(rename = "notify:tooltones:scope", default, skip_serializing_if = "Option::is_none")]
    pub notify_tooltones_scope: Option<String>,

    // -- Messaging bridge settings --

    /// Master enable for the Discord messaging bridge.
    /// When true, the bridge connects to the Discord Gateway at startup.
    #[serde(rename = "messaging:discord:enabled", default, skip_serializing_if = "is_false")]
    pub messaging_discord_enabled: bool,

    /// Discord bot token. Treat as a secret — do not log. Obtain from
    /// discord.com/developers/applications → Bot → Token.
    #[serde(rename = "messaging:discord:token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_discord_token: Option<String>,

    /// Channel ID to filter inbound messages and use as the default send target.
    #[serde(rename = "messaging:discord:channel", default, skip_serializing_if = "String::is_empty")]
    pub messaging_discord_channel: String,

    /// Agent ID that receives inbound Discord messages via the reactive bus.
    /// Absent → messages are logged but not forwarded to any agent.
    #[serde(rename = "messaging:discord:target", default, skip_serializing_if = "Option::is_none")]
    pub messaging_discord_target: Option<String>,

    /// Guild ID for guild-scoped slash command registration (Phase 2).
    #[serde(rename = "messaging:discord:guild", default, skip_serializing_if = "Option::is_none")]
    pub messaging_discord_guild: Option<String>,

    // -- Messaging bridge settings (Telegram) --

    /// Master enable for the Telegram messaging bridge.
    /// When true, the bridge starts long-polling getUpdates at startup.
    #[serde(rename = "messaging:telegram:enabled", default, skip_serializing_if = "is_false")]
    pub messaging_telegram_enabled: bool,

    /// Telegram bot token from @BotFather. Treat as a secret — do not log.
    #[serde(rename = "messaging:telegram:token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_telegram_token: Option<String>,

    /// Comma-separated allowlist of chat IDs permitted to reach the bridge.
    /// Inbound updates from any other chat are silently dropped.
    /// Stored as a string (not Vec<i64>) to keep the flat-key/settings.json
    /// convention simple — parsed to Vec<i64> at startup wiring time.
    #[serde(rename = "messaging:telegram:allowed_chats", default, skip_serializing_if = "String::is_empty")]
    pub messaging_telegram_allowed_chats: String,

    /// Default chat ID for outbound sends when a request doesn't override one.
    #[serde(rename = "messaging:telegram:default_chat", default, skip_serializing_if = "Option::is_none")]
    pub messaging_telegram_default_chat: Option<String>,

    /// Agent ID that receives inbound Telegram messages via the reactive bus.
    /// Absent → messages are logged but not forwarded to any agent.
    #[serde(rename = "messaging:telegram:target", default, skip_serializing_if = "Option::is_none")]
    pub messaging_telegram_target: Option<String>,

    // -- Slack messaging bridge --

    /// Master enable for the Slack messaging bridge.
    /// When true, the bridge opens a Socket Mode connection at startup.
    #[serde(rename = "messaging:slack:enabled", default, skip_serializing_if = "is_false")]
    pub messaging_slack_enabled: bool,

    /// Slack bot token (`xoxb-...`). Used for Web API calls (chat.postMessage).
    /// Treat as a secret — do not log. Obtain from api.slack.com/apps → OAuth & Permissions
    /// → Bot User OAuth Token, after installing the app to the workspace.
    #[serde(rename = "messaging:slack:bot_token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_slack_bot_token: Option<String>,

    /// Slack app-level token (`xapp-...`). Used only for apps.connections.open (Socket Mode).
    /// Treat as a secret — do not log. Obtain from api.slack.com/apps → Basic Information
    /// → App-Level Tokens, scope `connections:write`.
    #[serde(rename = "messaging:slack:app_token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_slack_app_token: Option<String>,

    /// Channel ID to filter inbound messages and use as the default send target.
    #[serde(rename = "messaging:slack:channel", default, skip_serializing_if = "String::is_empty")]
    pub messaging_slack_channel: String,

    /// Agent ID that receives inbound Slack messages via the reactive bus.
    /// Absent → messages are logged but not forwarded to any agent.
    #[serde(rename = "messaging:slack:target", default, skip_serializing_if = "Option::is_none")]
    pub messaging_slack_target: Option<String>,

    // -- WhatsApp Cloud API messaging bridge --
    //
    // See docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md.
    // No `messaging:whatsapp:mode` key — the spec's decision (§2.1) is
    // Cloud API only for v1, explicitly dropping the unofficial Baileys
    // path, so there is exactly one mode.

    /// Master enable for the WhatsApp Cloud API bridge.
    /// When true, the outbound sender starts and the `/webhook/whatsapp`
    /// routes become live (they're always registered on the router; this
    /// flag controls whether `WhatsAppBridge::get()` resolves).
    #[serde(rename = "messaging:whatsapp:enabled", default, skip_serializing_if = "is_false")]
    pub messaging_whatsapp_enabled: bool,

    /// WhatsApp Business phone number ID (Meta App Dashboard > WhatsApp > API Setup).
    #[serde(rename = "messaging:whatsapp:phone_number_id", default, skip_serializing_if = "String::is_empty")]
    pub messaging_whatsapp_phone_number_id: String,

    /// System User access token (permanent). Treat as a secret — do not log.
    #[serde(rename = "messaging:whatsapp:access_token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_whatsapp_access_token: Option<String>,

    /// Meta App Secret, used to validate X-Hub-Signature-256 on inbound
    /// webhooks. Treat as a secret — do not log.
    #[serde(rename = "messaging:whatsapp:app_secret", default, skip_serializing_if = "Option::is_none")]
    pub messaging_whatsapp_app_secret: Option<String>,

    /// Verify token used in the GET /webhook/whatsapp handshake. User-chosen,
    /// must match what's entered in Meta App Dashboard > WhatsApp > Configuration.
    /// Treat as a secret — do not log.
    #[serde(rename = "messaging:whatsapp:webhook_verify_token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_whatsapp_webhook_verify_token: Option<String>,

    /// Agent ID that receives inbound WhatsApp messages via the reactive bus.
    /// Absent → messages are logged but not forwarded to any agent.
    #[serde(rename = "messaging:whatsapp:target", default, skip_serializing_if = "Option::is_none")]
    pub messaging_whatsapp_target: Option<String>,

    /// Template name used for outbound sends outside the 24h customer
    /// service window. If unset and the window has expired, send() fails
    /// fast rather than round-tripping to the Graph API (spec §3.4).
    #[serde(rename = "messaging:whatsapp:fallback_template", default, skip_serializing_if = "Option::is_none")]
    pub messaging_whatsapp_fallback_template: Option<String>,

    /// Template language code (BCP-47). Default "en_US" if unset.
    #[serde(rename = "messaging:whatsapp:fallback_template_lang", default, skip_serializing_if = "Option::is_none")]
    pub messaging_whatsapp_fallback_template_lang: Option<String>,

    /// Public webhook origin (e.g. "wa.yourdomain.com") that a
    /// user-managed tunnel (Cloudflare Tunnel, ngrok, or otherwise) points
    /// at this instance's webhook port. **v1 does not spawn or supervise a
    /// tunnel subprocess** — this field is used only to print the full
    /// callback URL in the startup log as a setup reminder; the user is
    /// responsible for standing up the tunnel and registering
    /// `https://<this>/webhook/whatsapp` in Meta's App Dashboard themselves.
    /// See messaging/whatsapp/mod.rs for the full scoping rationale.
    #[serde(rename = "messaging:whatsapp:tunnel_domain", default, skip_serializing_if = "String::is_empty")]
    pub messaging_whatsapp_tunnel_domain: String,

    /// Catch-all for unknown/dynamic keys (e.g. `widget:hidden@defwidget@sysinfo`).
    /// These pass through serde unchanged so the frontend can access them as flat settings keys.
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
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

    /// Ordered short-names (no "defwidget@" prefix, same convention as
    /// `widget:pinned`) of this widget's children. A non-empty `children`
    /// list is what makes a widget entry a "parent" — it expands a submenu
    /// on the widget bar / More dropdown instead of opening a pane, so its
    /// own `blockdef` is unused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,

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
#[allow(dead_code)]
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
