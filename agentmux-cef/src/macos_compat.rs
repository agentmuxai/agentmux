// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// macOS ObjC runtime compatibility shims, extracted out of lib.rs's `run()`
// bootstrap sequence. Each function here is self-contained (declares its own
// local `extern "C"` ObjC bindings) and independently documented — they share
// no private state with `run()` beyond being called from within it. See each
// function's doc comment for the specific bug/workaround it addresses.

#![cfg(target_os = "macos")]

/// macOS 26 Tahoe compatibility shim.
///
/// CEF 146 calls private `NSApplication` selectors (e.g. `isHandlingSendEvent`,
/// `isSendingEvent`) during `NSDraggingSession` setup that Apple removed in
/// macOS 26. Without a handler the ObjC runtime walks `___forwarding___`,
/// finds nothing, and `objc_exception_throw`s; AppKit's default uncaught
/// handler calls `_objc_terminate()` and the host dies with `EXC_BREAKPOINT`
/// before Rust panic machinery runs.
///
/// We hook `+[NSApplication resolveInstanceMethod:]` on the metaclass —
/// the earliest point in the ObjC dispatch chain — and install typed stubs
/// for any unknown selector *before* the forwarding machinery is entered.
/// Swizzling `doesNotRecognizeSelector:` would not work: that method is
/// invoked FROM inside `___forwarding___`, and returning normally without
/// throwing corrupts forwarding state and causes a secondary crash there.
///
/// Return-type matters: `isHandlingSendEvent` and `isSendingEvent` return
/// `BOOL` and callers act on the value. A `void` stub leaves `x0 = self`
/// (truthy) on ARM64, making CEF think the app is already inside a
/// `sendEvent:` call and skip normal event routing — which breaks window
/// drag silently. A maintained allowlist of `BOOL`-returning selectors
/// gets a `BOOL_no_stub` returning `0` (NO); everything else gets a void
/// stub, which is safe for the unbounded set of removed Apple-private APIs.
///
/// Safety: Called once, before `cef::initialize`. `NSApplication` is a
/// singleton; adding a `+resolveInstanceMethod:` implementation on its
/// metaclass at startup is documented Apple behavior. No allocations, no
/// crossings of language boundaries that hold Rust references.
///
/// Ported from PR #403 (a5af, 2026-04-15) with rationale comments expanded.
/// See `docs/specs/SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md` and
/// `docs/analysis/REPORT_MACOS_TEAROFF_DRAG_CRASH_2026_05_29.md`.
pub(crate) unsafe fn patch_nsapp_unrecognized_selector() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn object_getClass(obj: Id) -> Class; // on a Class obj → returns metaclass
        fn sel_registerName(name: *const c_char) -> Sel;
        fn sel_getName(sel: Sel) -> *const c_char;
        fn class_addMethod(cls: Class, sel: Sel, imp: usize, types: *const c_char) -> u8;
    }

    // Void stub — safe for unknown selectors whose return value isn't read.
    unsafe extern "C" fn void_stub(_self: Id, _cmd: Sel) {}

    // BOOL stub returning 0 (NO). On ARM64 the return value lives in `x0`;
    // a void stub leaves `x0 = self` (truthy), breaking CEF's sendEvent: guard.
    unsafe extern "C" fn bool_no_stub(_self: Id, _cmd: Sel) -> u8 { 0 }

    // +resolveInstanceMethod: injected onto NSApplication's metaclass.
    // The ObjC runtime calls us the first time a selector is sent to an
    // NSApplication instance that has no implementation. We `class_addMethod`
    // a typed stub and return YES; the runtime retries the original message
    // against the freshly added method.
    unsafe extern "C" fn resolve_instance_method_impl(
        cls:  Class,
        _cmd: Sel,
        sel:  Sel,
    ) -> u8 {
        let name = {
            let ptr = sel_getName(sel);
            if ptr.is_null() {
                "<unknown>".to_owned()
            } else {
                std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };

        // BOOL-returning getters whose value callers act on. The truthy
        // garbage a void stub would leave in x0 breaks event routing.
        const BOOL_NO_SELECTORS: &[&str] = &[
            "isHandlingSendEvent",
            "isSendingEvent",
        ];

        if BOOL_NO_SELECTORS.contains(&name.as_str()) {
            tracing::warn!(selector = %name, "macOS 26 compat: adding BOOL(NO) stub");
            class_addMethod(cls, sel, bool_no_stub as usize, b"c@:\0".as_ptr() as _);
        } else {
            tracing::warn!(selector = %name, "macOS 26 compat: adding void stub");
            class_addMethod(cls, sel, void_stub as usize, b"v@:\0".as_ptr() as _);
        }
        1 // YES — resolved; runtime retries the original send
    }

    let cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls.is_null() {
        tracing::warn!("macOS 26 compat: NSApplication class not found");
        return;
    }

    // +resolveInstanceMethod: is a class method; it lives on the metaclass.
    let metacls = object_getClass(cls as Id);
    if metacls.is_null() {
        tracing::warn!("macOS 26 compat: NSApplication metaclass not found");
        return;
    }

    let sel = sel_registerName(b"resolveInstanceMethod:\0".as_ptr() as _);
    // "c@::" = BOOL return, id (Class), SEL (cmd), SEL (queried selector)
    let added = class_addMethod(
        metacls,
        sel,
        resolve_instance_method_impl as usize,
        b"c@::\0".as_ptr() as _,
    );
    if added != 0 {
        tracing::info!("macOS 26 compat: injected resolveInstanceMethod: into NSApplication metaclass");
    } else {
        tracing::warn!("macOS 26 compat: class_addMethod failed (method already exists?)");
    }
}

