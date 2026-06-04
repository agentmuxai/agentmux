// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Native macOS menu bar — Phase 1 of
// docs/specs/SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03.md.
//
// Built with raw libobjc FFI, matching the macOS shims in main.rs (no .m /
// build.rs change). Two kinds of item:
//
//   * STANDARD items (Edit: undo/redo/cut/copy/paste/select-all; App:
//     hide/quit; Window: minimize/zoom) use AppKit's standard selectors with a
//     nil target, so AppKit routes them to the first responder — the focused
//     CEF web view. This is what makes ⌘C/⌘V/⌘Z/⌘A work reliably in the web
//     inputs, and why it's the load-bearing reason for a native Edit menu.
//   * CUSTOM items dispatch `menu:invoke {commandId}` to the host frontend
//     renderers; the focused window runs it through the shared `commandRegistry`
//     (frontend/app/store/command-registry.ts). Phase 1 leaves custom items
//     accelerator-free — keymodel.ts keeps owning those chords (zero
//     regression); shortcut reconciliation is Phase 2.

#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{Arc, OnceLock};

use crate::state::AppState;

type Id = *mut c_void;
type Sel = *const c_void;
type Class = *mut c_void;

extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    fn objc_registerClassPair(cls: Class);
    fn class_addMethod(cls: Class, sel: Sel, imp: usize, types: *const c_char) -> u8;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
}

// NSEventModifierFlags.
const MOD_CMD: u64 = 1 << 20;
const MOD_SHIFT: u64 = 1 << 17;
const MOD_OPT: u64 = 1 << 19;

// AppState handle for the menu action (a bare C function with no captured env).
static MENU_STATE: OnceLock<Arc<AppState>> = OnceLock::new();

#[inline]
unsafe fn class(name: &[u8]) -> Class {
    objc_getClass(name.as_ptr() as *const c_char)
}
#[inline]
unsafe fn sel(name: &[u8]) -> Sel {
    sel_registerName(name.as_ptr() as *const c_char)
}
#[inline]
unsafe fn msg(recv: Id, s: Sel) -> Id {
    let f: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
unsafe fn msg_id(recv: Id, s: Sel, a: Id) -> Id {
    let f: extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s, a)
}
unsafe fn nsstr(s: &str) -> Id {
    let c = CString::new(s).unwrap_or_default();
    let f: extern "C" fn(Class, Sel, *const c_char) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    f(class(b"NSString\0"), sel(b"stringWithUTF8String:\0"), c.as_ptr())
}

/// `agentmuxMenuInvoke:` — the action for every custom menu item. Reads the
/// item's `representedObject` (the command id) and broadcasts `menu:invoke` to
/// the host frontends; the focused window runs the command.
unsafe extern "C" fn menu_invoke(_self: Id, _cmd: Sel, sender: Id) {
    if sender.is_null() {
        return;
    }
    let rep = msg(sender, sel(b"representedObject\0"));
    if rep.is_null() {
        return;
    }
    let utf8: extern "C" fn(Id, Sel) -> *const c_char =
        std::mem::transmute(objc_msgSend as *const c_void);
    let cstr = utf8(rep, sel(b"UTF8String\0"));
    if cstr.is_null() {
        return;
    }
    let cmd = CStr::from_ptr(cstr).to_string_lossy().into_owned();
    match MENU_STATE.get() {
        Some(state) => crate::events::emit_event_to_top_level_windows(
            state,
            "menu:invoke",
            &serde_json::json!({ "commandId": cmd }),
        ),
        None => tracing::warn!(command = %cmd, "menu:invoke before MENU_STATE set"),
    }
}

