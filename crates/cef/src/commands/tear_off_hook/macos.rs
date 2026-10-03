// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ═══════════════════════════════════════════════════════════════════════
// macOS — CGEventTap-based cross-window cursor tracking for the in-strip
// tab-drag "redock" gesture (Windows' HookMode::TabDrag equivalent).
//
// See docs/specs/SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md for the full
// design writeup. Summary of the scope decision (§0.1 of that spec):
// Windows' TearOff mode / SC_MOVE-handshake live-follow tear-off is dead
// code on every platform today (superseded by a commit-on-release model —
// requestTearOff's skipScMove is always true from its one call site), so
// there is no live-follow tear-off window to track here — only an
// ordinary in-strip HTML5 drag whose cursor may cross into another
// AgentMux window. `start_tear_off_tracking` (TearOff mode) is
// deliberately NOT given a macOS body; it keeps using the shared
// not-Windows no-op stub in `mod.rs`.
//
// Threading discipline: the CGEventTap callback runs on a dedicated
// thread with its own CFRunLoop (mirrors the Windows hook thread's
// GetMessage pump). It must NEVER touch AppKit/NSWindow/CEF Views objects
// directly — those require the main thread. This is not a theoretical
// concern in this codebase: a real (pre-CEF-migration) crash came from
// exactly this mistake (AppKit calls off the main thread).
// Cross-window hit-testing therefore uses `CGWindowListCopyWindowInfo`, a
// Core Graphics *window-server query* API that never touches our own
// NSWindow objects and is thread-safe by design — the macOS analogue of
// Windows' WindowFromPoint (also a system query, not an app-object call).
// windowNumber→label resolution uses a small Mutex<HashMap> cache
// populated on the CEF UI thread at window-creation time (a one-time,
// main-thread-safe read of NSWindow.windowNumber via the existing
// objc_msgSend idiom — see app/mod.rs), read-only from the hook thread
// thereafter.
// ═══════════════════════════════════════════════════════════════════════

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, EventField,
};
use core_graphics::window::{
    copy_window_info, kCGWindowBounds, kCGWindowListOptionOnScreenOnly, kCGWindowNumber,
};

use crate::state::AppState;

/// macOS virtual keycode for Escape (`kVK_Escape`). Not exposed by
/// core-graphics — this is Apple's own stable HIToolbox constant.
const KVK_ESCAPE: i64 = 0x35;

thread_local! {
    static HOOK_CTX: RefCell<Option<MacHookContext>> = const { RefCell::new(None) };
}

/// TabDrag-mode-only context — see this module's doc comment for why
/// TearOff mode isn't ported. Field meanings mirror the Windows
/// `HookContext` fields of the same name.
struct MacHookContext {
    state: Arc<AppState>,
    source_label: String,
    tab_id: String,
    source_ws_id: String,
    is_last_tab: bool,
    current_target: RefCell<Option<String>>,
    finalized: RefCell<bool>,
    /// Throttle for `candidate_label_under_cursor_uncached`'s
    /// `CGWindowListCopyWindowInfo` call — see
    /// `HIT_TEST_MIN_INTERVAL`'s doc comment for why this exists.
    /// `(when the cached result was
    /// computed, that result)`.
    last_hit_test: RefCell<(std::time::Instant, Option<String>)>,
}

/// The currently-running hook session's run loop, if any — mirrors
/// Windows' `ACTIVE_HOOK_THREAD`. `CFRunLoopStop` is documented safe
/// to call from any thread, which is exactly how this is used (from
/// `stop_active_hook_session`, potentially called from a Tokio worker
/// thread via the IPC handler).
static ACTIVE_HOOK_RUNLOOP: Mutex<Option<CFRunLoop>> = Mutex::new(None);