/// Suppress AppKit's native drag slide-back animation app-wide.
///
/// When an `NSDraggingSession` ends without a successful drop (the drag
/// operation resolves to `NSDragOperationNone`), AppKit animates the drag
/// image sliding back to where the drag began. For a pane/tab tear-off the
/// pointer is released outside any DOM drop target — the floating window is
/// created on mouseup — so blink reports `NSDragOperationNone` and the user
/// sees the drag image fly back into the source window before the new
/// window appears. That "rejection" animation is exactly what tear-off
/// wants gone; the frontend `preventUnhandled` guard (PR #1186) only
/// suppresses the WebKit-level snapback for *in-document* drops and can't
/// reach this AppKit-level animation.
///
/// `NSDraggingSession` exposes `animatesToStartingPositionsOnCancelOrFail`
/// (default `YES`) to control exactly this. CEF/Chromium starts every drag
/// via `-[NSView beginDraggingSessionWithItems:event:source:]` and never
/// flips the flag, so we swizzle that method: call the original, then set
/// the flag to `NO` on the returned session. Done at the `NSView` level so
/// it covers whichever Chromium content view initiates the drag. The flag
/// only affects the cancel/fail slide-back — successful in-window drops
/// (e.g. tab reorder) are unaffected — so disabling it globally is safe;
/// there is no place in the app where a drop-rejected slide-back is wanted.
///
/// Safety: called once at startup, before `cef::initialize`, on the main
/// thread. Mirrors the raw-libobjc FFI of `patch_nsapp_unrecognized_selector`.
pub(crate) unsafe fn disable_macos_drag_slideback() {
    use std::ffi::{c_char, c_void};

    type Id     = *mut c_void;
    type Sel    = *const c_void;
    type Class  = *mut c_void;
    type Method = *mut c_void;
    type Imp    = *const c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn class_getInstanceMethod(cls: Class, sel: Sel) -> Method;
        fn method_getImplementation(m: Method) -> Imp;
        fn method_setImplementation(m: Method, imp: Imp) -> Imp;
        fn objc_msgSend();
    }

    // IMP of the original beginDraggingSessionWithItems:event:source:, saved
    // so our replacement can chain to it. Single-threaded startup write +
    // main-thread-only drag reads, so a plain static is sufficient.
    static mut ORIGINAL_BEGIN_DRAG: Imp = std::ptr::null();

    // Replacement IMP: call the original to create the session, then clear
    // the slide-back flag on it before returning.
    unsafe extern "C" fn begin_drag_no_slideback(
        this:   Id,
        cmd:    Sel,
        items:  Id,
        event:  Id,
        source: Id,
    ) -> Id {
        let orig: extern "C" fn(Id, Sel, Id, Id, Id) -> Id =
            std::mem::transmute(ORIGINAL_BEGIN_DRAG);
        let session = orig(this, cmd, items, event, source);
        if !session.is_null() {
            // [session setAnimatesToStartingPositionsOnCancelOrFail:NO]
            let sel = sel_registerName(
                b"setAnimatesToStartingPositionsOnCancelOrFail:\0".as_ptr() as _,
            );
            let set_flag: extern "C" fn(Id, Sel, u8) =
                std::mem::transmute(objc_msgSend as *const c_void);
            set_flag(session, sel, 0); // NO
        }
        session
    }

    let cls = objc_getClass(b"NSView\0".as_ptr() as _);
    if cls.is_null() {
        tracing::warn!("drag slide-back: NSView class not found");
        return;
    }
    let sel = sel_registerName(
        b"beginDraggingSessionWithItems:event:source:\0".as_ptr() as _,
    );
    let method = class_getInstanceMethod(cls, sel);
    if method.is_null() {
        tracing::warn!("drag slide-back: beginDraggingSessionWithItems:event:source: not found");
        return;
    }
    ORIGINAL_BEGIN_DRAG = method_getImplementation(method);
    method_setImplementation(method, begin_drag_no_slideback as Imp);
    tracing::info!(
        "macOS drag polish: swizzled NSView beginDraggingSession to disable cancel/fail slide-back"
    );
}