/// Build (once) the shared target object that owns `agentmuxMenuInvoke:`.
/// Leaked on purpose — it lives for the process lifetime.
unsafe fn make_target() -> Id {
    let name = b"AgentMuxMenuTarget\0";
    let mut cls = class(name);
    if cls.is_null() {
        cls = objc_allocateClassPair(class(b"NSObject\0"), name.as_ptr() as *const c_char, 0);
        // -(void)agentmuxMenuInvoke:(id)sender  →  "v@:@"
        let imp: unsafe extern "C" fn(Id, Sel, Id) = menu_invoke;
        class_addMethod(
            cls,
            sel(b"agentmuxMenuInvoke:\0"),
            imp as usize,
            b"v@:@\0".as_ptr() as *const c_char,
        );
        objc_registerClassPair(cls);
    }
    msg(msg(cls as Id, sel(b"alloc\0")), sel(b"init\0"))
}

unsafe fn new_menu(title: &str) -> Id {
    let alloc = msg(class(b"NSMenu\0") as Id, sel(b"alloc\0"));
    msg_id(alloc, sel(b"initWithTitle:\0"), nsstr(title))
}

unsafe fn new_item(title: &str, key: &str, mask: u64) -> Id {
    let alloc = msg(class(b"NSMenuItem\0") as Id, sel(b"alloc\0"));
    let init: extern "C" fn(Id, Sel, Id, Sel, Id) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let item = init(
        alloc,
        sel(b"initWithTitle:action:keyEquivalent:\0"),
        nsstr(title),
        std::ptr::null(), // action set later (or nil for submenu headers)
        nsstr(key),
    );
    if mask != 0 {
        let set_mask: extern "C" fn(Id, Sel, u64) =
            std::mem::transmute(objc_msgSend as *const c_void);
        set_mask(item, sel(b"setKeyEquivalentModifierMask:\0"), mask);
    }
    item
}

/// Add a top-level submenu to the main menu; returns the submenu to populate.
unsafe fn add_submenu(main: Id, title: &str) -> Id {
    let header = new_item(title, "", 0);
    let sub = new_menu(title);
    msg_id(header, sel(b"setSubmenu:\0"), sub);
    msg_id(main, sel(b"addItem:\0"), header);
    sub
}

/// Standard item: nil target → AppKit routes the selector to the first responder.
unsafe fn add_std(menu: Id, title: &str, selector: &[u8], key: &str, mask: u64) {
    let item = new_item(title, key, mask);
    let set_action: extern "C" fn(Id, Sel, Sel) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_action(item, sel(b"setAction:\0"), sel(selector));
    msg_id(menu, sel(b"addItem:\0"), item);
}

/// Custom item: dispatches `menu:invoke {commandId}` to the focused frontend.
unsafe fn add_cmd(menu: Id, target: Id, title: &str, command_id: &str, key: &str, mask: u64) {
    let item = new_item(title, key, mask);
    let set_action: extern "C" fn(Id, Sel, Sel) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_action(item, sel(b"setAction:\0"), sel(b"agentmuxMenuInvoke:\0"));
    msg_id(item, sel(b"setTarget:\0"), target);
    msg_id(item, sel(b"setRepresentedObject:\0"), nsstr(command_id));
    msg_id(menu, sel(b"addItem:\0"), item);
}

unsafe fn add_sep(menu: Id) {
    let sep = msg(class(b"NSMenuItem\0") as Id, sel(b"separatorItem\0"));
    msg_id(menu, sel(b"addItem:\0"), sep);
}

fn is_dev() -> bool {
    let mode = agentmux_common::RuntimeMode::from_env().or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| agentmux_common::RuntimeMode::current(&d))
    });
    matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. }))
}

/// Install the native macOS menu bar. Must run AFTER `cef::initialize` (the
/// `NSApplication` instance exists by then) and after `set_macos_app_display_name`
/// (the app-menu title follows the process name).
pub fn install_menu_bar(state: Arc<AppState>) {
    unsafe { install_inner(state) }
}

