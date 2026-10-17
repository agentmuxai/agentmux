// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The desktop's own window-button layout, for the Linux title-bar controls.
// docs/specs/SPEC_LINUX_WINDOW_CONTROLS_GNOME_STYLE_2026_10_16.md §3.2, §4.

/// GNOME's window-button layout, `org.gnome.desktop.wm.preferences
/// button-layout` (for example `":minimize,maximize,close"`), read once and
/// cached. `None` off Linux, or when the setting can't be read (no libgio, no
/// such schema or key); the page then uses its default.
pub fn get_window_button_layout() -> serde_json::Value {
    #[cfg(target_os = "linux")]
    {
        static LAYOUT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        serde_json::json!(LAYOUT.get_or_init(|| unsafe { gsettings::read_string(c"org.gnome.desktop.wm.preferences", c"button-layout") }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        serde_json::Value::Null
    }
}

/// GSettings through a runtime-loaded libgio, as `app/gl_probe.rs` loads EGL:
/// no build dependency, and a system without GLib just gets `None`. The schema
/// is looked up first because `g_settings_new` aborts the process on an
/// unknown schema.
#[cfg(target_os = "linux")]
mod gsettings {
    use std::ffi::{c_char, c_int, c_void, CStr};

    type Ptr = *mut c_void;
    type SourceGetDefault = extern "C" fn() -> Ptr;
    type SourceLookup = extern "C" fn(Ptr, *const c_char, c_int) -> Ptr;
    type SchemaHasKey = extern "C" fn(Ptr, *const c_char) -> c_int;
    type SchemaUnref = extern "C" fn(Ptr);
    type SettingsNew = extern "C" fn(*const c_char) -> Ptr;
    type GetString = extern "C" fn(Ptr, *const c_char) -> *mut c_char;
    type ObjectUnref = extern "C" fn(Ptr);
    type Free = extern "C" fn(Ptr);

    unsafe fn sym<T: Copy>(lib: Ptr, name: &CStr) -> Option<T> {
        let p = libc::dlsym(lib, name.as_ptr());
        (!p.is_null()).then(|| std::mem::transmute_copy::<Ptr, T>(&p))
    }

    pub(super) unsafe fn read_string(schema_id: &CStr, key: &CStr) -> Option<String> {
        let gio = libc::dlopen(c"libgio-2.0.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if gio.is_null() {
            return None;
        }
        // dlsym on the libgio handle also finds GObject's and GLib's symbols,
        // which libgio depends on.
        let source_get_default: SourceGetDefault = sym(gio, c"g_settings_schema_source_get_default")?;
        let source_lookup: SourceLookup = sym(gio, c"g_settings_schema_source_lookup")?;
        let schema_has_key: SchemaHasKey = sym(gio, c"g_settings_schema_has_key")?;
        let schema_unref: SchemaUnref = sym(gio, c"g_settings_schema_unref")?;
        let settings_new: SettingsNew = sym(gio, c"g_settings_new")?;
        let get_string: GetString = sym(gio, c"g_settings_get_string")?;
        let object_unref: ObjectUnref = sym(gio, c"g_object_unref")?;
        let free: Free = sym(gio, c"g_free")?;

        let source = source_get_default();
        if source.is_null() {
            return None;
        }
        let schema = source_lookup(source, schema_id.as_ptr(), 1);
        if schema.is_null() {
            return None;
        }
        let has_key = schema_has_key(schema, key.as_ptr()) != 0;
        schema_unref(schema);
        if !has_key {
            return None;
        }
        let settings = settings_new(schema_id.as_ptr());
        if settings.is_null() {
            return None;
        }
        let raw = get_string(settings, key.as_ptr());
        let value = (!raw.is_null()).then(|| CStr::from_ptr(raw).to_string_lossy().into_owned());
        if !raw.is_null() {
            free(raw.cast());
        }
        object_unref(settings);
        // libgio stays loaded: unloading GLib from a process is not safe.
        value
    }
}