/// Set the macOS app display name (Dock tile + app-menu title).
///
/// A bundle-less binary (`task dev` direct-invoke, the launcher's flat
/// `dist/cef-dev/` layout) has no `Info.plist`, so AppKit derives the app name
/// from the process name — `agentmux-cef` — which is what shows in the Dock and
/// the menu bar's app menu. Override it with a friendly name: **dev** builds get
/// `AgentMux DEV` (so they're visibly distinct from a packaged `AgentMux` and
/// from each other when several instances run), everything else gets `AgentMux`.
/// A packaged `.app` carries `CFBundleName` in its `Info.plist`; this still runs
/// and simply matches it.
///
/// Uses `-[NSProcessInfo setProcessName:]`, which AppKit reads for the Dock
/// label and the app-menu title. Must run AFTER `cef::initialize` — Chromium
/// overwrites the process name during init, so a pre-init set is clobbered;
/// setting it here (right before our menu bar is built) is what sticks. Raw
/// libobjc FFI, mirroring the other macOS shims.
unsafe fn set_macos_app_display_name_impl(set_bundle_id: bool) {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    // Resolve dev vs. not by the exe PATH (`is_dev_self`), matching
    // `commands::platform::get_is_dev` and the menu name in `macos_menu.rs`.
    // NOT `AGENTMUX_RUNTIME_MODE`: a parent dev AgentMux leaks that env into
    // descendants, which would otherwise set the Dock / app-menu process name
    // to "AgentMux DEV" on a packaged build launched from inside a dev
    // instance. Build identity is a property of the binary on disk.
    let name = if agentmux_common::is_dev_self() { "AgentMux DEV" } else { "AgentMux" };

    // NSString *ns = [NSString stringWithUTF8String:name]
    let cls_str = objc_getClass(b"NSString\0".as_ptr() as _);
    if cls_str.is_null() {
        return;
    }
    let sel_with = sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _);
    let make: extern "C" fn(Class, Sel, *const c_char) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let cname = match std::ffi::CString::new(name) {
        Ok(c) => c,
        Err(_) => return,
    };
    let ns_name = make(cls_str, sel_with, cname.as_ptr());
    if ns_name.is_null() {
        return;
    }

    // pi = [NSProcessInfo processInfo]
    let cls_pi = objc_getClass(b"NSProcessInfo\0".as_ptr() as _);
    if cls_pi.is_null() {
        return;
    }
    let sel_pi = sel_registerName(b"processInfo\0".as_ptr() as _);
    let get_pi: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let pi = get_pi(cls_pi, sel_pi);
    if pi.is_null() {
        return;
    }

    // [pi setProcessName:ns_name]
    let sel_set = sel_registerName(b"setProcessName:\0".as_ptr() as _);
    let set_name: extern "C" fn(Id, Sel, Id) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_name(pi, sel_set, ns_name);

    // The app-menu title + Dock label for an unbundled binary come from the
    // main bundle's CFBundleName/CFBundleDisplayName, NOT the process name
    // (setProcessName above doesn't move them). Set them on the main bundle's
    // info dictionary, which is backed by a mutable dictionary. Guard on
    // isKindOfClass:NSMutableDictionary so an unexpected immutable dict is a
    // skip rather than a throw (an uncaught ObjC exception would terminate the
    // host on macOS 26).
    let cls_bundle = objc_getClass(b"NSBundle\0".as_ptr() as _);
    if !cls_bundle.is_null() {
        let sel_main = sel_registerName(b"mainBundle\0".as_ptr() as _);
        let get_main: extern "C" fn(Class, Sel) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        let bundle = get_main(cls_bundle, sel_main);
        if !bundle.is_null() {
            let sel_info = sel_registerName(b"infoDictionary\0".as_ptr() as _);
            let get_info: extern "C" fn(Id, Sel) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            let info = get_info(bundle, sel_info);
            let cls_mut = objc_getClass(b"NSMutableDictionary\0".as_ptr() as _);
            let sel_kind = sel_registerName(b"isKindOfClass:\0".as_ptr() as _);
            let is_kind: extern "C" fn(Id, Sel, Class) -> u8 =
                std::mem::transmute(objc_msgSend as *const c_void);
            if !info.is_null() && !cls_mut.is_null() && is_kind(info, sel_kind, cls_mut) != 0 {
                let sel_set_obj = sel_registerName(b"setObject:forKey:\0".as_ptr() as _);
                let set_obj: extern "C" fn(Id, Sel, Id, Id) =
                    std::mem::transmute(objc_msgSend as *const c_void);
                let k_name = make(cls_str, sel_with, b"CFBundleName\0".as_ptr() as _);
                let k_disp = make(cls_str, sel_with, b"CFBundleDisplayName\0".as_ptr() as _);
                set_obj(info, sel_set_obj, ns_name, k_name);
                set_obj(info, sel_set_obj, ns_name, k_disp);

                // CFBundleIdentifier: only set on PACKAGED builds (running
                // inside a proper .app bundle). For dev builds (flat binary,
                // no bundle), setting CFBundleIdentifier at any point — before
                // OR after cef::initialize — triggers macOS to fire a
                // LaunchServices notification that causes Chromium to spawn a
                // --type=web-app-shortcut-copier subprocess. That subprocess
                // CHECK-aborts (EXC_BREAKPOINT / SIGTRAP) because the binary
                // is not in a signed .app bundle. Packaged builds already have
                // CFBundleIdentifier in their Info.plist so no runtime set is
                // needed there either. Skip for dev builds entirely.
                // TODO(faf754e6): if LaunchServices Dock-tile isolation for
                // dev builds is needed in the future, find a way to set the ID
                // that does NOT trigger the MacAppCodeSignClone spawn (e.g.
                // a CEF source patch to disable the spawn for unbundled builds).
                if set_bundle_id && !agentmux_common::is_dev_self() {
                    let channel = std::env::var("AGENTMUX_CHANNEL")
                        .unwrap_or_else(|_| "dev".to_string());
                    let channel_sanitized: String = channel
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' })
                        .collect::<String>()
                        .to_lowercase();
                    let bundle_id = format!("ai.agentmux.{}.{}", channel_sanitized, env!("CARGO_PKG_VERSION"));
                    if let Ok(c_id) = std::ffi::CString::new(bundle_id.as_str()) {
                        let ns_id = make(cls_str, sel_with, c_id.as_ptr());
                        if !ns_id.is_null() {
                            let k_id = make(cls_str, sel_with, b"CFBundleIdentifier\0".as_ptr() as _);
                            set_obj(info, sel_set_obj, ns_id, k_id);
                        }
                    }
                    tracing::info!(
                        bundle_id = %bundle_id,
                        "macOS: set CFBundleName/CFBundleDisplayName/CFBundleIdentifier on main bundle"
                    );
                } else if set_bundle_id {
                    tracing::info!("macOS: set CFBundleName/CFBundleDisplayName on main bundle (dev build: CFBundleIdentifier skipped to prevent MacAppCodeSignClone SIGTRAP)");
                } else {
                    tracing::info!("macOS: set CFBundleName/CFBundleDisplayName on main bundle (pre-init pass)");
                }
            } else {
                tracing::warn!("macOS: main bundle info dict not mutable; app name unchanged");
            }
        }
    }

    tracing::info!(app_name = name, "macOS: set app display name (Dock + app menu)");
}

