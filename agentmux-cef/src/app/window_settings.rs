// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window-level settings plumbing: the Linux `WindowDelegate` FFI override
// that sets the Wayland app_id / X11 WM_CLASS, the selected-ozone-platform
// publication point, and the `window:transparent` settings.json reader used
// to gate transparent-compositing command-line flags before CefInitialize.
// Split out of `app.rs` (now `app/mod.rs`).

/// Override the `get_linux_window_properties` function pointer on a
/// `WindowDelegate` to write the AgentMux app_id directly to the C struct,
/// bypassing the buggy `CefString` → `cef_string_utf16_t` conversion in the
/// cef 146.7.0 wrapper (`Clear` variant gets dropped during writeback).
///
/// Without this, CEF emits `xdg_toplevel.set_app_id("")` and GNOME / KWin /
/// sway can't match the window to `agentmux.desktop`, so the AgentMux icon
/// never appears in the taskbar/dock/launcher.
///
/// Must be called once on every `WindowDelegate` we create (top-level, popup,
/// new sub-window) before passing it to `window_create_top_level`.
#[cfg(target_os = "linux")]
pub fn install_linux_window_properties_override(delegate: &cef::WindowDelegate) {
    use cef::ImplWindowDelegate;
    // Disambiguate: WindowDelegate implements get_raw on three traits
    // (ImplViewDelegate / ImplPanelDelegate / ImplWindowDelegate). We need
    // the WindowDelegate one to get the right struct type for casting.
    let raw: *mut cef::sys::_cef_window_delegate_t =
        <cef::WindowDelegate as ImplWindowDelegate>::get_raw(delegate);
    unsafe {
        (*raw).get_linux_window_properties = Some(write_linux_window_properties);
    }
}

/// Custom extern "C" shim invoked by libcef to populate
/// `_cef_linux_window_properties_t`. Writes "agentmux" to wayland_app_id
/// and the X11 wm_class fields via cef-dll-sys utf8→utf16 setters,
/// then returns 1 so libcef uses the values.
#[cfg(target_os = "linux")]
extern "C" fn write_linux_window_properties(
    _self_: *mut cef::sys::_cef_window_delegate_t,
    _window: *mut cef::sys::_cef_window_t,
    properties: *mut cef::sys::_cef_linux_window_properties_t,
) -> std::os::raw::c_int {
    if properties.is_null() {
        return 0;
    }
    const APP_ID: &[u8] = b"agentmux";
    unsafe {
        let props = &mut *properties;
        // The C struct's strings start zeroed (libcef constructs a default
        // CefLinuxWindowProperties). cef_string_utf8_to_utf16 allocates a
        // new utf-16 buffer and assigns it to the dest cef_string_utf16_t;
        // ownership transfers to libcef which calls dtor when done.
        cef::sys::cef_string_utf8_to_utf16(
            APP_ID.as_ptr().cast(), APP_ID.len(), &mut props.wayland_app_id,
        );
        cef::sys::cef_string_utf8_to_utf16(
            APP_ID.as_ptr().cast(), APP_ID.len(), &mut props.wm_class_class,
        );
        cef::sys::cef_string_utf8_to_utf16(
            APP_ID.as_ptr().cast(), APP_ID.len(), &mut props.wm_class_name,
        );
    }
    1
}