/// Serializes `start_tab_drag_tracking` and `stop_active_hook_session`
/// against each other — distinct from `ACTIVE_HOOK_RUNLOOP` above,
/// which only guards concurrent *access* to the state, not the
/// *ordering* of start-vs-stop. Without this, `start_tab_drag_tracking`
/// (IPC-dispatched via `spawn_blocking`, taking real time to spawn a
/// thread and complete `CGEventTapCreate`) and `stop_active_hook_session`
/// (was dispatched synchronously, so much faster) had no ordering
/// guarantee relative to each other: a fast drag-then-immediate-release
/// could have stop's fast path run — and no-op, since
/// `ACTIVE_HOOK_RUNLOOP` isn't populated yet — before start's hook
/// thread finished installing, leaving a zombie hook alive to
/// misattribute a later, unrelated mouseup/Escape to this drag
/// (reagent PR #2310 P1, found by Codex). `start_tab_drag_tracking`
/// holds this for its entire critical section (session-takeover stop +
/// spawn + ready-wait); `stop_active_hook_session` blocks on it too, so
/// a stop that arrives mid-install simply waits for the install to
/// finish (and then correctly stops the now-installed hook) instead of
/// racing ahead of it.
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

/// windowNumber → AgentMux window label, populated on the CEF UI
/// thread (`app/mod.rs`'s `on_window_created`/`on_window_destroyed`)
/// and read-only from the hook thread. `kCGWindowNumber` in a
/// `CGWindowListCopyWindowInfo` result is documented by Apple to
/// equal the corresponding `NSWindow`'s `windowNumber` — the same
/// value cached here at window-creation time.
static WINDOW_LABELS_BY_NUMBER: Mutex<Option<HashMap<i64, String>>> = Mutex::new(None);

pub(crate) fn register_window_number(number: i64, label: String) {
    let mut g = WINDOW_LABELS_BY_NUMBER.lock().unwrap();
    g.get_or_insert_with(HashMap::new).insert(number, label);
}

pub(crate) fn unregister_window_label(label: &str) {
    if let Some(map) = WINDOW_LABELS_BY_NUMBER.lock().unwrap().as_mut() {
        map.retain(|_, v| v != label);
    }
}

#[allow(non_snake_case)]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

/// Silent Accessibility-permission check — no OS prompt (the prompt
/// option key is explicitly set to `false`). Gate this before ever
/// attempting `CGEventTapCreate`: an unauthorized tap can be created
/// successfully but never fire, which would otherwise manifest as a
/// silent, undebuggable "redock just doesn't work" bug.
/// See SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §2.4.
fn accessibility_trusted_silent() -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::false_value().as_CFType(),
        )]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
    }
}

/// Same check, but with the OS prompt enabled — triggers the real
/// System Settings → Privacy & Security → Accessibility dialog if not
/// already trusted. Only called once per process lifetime (guarded by
/// `PROMPTED_THIS_SESSION` below): calling this on every single drag
/// attempt while the user hasn't granted it yet would re-pop the OS
/// dialog on every drag, which is worse than not prompting at all.
///
/// This is a deliberately minimal stand-in for the full Phase 7c UX
/// (in-app explanation before the OS prompt, "already asked" persisted
/// across app launches, settings deep-link) — see
/// SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §2.4/§4. Built now,
/// ahead of that phase, because without SOME request path the feature
/// is silently inert and un-discoverable: `accessibility_trusted_silent`
/// alone never shows the user any way to grant the permission, so the
/// hook just never installs and nothing visibly differs from before.
fn accessibility_trusted_prompting() -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
    }
}

static PROMPTED_THIS_SESSION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The check `start_tab_drag_tracking` actually calls: silent first
/// (cheap, no dialog), and if untrusted, prompt exactly once per
/// process lifetime so the user has a real way to grant the
/// permission and try again on their next drag.
pub fn accessibility_trusted() -> bool {
    if accessibility_trusted_silent() {
        return true;
    }
    use std::sync::atomic::Ordering;
    if PROMPTED_THIS_SESSION.swap(true, Ordering::SeqCst) {
        // Already prompted this run — don't re-pop the dialog on
        // every subsequent drag while the user hasn't acted on it
        // (or has it open) yet.
        return false;
    }
    tracing::info!(
        target: "dnd:tabdrag:macos",
        "[dnd:tabdrag:macos] Accessibility not yet granted — triggering the OS permission prompt (first attempt this session)"
    );
    accessibility_trusted_prompting()
}