pub(crate) unsafe fn set_macos_app_display_name() {
    set_macos_app_display_name_impl(true);
}

pub(crate) unsafe fn set_macos_app_display_name_pre_init() {
    set_macos_app_display_name_impl(false);
}

/// macOS accessibility activation governor — Layer 1 of
/// `docs/specs/SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03.md`.
///
/// Chromium enables its web-content accessibility tree the moment an AX client
/// sets `AXEnhancedUserInterface` on the application. That tree's macOS
/// implementation faults under external iteration on macOS-26 / CEF M114+
/// (CEF #3512: `AXPlatformNodeCocoa::AXChildren()` SEGV; here it surfaced as an
/// `EXC_BREAKPOINT` through the legacy `NSAccessibility…` accessor when a user
/// clicked the title-bar menu). The trigger attribute is **overloaded**:
/// VoiceOver sets it, but so do ordinary window managers / KVMs — Magnet,
/// Synergy — see Firefox bug 1664992. So a window manager merely doing its job
/// forces AgentMux into the crash-prone full-AX mode.
///
/// This swizzles the application's legacy `accessibilitySetValue:forAttribute:`
/// and applies a policy:
///   * `AXEnhancedUserInterface` (the window-manager/KVM path) does **not**
///     auto-enable web-content AX — unless `AGENTMUX_A11Y_HONOR_ENHANCED=1`.
///   * `AXManualAccessibility` (explicit assistive-technology / app intent —
///     the path Electron documents for enabling AX) **is** honored.
///   * every set is logged so the real activation path is observable in the
///     field (this is also how we confirm the fix against Magnet/Synergy).
///
/// Window-level AX (windows, buttons, title) is unaffected — only the descent
/// into the crash-prone web-content tree is gated. Full screen-reader support
/// returns unconditionally once the AX path itself is made non-fatal (Phase 2 /
/// Layer 2 of the spec). Not a blanket `--disable-renderer-accessibility`: that
/// would make AgentMux permanently inaccessible; this keeps the explicit
/// (`AXManualAccessibility`) enable path working.
///
/// Must run AFTER `cef::initialize` — the CEF `NSApplication` subclass that
/// implements the legacy AX setter only exists then. FFI mirrors
/// `disable_macos_drag_slideback`. Idempotent enough for once-at-startup.
pub(crate) unsafe fn install_macos_accessibility_governor() {
    use std::ffi::{c_char, c_void};

    type Id     = *mut c_void;
    type Sel    = *const c_void;
    type Class  = *mut c_void;
    type Method = *mut c_void;
    type Imp    = *const c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn object_getClass(obj: Id) -> Class;
        fn class_getName(cls: Class) -> *const c_char;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn class_getInstanceMethod(cls: Class, sel: Sel) -> Method;
        fn method_getImplementation(m: Method) -> Imp;
        fn method_setImplementation(m: Method, imp: Imp) -> Imp;
        fn objc_msgSend();
    }

    // Original Chromium IMP, saved so the governed replacement can chain to it.
    // Single-threaded startup write; main-thread-only AX reads afterward.
    static mut ORIGINAL_SET_AX: Imp = std::ptr::null();
    // Read the override once at install (env reads in the hot path are wasteful
    // and AX sets are rare, but a static keeps the IMP allocation-free).
    static mut HONOR_ENHANCED: bool = false;

    // [attr isEqualToString:@literal] without bringing in a string crate.
    unsafe fn attr_is(attr: Id, literal: &[u8]) -> bool {
        if attr.is_null() {
            return false;
        }
        let objc_get_class: extern "C" fn(*const c_char) -> Class =
            std::mem::transmute(objc_getClass as *const c_void);
        let cls = objc_get_class(b"NSString\0".as_ptr() as _);
        if cls.is_null() {
            return false;
        }
        let sel_with = sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _);
        let make: extern "C" fn(Class, Sel, *const c_char) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        let lit = make(cls, sel_with, literal.as_ptr() as *const c_char);
        if lit.is_null() {
            return false;
        }
        let sel_eq = sel_registerName(b"isEqualToString:\0".as_ptr() as _);
        let eq: extern "C" fn(Id, Sel, Id) -> u8 =
            std::mem::transmute(objc_msgSend as *const c_void);
        eq(attr, sel_eq, lit) != 0
    }

    // Replacement for -[<app> accessibilitySetValue:(id)value forAttribute:(NSString*)attr].
    unsafe extern "C" fn governed_set_ax(this: Id, cmd: Sel, value: Id, attribute: Id) {
        if attr_is(attribute, b"AXEnhancedUserInterface\0") {
            if !HONOR_ENHANCED {
                tracing::warn!(
                    "a11y governor: blocked AXEnhancedUserInterface activation \
                     (window-manager/KVM path — CEF #3512). \
                     Set AGENTMUX_A11Y_HONOR_ENHANCED=1 to allow."
                );
                return; // swallow → web-content AX stays off
            }
            tracing::warn!("a11y governor: honoring AXEnhancedUserInterface (override enabled)");
        } else if attr_is(attribute, b"AXManualAccessibility\0") {
            tracing::info!("a11y governor: honoring AXManualAccessibility (explicit enable)");
        } else {
            tracing::debug!("a11y governor: passthrough accessibilitySetValue:forAttribute:");
        }
        let orig: extern "C" fn(Id, Sel, Id, Id) = std::mem::transmute(ORIGINAL_SET_AX);
        orig(this, cmd, value, attribute);
    }

    // Honor only an explicit truthy value — keying on presence would make
    // `AGENTMUX_A11Y_HONOR_ENHANCED=0` *enable* the override (reagent P2).
    HONOR_ENHANCED = std::env::var("AGENTMUX_A11Y_HONOR_ENHANCED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // app = [NSApplication sharedApplication]
    let cls_nsapp = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls_nsapp.is_null() {
        tracing::warn!("a11y governor: NSApplication class not found; skipping");
        return;
    }
    let sel_shared = sel_registerName(b"sharedApplication\0".as_ptr() as _);
    let shared: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let app = shared(cls_nsapp, sel_shared);
    if app.is_null() {
        tracing::warn!("a11y governor: sharedApplication nil; skipping");
        return;
    }

    let app_cls = object_getClass(app);
    let cls_name = {
        let p = class_getName(app_cls);
        if p.is_null() {
            "<unknown>".to_owned()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };

    // --- Layer 2a guard (LOAD-BEARING): -[NSApplication accessibilityParent] → nil ---
    //
    // Installed FIRST and INDEPENDENTLY of the Layer 1 setter swizzle below, so a
    // macOS/CEF build that lacks the legacy setter still gets this fix (reagent P1
    // — the setter was previously a gate that skipped this guard on early return).
    //
    // The observed crash (EXC_BREAKPOINT, both reports) is an *incoming* AX READ —
    // an external client (Magnet/Synergy) calls CopyAttributeValue on the app,
    // which routes through:
    //   -[NSApplication accessibilityParent]
    //     → NSAccessibilityGetObjectForAttributeUsingLegacyAPI
    //     → NSAccessibilitySetUnsupportedAttributeError
    //     → +[NSString stringWithFormat:]  → CF string trap (with a CEF AX object
    //        as the %@ arg — CEF #3512).
    // NSApplication is the AX root; its parent is legitimately nil. Returning nil
    // DIRECTLY short-circuits before the crashy legacy bridge runs. Safe and
    // semantically correct, and it does not disable accessibility — windows/title
    // still answer.
    unsafe extern "C" fn accessibility_parent_nil(_this: Id, _cmd: Sel) -> Id {
        std::ptr::null_mut()
    }
    let sel_parent = sel_registerName(b"accessibilityParent\0".as_ptr() as _);
    let m_parent = class_getInstanceMethod(app_cls, sel_parent);
    if !m_parent.is_null() {
        method_setImplementation(m_parent, accessibility_parent_nil as Imp);
        tracing::info!(app_class = %cls_name, "a11y governor: guarded -[NSApplication accessibilityParent] → nil (SPEC L2a)");
    } else {
        tracing::warn!(app_class = %cls_name, "a11y governor: accessibilityParent not found on app class");
    }

    // --- Layer 1 (defense in depth): govern AXEnhancedUserInterface activation ---
    // Independent of L2a above; if the legacy setter is absent we just log and the
    // load-bearing parent guard still stands.
    let sel_set = sel_registerName(b"accessibilitySetValue:forAttribute:\0".as_ptr() as _);
    let method = class_getInstanceMethod(app_cls, sel_set);
    if !method.is_null() {
        ORIGINAL_SET_AX = method_getImplementation(method);
        method_setImplementation(method, governed_set_ax as Imp);
        tracing::info!(
            honor_enhanced = HONOR_ENHANCED,
            "a11y governor: swizzled accessibilitySetValue:forAttribute: (SPEC L1)"
        );
    } else {
        tracing::warn!(
            "a11y governor: accessibilitySetValue:forAttribute: not found on app class — \
             activation governor inactive (parent guard still installed)"
        );
    }
}

/// Promote the process to a regular, Dock-visible macOS app.
///
/// A bare Mach-O launched outside an `.app` bundle — both `task dev`
/// direct-invoke and the launcher's flat `dist/cef-dev/` layout — has no
/// `Info.plist`, so LaunchServices leaves it as a background/accessory
/// process: it can open windows but gets **no Dock tile and no menu bar**, so
/// the AgentMux instance is invisible in the taskbar (`lsappinfo` reports it
/// `type="BackgroundOnly"`). `-[NSApplication setActivationPolicy:]` with
/// `NSApplicationActivationPolicyRegular` (raw value `0`) flips it to a normal
/// foreground app so it shows in the Dock; this must run before
/// `set_macos_dock_icon` (the icon needs a tile to land on). Harmless in a
/// future packaged `.app` (already regular there). Idempotent.
///
/// Must run on the main thread after `cef::initialize` (NSApplication exists
/// by then). FFI mirrors `set_macos_dock_icon` — raw libobjc, no extra crate.
pub(crate) unsafe fn set_macos_activation_policy_regular() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    // NSApplicationActivationPolicyRegular == 0 (Accessory == 1, Prohibited == 2).
    const NS_ACTIVATION_POLICY_REGULAR: isize = 0;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    let cls_nsapp = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls_nsapp.is_null() {
        tracing::warn!("activation-policy: NSApplication class not found; skipping");
        return;
    }

    // NSApplication *app = [NSApplication sharedApplication]
    let sel_shared = sel_registerName(b"sharedApplication\0".as_ptr() as _);
    let msg_shared: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let app = msg_shared(cls_nsapp, sel_shared);
    if app.is_null() {
        tracing::warn!("activation-policy: NSApplication.sharedApplication unavailable");
        return;
    }

    // BOOL ok = [app setActivationPolicy:NSApplicationActivationPolicyRegular]
    let sel_set = sel_registerName(b"setActivationPolicy:\0".as_ptr() as _);
    let msg_set: extern "C" fn(Id, Sel, isize) -> u8 =
        std::mem::transmute(objc_msgSend as *const ());
    let ok = msg_set(app, sel_set, NS_ACTIVATION_POLICY_REGULAR);
    tracing::info!(ok = ok != 0, "activation-policy: set NSApplication to Regular (Dock-visible)");
}