unsafe fn install_inner(state: Arc<AppState>) {
    let _ = MENU_STATE.set(state);
    let target = make_target();
    let name = if is_dev() { "AgentMux DEV" } else { "AgentMux" };

    let main = new_menu("MainMenu");

    // ── App menu (AppKit titles it from the process name; item labels use it) ──
    let app_menu = add_submenu(main, name);
    add_cmd(app_menu, target, "Settings…", "dev:open_settings", "", 0);
    add_cmd(app_menu, target, "Identity & Memory…", "app:identity", "", 0);
    add_sep(app_menu);
    add_std(app_menu, &format!("Hide {name}"), b"hide:\0", "h", MOD_CMD);
    add_std(app_menu, "Hide Others", b"hideOtherApplications:\0", "h", MOD_CMD | MOD_OPT);
    add_std(app_menu, "Show All", b"unhideAllApplications:\0", "", 0);
    add_sep(app_menu);
    add_std(app_menu, &format!("Quit {name}"), b"terminate:\0", "q", MOD_CMD);

    // ── File ──
    let file = add_submenu(main, "File");
    add_cmd(file, target, "New Tab", "tab:new", "", 0);
    add_cmd(file, target, "New Window", "window:new", "", 0);
    add_sep(file);
    add_cmd(file, target, "Close Tab", "tab:close", "", 0);
    add_cmd(file, target, "Close Window", "window:close", "", 0);

    // ── Edit (standard selectors — the editing-in-web-inputs win) ──
    let edit = add_submenu(main, "Edit");
    add_std(edit, "Undo", b"undo:\0", "z", MOD_CMD);
    add_std(edit, "Redo", b"redo:\0", "z", MOD_CMD | MOD_SHIFT);
    add_sep(edit);
    add_std(edit, "Cut", b"cut:\0", "x", MOD_CMD);
    add_std(edit, "Copy", b"copy:\0", "c", MOD_CMD);
    add_std(edit, "Paste", b"paste:\0", "v", MOD_CMD);
    add_std(edit, "Select All", b"selectAll:\0", "a", MOD_CMD);

    // ── View ──
    let view = add_submenu(main, "View");
    add_cmd(view, target, "Command Palette…", "view:command-palette", "", 0);
    add_sep(view);
    add_cmd(view, target, "Zoom In", "view:zoom:in", "", 0);
    add_cmd(view, target, "Zoom Out", "view:zoom:out", "", 0);
    add_cmd(view, target, "Actual Size", "view:zoom:reset", "", 0);
    add_sep(view);
    add_cmd(view, target, "Toggle DevTools", "dev:devtools", "", 0);

    // ── Window (standard + tab nav) ──
    let window = add_submenu(main, "Window");
    // No ⌘M key equivalent in Phase 1: AppKit's performKeyEquivalent: would
    // consume ⌘M before the web content, stealing it from the existing
    // magnify-pane shortcut (keymodel.ts `Cmd:m`). The standard ⌘M→Minimize
    // assignment lands in Phase 2, together with the magnify→⌃⌘M remap and the
    // keymodel cession (see SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03 §4).
    add_std(window, "Minimize", b"performMiniaturize:\0", "", 0);
    add_std(window, "Zoom", b"performZoom:\0", "", 0);
    add_sep(window);
    add_cmd(window, target, "Next Tab", "tab:next", "", 0);
    add_cmd(window, target, "Previous Tab", "tab:prev", "", 0);

    // ── Help ──
    let help = add_submenu(main, "Help");
    add_cmd(help, target, &format!("{name} Help"), "help:docs", "", 0);

    // Install + register the Windows menu (auto window list / Bring-All-to-Front).
    let app = msg(class(b"NSApplication\0") as Id, sel(b"sharedApplication\0"));
    msg_id(app, sel(b"setMainMenu:\0"), main);
    msg_id(app, sel(b"setWindowsMenu:\0"), window);

    tracing::info!("macOS: installed native menu bar (SPEC menu-bar Phase 1)");
}