/// Does the actual stop, without acquiring `HOOK_LIFECYCLE_LOCK` itself
/// — callers that already hold it (namely `start_tab_drag_tracking`'s
/// session-takeover step) call this directly to avoid deadlocking on
/// their own lock. The public `stop_active_hook_session` below is a
/// thin wrapper that acquires the lock first.
fn stop_active_hook_session_locked() {
    let rl = { ACTIVE_HOOK_RUNLOOP.lock().map(|mut g| g.take()).unwrap_or(None) };
    if let Some(rl) = rl {
        rl.stop();
    }
}

/// Stop the active hook session, if any. Idempotent — mirrors the
/// Windows function of the same name. Called from the frontend's
/// dragend belt-and-suspenders `stop_tab_drag_tracking` IPC call.
/// Blocks on `HOOK_LIFECYCLE_LOCK` — see that static's doc comment for
/// why this matters (reagent PR #2310 P1): if a `start_tab_drag_tracking`
/// is mid-install (holding the lock), this waits for it to finish
/// rather than racing ahead and no-oping against not-yet-populated
/// state.
pub fn stop_active_hook_session() {
    let _guard = HOOK_LIFECYCLE_LOCK.lock();
    stop_active_hook_session_locked();
}

/// Install the CGEventTap for an ordinary in-strip tab drag (cross-
/// window tab remount). Mirrors Windows'
/// `start_tab_drag_tracking`/`HookMode::TabDrag` exactly at the IPC
/// contract level: same event names, same payload shapes, so the
/// frontend (`droppable-tab.tsx`, `tab-tearoff-events.ts`) needs no
/// changes.
///
/// Falls back to a silent no-op when Accessibility isn't granted —
/// the existing `CrossWindowDropOverlay` append-only cross-window
/// drag path (already shipped, works on macOS today) keeps working
/// exactly as it does now; this hook is a pure upgrade on top of it,
/// never a replacement it depends on.
pub fn start_tab_drag_tracking(
    state: Arc<AppState>,
    source_label: String,
    tab_id: String,
    source_ws_id: String,
    is_last_tab: bool,
) -> Result<(), String> {
    // Held for this entire function — see HOOK_LIFECYCLE_LOCK's doc
    // comment (reagent PR #2310 P1). Uses the _locked variant for the
    // session-takeover stop below to avoid deadlocking on this same
    // lock; stop_active_hook_session (the public one, called from the
    // separate stop_tab_drag_tracking IPC command) acquires it itself
    // and will correctly block here until this function returns.
    let _lifecycle_guard = HOOK_LIFECYCLE_LOCK.lock();

    // One session at a time, same as Windows.
    stop_active_hook_session_locked();

    if !accessibility_trusted() {
        tracing::warn!(
            target: "dnd:tabdrag:macos",
            "[dnd:tabdrag:macos] Accessibility permission not granted — skipping CGEventTap install; falling back to append-only cross-window drag"
        );
        return Ok(());
    }

    // Oneshot channel so the caller only returns once the tap is
    // actually installed and enabled — mirrors Windows' ready_tx/
    // ready_rx handshake, for the same reason (don't miss the first
    // few mouse events of the drag).
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    std::thread::Builder::new()
        .name("tab-drag-hook-macos".to_string())
        .spawn(move || {
            let ctx = MacHookContext {
                state,
                source_label,
                tab_id,
                source_ws_id,
                is_last_tab,
                current_target: RefCell::new(None),
                finalized: RefCell::new(false),
                // Backdated so the very first hit test always runs
                // immediately rather than waiting out the throttle.
                last_hit_test: RefCell::new((
                    std::time::Instant::now() - HIT_TEST_MIN_INTERVAL,
                    None,
                )),
            };
            HOOK_CTX.with(|cell| *cell.borrow_mut() = Some(ctx));

            // ListenOnly: this hook only observes, never intercepts —
            // returning None from the callback (below) always passes
            // the event through untouched, so the tab strip's own
            // HTML5 drag session and the OS's own event delivery are
            // completely unaffected by this tap's presence.
            let tap_result = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![
                    CGEventType::MouseMoved,
                    // Quartz reports pointer motion as LeftMouseDragged,
                    // not MouseMoved, whenever the left button is held —
                    // i.e. for the ENTIRE duration of a tab drag. Without
                    // this, handle_mouse_move never fired during the
                    // drag itself: only the final LeftMouseUp hit-test
                    // worked, so the live hover indicator this hook is
                    // supposed to drive never actually tracked the
                    // cursor (found by Codex, reagent PR #2310 P1).
                    CGEventType::LeftMouseDragged,
                    CGEventType::LeftMouseUp,
                    CGEventType::KeyDown,
                ],
                |_proxy: CGEventTapProxy, etype: CGEventType, event: &CGEvent| {
                    handle_tap_event(etype, event);
                    None
                },
            );

            let tap = match tap_result {
                Ok(t) => t,
                Err(_) => {
                    HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                    let _ = ready_tx.send(Err(
                        "CGEventTapCreate failed (unexpected — Accessibility was already \
                         confirmed granted)"
                            .to_string(),
                    ));
                    return;
                }
            };

            let loop_source = match tap.mach_port.create_runloop_source(0) {
                Ok(s) => s,
                Err(_) => {
                    HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                    let _ = ready_tx
                        .send(Err("CFMachPort create_runloop_source failed".to_string()));
                    return;
                }
            };

            let run_loop = CFRunLoop::get_current();
            run_loop.add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
            tap.enable();

            if let Ok(mut g) = ACTIVE_HOOK_RUNLOOP.lock() {
                *g = Some(run_loop.clone());
            }

            let _ = ready_tx.send(Ok(()));

            tracing::info!(
                target: "dnd:tabdrag:macos",
                "[dnd:tabdrag:macos] CGEventTap installed, entering run loop"
            );

            // Blocks until CFRunLoopStop is called — either by this
            // thread's own tap callback (mouseup / ESC) or by
            // stop_active_hook_session (session takeover / dragend
            // belt-and-suspenders).
            CFRunLoop::run_current();

            HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
            // Vacate the active-session slot — but only if it still
            // points at us (a superseding session may have already
            // replaced it). Mirrors the Windows thread-id comparison.
            if let Ok(mut g) = ACTIVE_HOOK_RUNLOOP.lock() {
                if g.as_ref() == Some(&run_loop) {
                    *g = None;
                }
            }

            tracing::info!(
                target: "dnd:tabdrag:macos",
                "[dnd:tabdrag:macos] run loop exited, thread exiting"
            );
        })
        .map_err(|e| format!("failed to spawn hook thread: {}", e))?;

    ready_rx
        .recv()
        .map_err(|e| format!("hook ready channel closed: {}", e))?
}