/// Set the macOS Dock icon for the running process.
///
/// `task dev` launches the bare `agentmux-cef` Mach-O directly (not inside an
/// `.app` bundle — see `Taskfile.yml::dev:serve`), so macOS has no
/// `CFBundleIconFile` to read and shows the generic executable tile in the
/// Dock. Rather than restructure the dev launch around a bundle, we set the
/// icon at runtime via `-[NSApplication setApplicationIconImage:]`, which
/// works whether or not we're in a bundle and also overrides a bundle icon
/// in any future packaged build — one code path for all launch modes.
///
/// The PNG is embedded at compile time (`include_bytes!`) so there's no
/// dependency on the `dist/` layout or a resource-path lookup at runtime.
/// It's the SAME normal AgentMux logo the Linux taskbar uses
/// (`assets/linux/icons/hicolor/512x512/apps/agentmux.png`, wired up in
/// `scripts/install-linux-desktop.sh`), keeping the Dock/taskbar icon
/// identical across platforms.
///
/// Must run on the main thread after `cef::initialize` (NSApplication exists
/// by then). FFI mirrors `patch_nsapp_unrecognized_selector` — raw libobjc,
/// no `objc2`/`cocoa` crate dependency. The created NSImage is intentionally
/// leaked (one per process lifetime): `setApplicationIconImage:` retains it
/// and the icon lives as long as the app does.
pub(crate) unsafe fn set_macos_dock_icon() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    // The normal AgentMux logo (panel layout, not the brain-alternate),
    // matching the Linux taskbar source.
    const ICON_PNG: &[u8] =
        include_bytes!("../../assets/linux/icons/hicolor/512x512/apps/agentmux.png");

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        // objc_msgSend is declared bare and transmuted to the exact prototype
        // at each call site — the ARM64 calling convention requires the real
        // signature, not a variadic stand-in.
        fn objc_msgSend();
    }

    let cls_nsdata  = objc_getClass(b"NSData\0".as_ptr() as _);
    let cls_nsimage = objc_getClass(b"NSImage\0".as_ptr() as _);
    let cls_nsapp   = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls_nsdata.is_null() || cls_nsimage.is_null() || cls_nsapp.is_null() {
        tracing::warn!("dock-icon: an AppKit class was not found; skipping");
        return;
    }

    // NSData *data = [NSData dataWithBytes:ICON_PNG.ptr length:ICON_PNG.len]
    let sel_data = sel_registerName(b"dataWithBytes:length:\0".as_ptr() as _);
    let msg_data: extern "C" fn(Class, Sel, *const c_void, usize) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let data = msg_data(cls_nsdata, sel_data, ICON_PNG.as_ptr() as *const c_void, ICON_PNG.len());
    if data.is_null() {
        tracing::warn!("dock-icon: NSData creation failed");
        return;
    }

    // NSImage *img = [[NSImage alloc] initWithData:data]
    let sel_alloc = sel_registerName(b"alloc\0".as_ptr() as _);
    let msg_alloc: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let img_alloc = msg_alloc(cls_nsimage, sel_alloc);
    let sel_init = sel_registerName(b"initWithData:\0".as_ptr() as _);
    let msg_init: extern "C" fn(Id, Sel, Id) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let image = msg_init(img_alloc, sel_init, data);
    if image.is_null() {
        tracing::warn!("dock-icon: NSImage creation failed (corrupt PNG?)");
        return;
    }

    // NSApplication *app = [NSApplication sharedApplication]
    let sel_shared = sel_registerName(b"sharedApplication\0".as_ptr() as _);
    let msg_shared: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let app = msg_shared(cls_nsapp, sel_shared);
    if app.is_null() {
        tracing::warn!("dock-icon: NSApplication.sharedApplication unavailable");
        return;
    }

    // [app setApplicationIconImage:img]
    let sel_set = sel_registerName(b"setApplicationIconImage:\0".as_ptr() as _);
    let msg_set: extern "C" fn(Id, Sel, Id) =
        std::mem::transmute(objc_msgSend as *const ());
    msg_set(app, sel_set, image);

    tracing::info!("dock-icon: set NSApplication icon to embedded AgentMux logo");
}