/// The ozone platform this process appended to the command line (Linux).
/// Unset when nothing was appended (pure X11 session → Chromium's X11
/// default). Read by `ui_tasks::SetWindowAlphaTask` to decide whether
/// `window_handle()` is an X11 XID that `_NET_WM_WINDOW_OPACITY` can target.
#[cfg(target_os = "linux")]
pub static SELECTED_OZONE_PLATFORM: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Read `window:transparent` from the user's settings.json before CefInitialize.
/// Gates the transparent-compositing command-line flags so non-transparent
/// windows don't pay the LCD-text and opacity-flash penalties. Returns false
/// if the file is absent or the key is missing (default = opaque).
pub(crate) fn read_window_transparent_setting() -> bool {
    // Candidate locations for the LIVE settings.json, in priority order —
    // mirrors srv's `config_watcher_fs::resolve_settings_dir()` (the file the
    // settings UI actually edits). See
    // docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md.
    //
    // Isolated (default for every channel except `stable` —
    // agentmux_common::isolated_settings_enabled()):
    //   1. $AGENTMUX_SETTINGS_DIR/settings.json (explicit override)
    //   2. $AGENTMUX_CONFIG_DIR (or $AGENTMUX_CONFIG_HOME — pre-unification
    //      name, same value)/settings.json — the channel-scoped config dir.
    // Deliberately does NOT fall through to the shared/legacy candidates
    // below: an isolated channel's own settings.json existing-but-lacking
    // `window:transparent` must read the default (false), not silently
    // inherit the shared file's value — that fallthrough would defeat
    // isolation for exactly the surprise-carry-over case this spec exists
    // to prevent, just for a different key.
    //
    // Global (`stable`, or an explicit AGENTMUX_ISOLATED_SETTINGS=0
    // opt-out) — unchanged from before this spec:
    //   1. $AGENTMUX_SETTINGS_DIR/settings.json (explicit override)
    //   2. $AGENTMUX_CONFIG_HOME/../../settings.json (channels-root shared
    //      file — the modern location, e.g. ~/.agentmux/channels/settings.json)
    //   3. $AGENTMUX_CONFIG_DIR/settings.json (per-instance config dir)
    //   4. ~/.agentmux/channels/settings.json
    //   5. ~/.agentmux/settings.json (legacy)
    // First file that exists wins. The old code (pre-dating this
    // module's original fix) checked ONLY (3), which is empty in every
    // real deployment — so `window:transparent` silently read `false`
    // for everyone on Linux/macOS.
    fn candidates() -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Some(d) = std::env::var_os("AGENTMUX_SETTINGS_DIR").filter(|s| !s.is_empty()) {
            out.push(std::path::PathBuf::from(d).join("settings.json"));
        }

        if agentmux_common::isolated_settings_enabled() {
            // Isolated channel: the per-instance config dir IS the settings
            // dir (no parent-walk to a shared location) — same value
            // srv's resolve_settings_dir() uses on an isolated channel.
            // Prefers AGENTMUX_CONFIG_DIR (the canonical var) but accepts
            // AGENTMUX_CONFIG_HOME too, since bootstrap.rs re-exports the
            // same value under that legacy name and this function may run
            // in a process that only has one of the two set.
            //
            // No DataPaths::from_env() fallback here (reagentx P1 on
            // #2664): from_env() itself requires AGENTMUX_CONFIG_DIR via
            // `?` (data_paths.rs:307), so it can only ever succeed in
            // exactly the case already handled above, and can only be
            // reached here when that case already failed — i.e. it would
            // always return None too. A branch that can never produce a
            // value is worse than no branch: it reads as "there's a
            // working fallback" when there isn't one. If both vars are
            // genuinely absent, there is no isolated candidate to offer.
            if let Some(d) = std::env::var_os("AGENTMUX_CONFIG_DIR")
                .or_else(|| std::env::var_os("AGENTMUX_CONFIG_HOME"))
                .filter(|s| !s.is_empty())
            {
                out.push(std::path::PathBuf::from(d).join("settings.json"));
            }
            return out;
        }

        if let Some(d) = std::env::var_os("AGENTMUX_CONFIG_HOME").filter(|s| !s.is_empty()) {
            let p = std::path::PathBuf::from(d);
            if let Some(root) = p.parent().and_then(|p| p.parent()) {
                out.push(root.join("settings.json"));
            }
        }
        // No DataPaths::from_env() fallback here either (same dead-branch
        // reasoning as above, pre-existing before this PR but the same
        // bug: from_env() requires AGENTMUX_CONFIG_DIR, and this branch
        // is only reached when AGENTMUX_CONFIG_DIR is already unset — so
        // it could never have produced a value here either).
        if let Some(d) = std::env::var_os("AGENTMUX_CONFIG_DIR").filter(|s| !s.is_empty()) {
            out.push(std::path::PathBuf::from(d).join("settings.json"));
        }
        if let Some(home) = dirs::home_dir() {
            out.push(home.join(".agentmux").join("channels").join("settings.json"));
            out.push(home.join(".agentmux").join("settings.json"));
        }
        out
    }
    for path in candidates() {
        if !path.exists() {
            continue;
        }
        // settings.json is JSONC (comments + trailing commas) — a strict
        // serde_json parse fails on the shipped template. Use the same
        // lenient reader the settings command path uses.
        let map = crate::commands::platform::read_settings_jsonc(&path);
        if let Some(v) = map.get("window:transparent").and_then(|v| v.as_bool()) {
            tracing::info!("window:transparent={} (from {})", v, path.display());
            return v;
        }
        // File exists but has no (uncommented) key → default false, but keep
        // scanning lower-priority locations in case an older file has it.
    }
    false
}