fn handle_tap_event(etype: CGEventType, event: &CGEvent) {
    match etype {
        // MouseMoved fires when no button is held; LeftMouseDragged
        // fires when the left button IS held — i.e. for a tab drag's
        // entire duration. Both drive the same hover hit-test.
        CGEventType::MouseMoved | CGEventType::LeftMouseDragged => handle_mouse_move(event),
        CGEventType::LeftMouseUp => {
            handle_button_up(event);
            CFRunLoop::get_current().stop();
        }
        CGEventType::KeyDown => {
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            if keycode == KVK_ESCAPE {
                // TabDrag mode: this hook doesn't own the underlying
                // native HTML5 drag session (pragmatic-drag-and-drop,
                // owned by the renderer) — it can't stop that drag by
                // itself. Originally this just retired the hook
                // session silently, on the assumption the native drag
                // would cancel itself on Escape. It doesn't: web DnD
                // gives browsers no such obligation, and empirically
                // (live testing) the tab still tore off / reordered
                // normally on release regardless of Escape.
                //
                // Fix: tell the SOURCE renderer explicitly via IPC
                // event, so its drop handler can skip the tear-off/
                // reorder decision at release time. A DOM-level
                // `keydown` listener was tried first and didn't work
                // either — Chromium's internal native-drag handling
                // appears to suppress normal input dispatch to the
                // page for the drag's duration. This CGEventTap sees
                // the raw OS-level HID keystroke instead, entirely
                // outside the renderer's own event pipeline, so it
                // isn't subject to that suppression.
                // See SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §5.
                HOOK_CTX.with(|cell| {
                    let ctx_ref = cell.borrow();
                    if let Some(ctx) = ctx_ref.as_ref() {
                        if *ctx.finalized.borrow() {
                            return;
                        }
                        *ctx.finalized.borrow_mut() = true;
                        tracing::info!(
                            target: "dnd:tabdrag:macos",
                            tab_id = %ctx.tab_id,
                            "[dnd:tabdrag:macos] ESC pressed — session aborted"
                        );
                        crate::events::emit_event_to_window(
                            &ctx.state,
                            &ctx.source_label,
                            "tabdrag:escape-pressed",
                            &serde_json::json!({ "tabId": ctx.tab_id }),
                        );
                        if let Some(target_label) = ctx.current_target.borrow().as_ref() {
                            crate::events::emit_event_to_window(
                                &ctx.state,
                                target_label,
                                "tearoff:hover-cleared",
                                &serde_json::json!({}),
                            );
                        }
                    }
                });
                CFRunLoop::get_current().stop();
            }
        }
        _ => {}
    }
}

fn handle_mouse_move(event: &CGEvent) {
    HOOK_CTX.with(|cell| {
        let ctx_ref = cell.borrow();
        let Some(ctx) = ctx_ref.as_ref() else {
            return;
        };
        let loc = event.location();
        let (cursor_x, cursor_y) = (loc.x, loc.y);

        // Throttled — see candidate_label_under_cursor_throttled's doc
        // comment. A few-ms-stale hover target is imperceptible for a
        // visual indicator; querying CGWindowListCopyWindowInfo on
        // every single MouseMoved tap event (up to ~120 Hz) is not
        // free, and doing so was a genuine, user-reported performance
        // regression during initial live testing.
        let candidate = candidate_label_under_cursor_throttled(ctx, cursor_x, cursor_y);
        let prev = ctx.current_target.borrow().clone();
        let candidate_changed = prev != candidate;

        if candidate_changed {
            if let Some(prev_label) = prev.as_ref() {
                crate::events::emit_event_to_window(
                    &ctx.state,
                    prev_label,
                    "tearoff:hover-cleared",
                    &serde_json::json!({}),
                );
            }
        }
        // Always emit hover-changed when over a candidate, not just
        // on candidate-change — the destination's insertion
        // indicator tracks cursor X continuously. Mirrors Windows'
        // handle_mouse_move exactly (reagent PR #565 P1 there).
        if let Some(cur_label) = candidate.as_ref() {
            crate::events::emit_event_to_window(
                &ctx.state,
                cur_label,
                "tearoff:hover-changed",
                &serde_json::json!({
                    "cursorX": cursor_x,
                    "cursorY": cursor_y,
                    "tabId": ctx.tab_id,
                }),
            );
        }
        if candidate_changed {
            *ctx.current_target.borrow_mut() = candidate;
        }
    });
}

fn handle_button_up(event: &CGEvent) {
    HOOK_CTX.with(|cell| {
        let ctx_ref = cell.borrow();
        let Some(ctx) = ctx_ref.as_ref() else {
            return;
        };
        if *ctx.finalized.borrow() {
            return;
        }
        *ctx.finalized.borrow_mut() = true;

        let loc = event.location();
        let (cursor_x, cursor_y) = (loc.x, loc.y);
        // Fresh, not throttled — this is a single one-off call (not a
        // per-move hot path) and it decides the actual merge outcome,
        // so correctness beats the sub-millisecond cost saved by
        // reusing a possibly-stale cached candidate.
        let candidate = candidate_label_under_cursor_uncached(ctx, cursor_x, cursor_y);

        tracing::info!(
            target: "dnd:tabdrag:macos",
            tab_id = %ctx.tab_id,
            cursor_x = %cursor_x,
            cursor_y = %cursor_y,
            target = ?candidate,
            "[dnd:tabdrag:macos] mouseup — finalize"
        );

        // The only outcome this session owns is a release over
        // another AgentMux window — emit tabdrag:merge-direct and
        // let that window strip-hit-test and move the tab. Release
        // over the source window (in-window reorder) or over
        // nothing (existing CrossWindowDropOverlay cross-window
        // append path) is owned by the existing pipelines; emitting
        // nothing here keeps them un-double-processed. Mirrors Windows'
        // handle_button_up TabDrag branch exactly.
        if let Some(target_label) = &candidate {
            if target_label != &ctx.source_label {
                crate::events::emit_event_to_window(
                    &ctx.state,
                    target_label,
                    "tabdrag:merge-direct",
                    &serde_json::json!({
                        "tabId": ctx.tab_id,
                        "fromWsId": ctx.source_ws_id,
                        "sourceWindowLabel": ctx.source_label,
                        "isLastTab": ctx.is_last_tab,
                        "cursorX": cursor_x,
                        "cursorY": cursor_y,
                    }),
                );
            }
        }
    });
}

/// Minimum interval between real `CGWindowListCopyWindowInfo` calls
/// from the mouse-move hot path (~30 Hz). Unlike Windows'
/// `WindowFromPoint` (an O(1) OS-maintained spatial index lookup —
/// genuinely cheap per call), `CGWindowListCopyWindowInfo` enumerates
/// and builds a full CFArray/CFDictionary description of every
/// on-screen window system-wide. Calling it unthrottled on every
/// `MouseMoved` tap event (up to ~120 Hz) was a real, user-reported
/// performance regression found during initial live testing — this
/// throttle is the fix, not a compromise: a stale-by-at-most-33ms
/// hover target is imperceptible for a visual indicator, so there is
/// no user-visible cost, only the CPU saved.
const HIT_TEST_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

/// Throttled hit test for the mouse-move hot path — reuses the last
/// result if it's fresher than `HIT_TEST_MIN_INTERVAL`, otherwise
/// re-runs `candidate_label_under_cursor_uncached` and refreshes the
/// cache. Do NOT use this for the mouseup finalize decision — see
/// `handle_button_up`'s call site for why that one stays uncached.
fn candidate_label_under_cursor_throttled(
    ctx: &MacHookContext,
    x: f64,
    y: f64,
) -> Option<String> {
    {
        let cached = ctx.last_hit_test.borrow();
        if cached.0.elapsed() < HIT_TEST_MIN_INTERVAL {
            return cached.1.clone();
        }
    }
    let fresh = candidate_label_under_cursor_uncached(ctx, x, y);
    *ctx.last_hit_test.borrow_mut() = (std::time::Instant::now(), fresh.clone());
    fresh
}

/// Point-in-rect hit test against our own on-screen windows via
/// `CGWindowListCopyWindowInfo` — see this module's doc comment for
/// why this is the thread-safe choice over any NSWindow-touching
/// API, and `HIT_TEST_MIN_INTERVAL`'s doc comment for why callers on
/// the mouse-move hot path go through the throttled wrapper instead
/// of calling this directly. Excludes the source window (mirrors
/// Windows' TabDrag-mode exclusion — its own pragmatic-dnd reorder
/// owns the strip while the cursor is over it; see the Windows
/// `candidate_label_under_cursor_locked`'s comment).
fn candidate_label_under_cursor_uncached(
    ctx: &MacHookContext,
    x: f64,
    y: f64,
) -> Option<String> {
    let labels_guard = WINDOW_LABELS_BY_NUMBER.lock().ok()?;
    let labels = labels_guard.as_ref()?;
    if labels.is_empty() {
        return None;
    }

    // `CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, ...)`
    // returns windows in FRONT-TO-BACK z-order (Apple's documented
    // behavior for this option). This matters: we need the TOPMOST
    // window whose bounds contain the cursor — any app, not just ours
    // — and only treat it as a redock candidate if that topmost window
    // happens to be one of our own. Originally this loop skipped
    // straight past any entry not in our `labels` map before ever
    // checking its bounds, which meant an AgentMux window whose bounds
    // contained the cursor but was visually COVERED by some other app's
    // window on top of it at that exact point would still be reported
    // as the candidate — occlusion was never considered. Fixed by
    // checking bounds for every entry, in z-order, and stopping at the
    // first (frontmost) one that contains the point, exactly mirroring
    // how Windows' `WindowFromPoint` inherently only ever returns the
    // single topmost HWND at a point (reagent PR #2310 P2).
    let info = copy_window_info(kCGWindowListOptionOnScreenOnly, 0)?;
    let count = info.len();
    for i in 0..count {
        let Some(item) = info.get(i) else { continue };
        // `copy_window_info` returns an untyped CFArray (element type
        // `*const c_void`); each element is actually a CFDictionary —
        // wrap it under the "get" rule (borrowed from the array, not
        // owned) to read it safely and typed.
        let dict: CFDictionary<CFType, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*item as CFDictionaryRef) };

        let Some(bounds_ref) = dict.find(unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) }.as_CFType()) else {
            continue;
        };
        // `CFDictionary<CFType, CFType>` isn't `ConcreteCFType` (only
        // the fully-untyped `CFDictionary<*const c_void, *const
        // c_void>` is), so `.downcast()` isn't available here —
        // reinterpret the raw ref directly instead. Safe: Apple
        // documents `kCGWindowBounds`'s value as itself a
        // CFDictionary (X/Y/Width/Height), and `wrap_under_get_rule`
        // borrows (retains without taking ownership) exactly like
        // `.downcast()` would have.
        let bounds_dict: CFDictionary<CFType, CFType> = unsafe {
            CFDictionary::wrap_under_get_rule(
                bounds_ref.as_concrete_TypeRef() as CFDictionaryRef
            )
        };
        let (Some(bx), Some(by), Some(bw), Some(bh)) = (
            cf_dict_number(&bounds_dict, "X"),
            cf_dict_number(&bounds_dict, "Y"),
            cf_dict_number(&bounds_dict, "Width"),
            cf_dict_number(&bounds_dict, "Height"),
        ) else {
            continue;
        };
        if !(x >= bx && x <= bx + bw && y >= by && y <= by + bh) {
            // This entry's bounds don't contain the cursor at all —
            // irrelevant regardless of z-order, keep scanning.
            continue;
        }

        // Found the FRONTMOST window (any app) whose bounds contain
        // the cursor. This is the one and only candidate check for
        // this hit test — if it isn't one of our own (non-source)
        // windows, some other window is occluding us here and there
        // is no valid redock candidate, full stop (do NOT keep
        // scanning further back for an AgentMux window that's
        // actually hidden behind this one).
        let Some(number_ref) = dict.find(unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) }.as_CFType()) else {
            return None;
        };
        let Some(number) = number_ref.downcast::<CFNumber>().and_then(|n| n.to_i64()) else {
            return None;
        };
        let Some(label) = labels.get(&number) else {
            return None;
        };
        if label == &ctx.source_label {
            return None;
        }
        return Some(label.clone());
    }
    None
}

fn cf_dict_number(dict: &CFDictionary<CFType, CFType>, key: &str) -> Option<f64> {
    dict.find(CFString::new(key).as_CFType())?
        .downcast::<CFNumber>()?
        .to_f64()
}
