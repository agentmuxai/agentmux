// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Native macOS startup splash, owned by the **launcher** — the macOS analogue
//! of the Win32 `splash.rs`, matching its look and effects:
//!
//! * a solid **dark backdrop** (RGB 26,26,31) with the brain logo centered on
//!   it (Win32 paints an opaque dark DIB; here a layer-backed `NSView`);
//! * the brain **pulses** (alpha fades in 0→1 over 200 ms, then a 1.1 Hz sine
//!   between ~0.73 and 1.0 — the backdrop stays solid);
//! * on first host frame the whole splash **fades out** (~160 ms) then leaves.
//!
//! Why the launcher and not the host: the launcher is a tiny binary up in
//! milliseconds, whereas `agentmux-cef` *is* the multi-second CEF load. This is
//! the only place a splash can paint *before* CEF initializes. The launcher
//! shows it in its own process (no AppKit/CEF runloop conflict); the host owns
//! its own runloop in the child process.
//!
//! Raw Objective-C runtime FFI (same approach as `agentmux-cef/src/main.rs`) so
//! the launcher pulls in no heavy new deps — `NSImage` decodes the bundled PNG.
//!
//! Dismiss protocol: `show()` sets `AGENTMUX_SPLASH_READY_FILE`; the host
//! inherits it and `write`s the file the moment CEF paints its first frame (see
//! `agentmux-cef/src/client/mod.rs`). `run_until_dismissed()` pumps a
//! CoreFoundation runloop on the main thread, animating the pulse and polling
//! for that file (with a safety timeout), then fades out and orders the window
//! away — and keeps the runloop turning afterward so the removal actually
//! flushes (the supervisor thread owns process lifetime).

#![cfg(target_os = "macos")]

use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::startup_events::{StartupEvent, StartupStatus};

// The brain logo (transparent PNG). NSImage decodes it natively at runtime.
static BRAIN_PNG: &[u8] = include_bytes!("../resources/brain.png");

const BRAIN_PX: f64 = 150.0; // displayed brain size
const PAD_PX: f64 = 35.0; // backdrop padding around the brain
const SPLASH_PX: f64 = BRAIN_PX + PAD_PX * 2.0; // 220×220 brain-region square
const CORNER_RADIUS: f64 = 16.0;

// Footer (identity strip near the bottom) — native NSTextField labels showing
// user@host / v<version> (+ dev label). SPEC_SPLASH_USERINFO_AND_DISABLE.
const FOOTER_LINE_H: f64 = 18.0;
const FOOTER_PAD: f64 = 10.0;
const FOOTER_BAND_H: f64 = FOOTER_PAD * 2.0 + FOOTER_LINE_H * 2.0; // 56
/// Full card: width = brain region; height = brain region + footer band.
const SPLASH_W: f64 = SPLASH_PX;
/// Brain region + stage-telemetry panel + footer. STAGE_AREA_H is defined
/// further below (stage-telemetry panel constants) — const items can
/// reference later ones in the same module, so declaration order here
/// doesn't matter.
const SPLASH_H: f64 = SPLASH_PX + STAGE_AREA_H + FOOTER_BAND_H;
// Muted footer text color #8A8A93.
const FOOTER_R: f64 = 0x8A as f64 / 255.0;
const FOOTER_G: f64 = 0x8A as f64 / 255.0;
const FOOTER_B: f64 = 0x93 as f64 / 255.0;

// Backdrop color — matches the Win32 splash's BG (R=0x1A, G=0x1A, B=0x1F).
const BG_R: f64 = 26.0 / 255.0;
const BG_G: f64 = 26.0 / 255.0;
const BG_B: f64 = 31.0 / 255.0;

// Darkened window-edge border — SPEC_SPLASH_SCREEN_BORDER_2026_08_25.md.
// Roughly half BG's RGB values, a straightforward "darker than the
// backdrop" reading; not yet confirmed against a real display.
const BORDER_R: f64 = 13.0 / 255.0;
const BORDER_G: f64 = 13.0 / 255.0;
const BORDER_B: f64 = 16.0 / 255.0;
const BORDER_WIDTH: f64 = 2.0;

/// Safety net: if the host never signals (crash before first paint), tear the
/// splash down anyway so it can't get stuck on screen.
const DISMISS_TIMEOUT: Duration = Duration::from_secs(10);
const FADE_OUT: f64 = 0.16; // seconds

// ── Startup-stage telemetry panel ───────────────────────────────────────────
// Renders StartupEvent stage/sub-item timing live, between the brain and the
// footer — the macOS counterpart of splash.rs's (Windows) software-blitted
// panel and splash_linux's StageList. Deliberately a separate, self-contained
// implementation rather than sharing Windows'/Linux's code: consolidating all
// three into one shared module is a real cleanup opportunity (flagged in
// SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md §B.4.3) but
// doing it in this PR would mean editing two already-working splashes I
// cannot build or run to verify — safer to add macOS purely additively here
// and do the consolidation as its own reviewable follow-up.
//
// Uses native NSTextField rows (retained-mode) instead of a software glyph
// blitter — matches the existing footer's implementation, and setStringValue:
// is a simpler live-update primitive than Windows' manual DIB compositing.
const MAX_STAGE_ROWS: usize = 12;
const STAGE_ROW_H: f64 = 16.0;
const STAGE_PAD_TOP: f64 = 8.0;
const STAGE_PAD_BOTTOM: f64 = 6.0;
const STAGE_MARGIN_L: f64 = 14.0;
const STAGE_MARGIN_R: f64 = 10.0;
const STAGE_AREA_H: f64 = STAGE_PAD_TOP + MAX_STAGE_ROWS as f64 * STAGE_ROW_H + STAGE_PAD_BOTTOM;
/// Split point between the name column and the right-aligned time column,
/// as a fraction of SPLASH_W.
const STAGE_NAME_FRAC: f64 = 0.60;
const STAGE_LABEL_MAX_CHARS: usize = 16;
const SUB_LABEL_MAX_CHARS: usize = 13;
const STAGE_INDENT: f64 = 14.0;

// Colors — same values as splash.rs (Windows) for visual consistency.
const STAGE_R: f64 = 0xC0 as f64 / 255.0;
const STAGE_G: f64 = 0xC0 as f64 / 255.0;
const STAGE_B: f64 = 0xCC as f64 / 255.0;
const TIME_DONE_R: f64 = 0x60 as f64 / 255.0;
const TIME_DONE_G: f64 = 0xCC as f64 / 255.0;
const TIME_DONE_B: f64 = 0x80 as f64 / 255.0;
const TIME_RUN_R: f64 = 0x70 as f64 / 255.0;
const TIME_RUN_G: f64 = 0x90 as f64 / 255.0;
const TIME_RUN_B: f64 = 0xFF as f64 / 255.0;
const SUB_R: f64 = 0x7A as f64 / 255.0;
const SUB_G: f64 = 0x7A as f64 / 255.0;
const SUB_B: f64 = 0x82 as f64 / 255.0;
const STATUS_OK_R: f64 = 0x44 as f64 / 255.0;
const STATUS_OK_G: f64 = 0xBB as f64 / 255.0;
const STATUS_OK_B: f64 = 0x44 as f64 / 255.0;
const STATUS_WARN_R: f64 = 0xCC as f64 / 255.0;
const STATUS_WARN_G: f64 = 0xAA as f64 / 255.0;
const STATUS_WARN_B: f64 = 0x33 as f64 / 255.0;
const STATUS_ERR_R: f64 = 0xCC as f64 / 255.0;
const STATUS_ERR_G: f64 = 0x44 as f64 / 255.0;
const STATUS_ERR_B: f64 = 0x44 as f64 / 255.0;

struct SubRow {
    id: String,
    label: String,
    started_at: Instant,
    done: Option<(u64, StartupStatus, Option<String>)>,
}

struct StageRow {
    stage: &'static str,
    label: &'static str,
    started_at: Instant,
    done: Option<(u64, StartupStatus, Option<String>)>,
    subs: Vec<SubRow>,
}

/// Identifies which row a `Begin` event created, so `run_until_dismissed`'s
/// drain loop can recognize when a matching `End` arrives for a row that was
/// *itself* created in this same drain pass — see the count-up note there.
#[derive(PartialEq)]
enum BeginKey {
    Stage(&'static str),
    Sub(&'static str, String),
}

/// Same state machine as splash.rs's (Windows) `apply_event` /
/// splash_linux's `StageList::apply` — see the module doc above for why this
/// isn't shared code yet.
fn apply_event(stages: &mut Vec<StageRow>, ev: StartupEvent) {
    match ev {
        StartupEvent::StageBegin { stage, label } => {
            stages.push(StageRow {
                stage,
                label,
                started_at: Instant::now(),
                done: None,
                subs: Vec::new(),
            });
        }
        StartupEvent::StageEnd { stage, duration_ms, status, detail } => {
            if let Some(row) = stages.iter_mut().rev().find(|r| r.stage == stage) {
                row.done = Some((duration_ms, status, detail));
            }
        }
        StartupEvent::SubBegin { stage, id, label } => {
            if let Some(row) = stages.iter_mut().rev().find(|r| r.stage == stage) {
                row.subs.push(SubRow {
                    id,
                    label,
                    started_at: Instant::now(),
                    done: None,
                });
            }
        }
        StartupEvent::SubEnd { stage, id, duration_ms, status, detail } => {
            if let Some(row) = stages.iter_mut().rev().find(|r| r.stage == stage) {
                if let Some(sub) = row.subs.iter_mut().rev().find(|s| s.id == id) {
                    sub.done = Some((duration_ms, status, detail));
                }
            }
        }
    }
}

/// Applies one tick's worth of already-fetched events to `stages`. Returns
/// `true` if anything changed. Pulled out of `run_until_dismissed` as a pure
/// function (taking a plain `Vec<StartupEvent>` instead of draining
/// `self.startup_rx` directly) so the deferral logic is unit-testable
/// without a live splash window.
///
/// A row is created `done: None` on Begin and only shows a live "running"
/// time once a render happens before its End is applied (`flatten_rows`). If
/// a step finishes fast enough that both its Begin and End are already
/// queued by the time a tick drains, they'd otherwise be applied back-to-back
/// in this same pass — the row is created already-done and no "running"
/// frame is ever painted, so it just snaps to its final value instead of
/// visibly counting up (docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md
/// §2). Fix: apply anything deferred from the *previous* tick first
/// (guaranteeing its row spent at least one tick — and one render, since
/// `run_until_dismissed` always redraws while `ready_at.is_none()` — in the
/// "running" state), then apply this tick's fresh events, holding back any
/// End whose matching Begin was *also* seen in this same batch rather than
/// applying it immediately.
fn apply_tick(
    stages: &mut Vec<StageRow>,
    deferred: &mut Vec<StartupEvent>,
    fresh: Vec<StartupEvent>,
) -> bool {
    let mut changed = false;
    for ev in deferred.drain(..) {
        apply_event(stages, ev);
        changed = true;
    }
    let mut began_this_tick: Vec<BeginKey> = Vec::new();
    for ev in fresh {
        let defer = match &ev {
            StartupEvent::StageEnd { stage, .. } => {
                began_this_tick.contains(&BeginKey::Stage(stage))
            }
            StartupEvent::SubEnd { stage, id, .. } => {
                began_this_tick.contains(&BeginKey::Sub(stage, id.clone()))
            }
            _ => false,
        };
        if defer {
            deferred.push(ev);
            continue;
        }
        match &ev {
            StartupEvent::StageBegin { stage, .. } => {
                began_this_tick.push(BeginKey::Stage(stage));
            }
            StartupEvent::SubBegin { stage, id, .. } => {
                began_this_tick.push(BeginKey::Sub(stage, id.clone()));
            }
            _ => {}
        }
        apply_event(stages, ev);
        changed = true;
    }
    changed
}

fn trunc(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect::<String>() + ".."
    }
}

fn format_ms(ms: u64) -> String {
    if ms >= 10_000 {
        format!("{:.0}s", ms as f64 / 1000.0)
    } else if ms >= 1_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}ms", ms)
    }
}

fn format_running(started_at: Instant) -> String {
    let s = started_at.elapsed().as_secs_f32();
    if s >= 10.0 {
        format!("> {:.0}s", s)
    } else {
        format!("> {:.1}s", s)
    }
}

/// One flattened display row: (indent, label, time_text, label_color,
/// time_color) — computed fresh each tick from the current `Vec<StageRow>`.
struct FlatRow {
    indented: bool,
    label: String,
    time_text: String,
    label_color: (f64, f64, f64),
    time_color: (f64, f64, f64),
}

fn flatten_rows(stages: &[StageRow], total_ms: Option<u64>) -> Vec<FlatRow> {
    let mut out = Vec::new();
    for stage in stages {
        if out.len() >= MAX_STAGE_ROWS {
            break;
        }
        let (time_text, time_color) = match &stage.done {
            Some((ms, _status, _detail)) => (format_ms(*ms), (TIME_DONE_R, TIME_DONE_G, TIME_DONE_B)),
            None => (format_running(stage.started_at), (TIME_RUN_R, TIME_RUN_G, TIME_RUN_B)),
        };
        out.push(FlatRow {
            indented: false,
            label: trunc(stage.label, STAGE_LABEL_MAX_CHARS),
            time_text,
            label_color: (STAGE_R, STAGE_G, STAGE_B),
            time_color,
        });
        for sub in &stage.subs {
            if out.len() >= MAX_STAGE_ROWS {
                break;
            }
            // NSTextField is one color per field — combine the status glyph
            // into the time text and color the whole thing by status once
            // done (Windows shows the glyph as a separately-colored swatch
            // next to a SUB_COLOR time; that needs a 3rd field per row,
            // which isn't worth the extra layout complexity here).
            let (time_text, time_color) = match &sub.done {
                Some((ms, status, _detail)) => {
                    let glyph = match status {
                        StartupStatus::Ok => "+",
                        StartupStatus::Warn => "!",
                        StartupStatus::Error => "X",
                    };
                    let color = match status {
                        StartupStatus::Ok => (STATUS_OK_R, STATUS_OK_G, STATUS_OK_B),
                        StartupStatus::Warn => (STATUS_WARN_R, STATUS_WARN_G, STATUS_WARN_B),
                        StartupStatus::Error => (STATUS_ERR_R, STATUS_ERR_G, STATUS_ERR_B),
                    };
                    (format!("{} {}", format_ms(*ms), glyph), color)
                }
                None => (format_running(sub.started_at), (TIME_RUN_R, TIME_RUN_G, TIME_RUN_B)),
            };
            out.push(FlatRow {
                indented: true,
                label: trunc(&sub.label, SUB_LABEL_MAX_CHARS),
                time_text,
                label_color: (SUB_R, SUB_G, SUB_B),
                time_color,
            });
        }
    }
    if let Some(ms) = total_ms {
        // "other" — the gap between `total` (a wall-clock stopwatch running
        // from splash-window creation to first-paint-detected) and the sum
        // of top-level stage durations. Only completed (`done`) stages are
        // summed: they're already exclusive of each other (a stage's own
        // duration already covers whatever its subs took), and an
        // undone stage would inflate the sum with an ever-changing partial
        // count. Any residual is genuinely uncovered by any instrumented
        // stage — e.g. cef_init-end -> first-paint, or process-spawn
        // scheduling gaps between stages — not a double-count or a bug; see
        // docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md §5.
        // saturating_sub guards the (should-be-impossible, but not worth a
        // panic over) case where accounted time exceeds total_ms.
        let accounted_ms: u64 = stages
            .iter()
            .filter_map(|s| s.done.as_ref().map(|(dur, _, _)| *dur))
            .sum();
        let other_ms = ms.saturating_sub(accounted_ms);
        // "total" is the existing, higher-priority row: only add "other"
        // when there's room for both, so a nearly-full panel still shows the
        // total rather than silently dropping it for the new row.
        if out.len() + 2 <= MAX_STAGE_ROWS {
            out.push(FlatRow {
                indented: false,
                label: String::new(),
                time_text: format!("other: {}", format_ms(other_ms)),
                label_color: (SUB_R, SUB_G, SUB_B),
                time_color: (SUB_R, SUB_G, SUB_B),
            });
        }
        if out.len() < MAX_STAGE_ROWS {
            out.push(FlatRow {
                indented: false,
                label: String::new(),
                time_text: format!("total: {}", format_ms(ms)),
                label_color: (SUB_R, SUB_G, SUB_B),
                time_color: (SUB_R, SUB_G, SUB_B),
            });
        }
    }
    out
}

#[allow(non_camel_case_types)]
type id = *mut std::ffi::c_void;
type Class = *const std::ffi::c_void;
type SEL = *const std::ffi::c_void;

const NIL: id = std::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type CFStringRef = *const std::ffi::c_void;

#[link(name = "AppKit", kind = "framework")]
#[link(name = "Foundation", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
#[link(name = "QuartzCore", kind = "framework")]
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> SEL;
    fn objc_msgSend();
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source_handled: u8) -> i32;
}

#[inline]
unsafe fn class(name: &[u8]) -> id {
    // Cast Class (*const) to id (*mut) so it can be a `send` receiver — both
    // are just pointers to ObjC objects as far as objc_msgSend is concerned.
    objc_getClass(name.as_ptr() as *const c_char) as id
}
#[inline]
unsafe fn sel(name: &[u8]) -> SEL {
    sel_registerName(name.as_ptr() as *const c_char)
}

#[inline]
unsafe fn send(recv: id, s: SEL) -> id {
    let f: extern "C" fn(id, SEL) -> id = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
#[inline]
unsafe fn send_void(recv: id, s: SEL) {
    let f: extern "C" fn(id, SEL) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
#[inline]
unsafe fn send_id(recv: id, s: SEL, a: id) -> id {
    let f: extern "C" fn(id, SEL, id) -> id = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
#[inline]
unsafe fn send_void_id(recv: id, s: SEL, a: id) {
    let f: extern "C" fn(id, SEL, id) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
#[inline]
unsafe fn send_void_bool(recv: id, s: SEL, a: u8) {
    let f: extern "C" fn(id, SEL, u8) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
#[inline]
unsafe fn send_void_i64(recv: id, s: SEL, a: i64) {
    let f: extern "C" fn(id, SEL, i64) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
#[inline]
unsafe fn send_void_f64(recv: id, s: SEL, a: f64) {
    let f: extern "C" fn(id, SEL, f64) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
#[inline]
unsafe fn send_id_f64(recv: id, s: SEL, a: f64) -> id {
    let f: extern "C" fn(id, SEL, f64) -> id = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
#[inline]
unsafe fn send_void_rect(recv: id, s: SEL, a: CGRect) {
    let f: extern "C" fn(id, SEL, CGRect) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a)
}
/// `[NSString stringWithUTF8String:]` — copies, so the temporary CString is fine.
unsafe fn nsstring(s: &str) -> id {
    let cstr = std::ffi::CString::new(s).unwrap_or_default();
    let f: extern "C" fn(id, SEL, *const c_char) -> id =
        std::mem::transmute(objc_msgSend as *const ());
    f(
        class(b"NSString\0"),
        sel(b"stringWithUTF8String:\0"),
        cstr.as_ptr(),
    )
}

/// `[NSColor colorWithSRGBRed:green:blue:alpha:1.0]` — autoreleased, so
/// callers outside build_window()'s explicit pool must run inside their own
/// (see update_stage_fields).
unsafe fn color(r: f64, g: f64, b: f64) -> id {
    let f: extern "C" fn(id, SEL, f64, f64, f64, f64) -> id =
        std::mem::transmute(objc_msgSend as *const ());
    f(
        class(b"NSColor\0"),
        sel(b"colorWithSRGBRed:green:blue:alpha:\0"),
        r,
        g,
        b,
        1.0,
    )
}

// -----------------------------------------------------------------------------
// Reopen handler (Finder double-click / `open` without -n of a RUNNING instance).
//
// On macOS the launcher is `CFBundleExecutable`, so LaunchServices delivers the
// reopen Apple Event to THIS process (the host — a child ASN that owns the Dock
// tile — only receives the Dock-click reopen; see
// docs/specs/SPEC_MACOS_REOPEN_NEW_WINDOW_2026_06_22.md). The launcher can't open
// a CEF window itself, so it forwards `open_new_window` to the running host over
// the single-instance socket this instance already bound. Without this handler
// the reopen is unhandled and macOS shows "AgentMux is not responding".
// -----------------------------------------------------------------------------

/// `(data_dir, dir_hash)` of THIS instance's bound single-instance socket,
/// published by the supervisor (`run_unix`) the moment it wins the bind. The
/// reopen handler forwards to exactly this socket and never recomputes it — so a
/// leaked `AGENTMUX_CHANNEL` or `AGENTMUX_IPC_VERSION_OVERRIDE` can't misroute the
/// forward to the wrong (channel, version) instance.
static REOPEN_TARGET: OnceLock<(PathBuf, String)> = OnceLock::new();

/// Publish this launcher's bound-socket identity to the reopen handler. Called
/// once from the supervisor thread after `bind_socket_with_recovery` wins the
/// socket; no-op if already set.
pub fn set_reopen_target(data_dir: PathBuf, dir_hash: String) {
    let _ = REOPEN_TARGET.set((data_dir, dir_hash));
}

/// `-(BOOL)applicationShouldHandleReopen:(NSApplication*)sender
/// hasVisibleWindows:(BOOL)flag` — type encoding `c@:@c` (BOOL = signed char).
/// Always opens a NEW window (forward `open_new_window`), regardless of
/// `hasVisibleWindows`. Returns NO so AppKit doesn't also run its default reopen.
unsafe extern "C" fn should_handle_reopen(
    _self: id,
    _cmd: SEL,
    _app: id,
    _has_visible_windows: u8,
) -> u8 {
    match REOPEN_TARGET.get() {
        Some((data_dir, dir_hash)) => {
            crate::log("reopen-hook:fired proc=launcher — forwarding open_new_window");
            crate::second_instance::forward_open_new_window_or_log(data_dir, dir_hash);
        }
        // Reopen fired before the socket was bound (host still starting): the
        // first window is already on its way, so do nothing (SPEC §6.4).
        None => crate::log("reopen-hook:fired proc=launcher — ignored (socket not bound yet)"),
    }
    0 // NO — handled
}

/// Install `applicationShouldHandleReopen:hasVisibleWindows:` on the splash's
/// `NSApplication`. The launcher's splash NSApp has no delegate, so we install a
/// dedicated one; if one ever exists we add/override the method onto its class
/// (the Chromium-proof technique the host uses in `agentmux-cef::macos_menu`).
/// MUST run on the main thread after the NSApplication exists.
unsafe fn install_reopen_handler() {
    extern "C" {
        fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
        fn objc_registerClassPair(cls: Class);
        fn class_addMethod(cls: Class, sel: SEL, imp: usize, types: *const c_char) -> u8;
        fn object_getClass(obj: id) -> Class;
        fn class_getInstanceMethod(cls: Class, sel: SEL) -> *mut std::ffi::c_void;
        fn method_setImplementation(
            m: *mut std::ffi::c_void,
            imp: *const std::ffi::c_void,
        ) -> *const std::ffi::c_void;
    }

    let imp: unsafe extern "C" fn(id, SEL, id, u8) -> u8 = should_handle_reopen;
    let sel_reopen = sel(b"applicationShouldHandleReopen:hasVisibleWindows:\0");
    let types = b"c@:@c\0".as_ptr() as *const c_char;

    let app = send(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
    if app.is_null() {
        crate::log("reopen-hook: sharedApplication nil; launcher handler not installed");
        return;
    }
    let current_delegate = send(app, sel(b"delegate\0"));
    if current_delegate.is_null() {
        let name = b"AgentMuxLauncherReopenDelegate\0";
        let mut dcls = objc_getClass(name.as_ptr() as *const c_char);
        if dcls.is_null() {
            let superclass = objc_getClass(b"NSObject\0".as_ptr() as *const c_char);
            dcls = objc_allocateClassPair(superclass, name.as_ptr() as *const c_char, 0);
            class_addMethod(dcls, sel_reopen, imp as usize, types);
            objc_registerClassPair(dcls);
        }
        // alloc/init returns a +1-retained object we deliberately never release —
        // NSApplication's `delegate` is a WEAK reference, so the leak keeps it
        // valid for the process lifetime.
        let delegate = send(send(dcls as id, sel(b"alloc\0")), sel(b"init\0"));
        send_void_id(app, sel(b"setDelegate:\0"), delegate);
        crate::log("reopen-hook: installed dedicated launcher reopen delegate");
    } else {
        // class_addMethod adds an override on THIS class only when the selector
        // isn't already implemented directly; otherwise replace its own IMP.
        let cls = object_getClass(current_delegate);
        if class_addMethod(cls, sel_reopen, imp as usize, types) != 0 {
            crate::log("reopen-hook: added applicationShouldHandleReopen to existing launcher delegate");
        } else {
            let m = class_getInstanceMethod(cls, sel_reopen);
            if !m.is_null() {
                method_setImplementation(m, imp as *const std::ffi::c_void);
                crate::log("reopen-hook: swizzled applicationShouldHandleReopen on existing launcher delegate");
            }
        }
    }
}

/// A live splash window plus the path the host touches when it's ready.
pub struct Splash {
    window: id,
    image_view: id,
    ready_file: PathBuf,
    /// Pre-allocated (label_field, time_field) NSTextField pairs, one per
    /// potential stage/sub-item row — hidden/empty until an event fills them.
    stage_fields: Vec<(id, id)>,
    startup_rx: Receiver<StartupEvent>,
}

impl Splash {
    /// Create the `AGENTMUX_SPLASH_READY_FILE` env path and the window, paint
    /// it, and return the handle. MUST be called on the process main thread,
    /// before the supervisor thread spawns the host (so the host inherits the
    /// env var). `startup_rx` delivers `StartupEvent`s from the supervisor
    /// worker thread — drained each tick in `run_until_dismissed`.
    pub fn show(startup_rx: Receiver<StartupEvent>) -> Splash {
        let ready_file =
            std::env::temp_dir().join(format!("agentmux-splash-ready-{}", std::process::id()));
        let _ = std::fs::remove_file(&ready_file);
        // Inherited by the host spawned later on the supervisor thread.
        std::env::set_var("AGENTMUX_SPLASH_READY_FILE", &ready_file);

        // NSAutoreleasePool ensures autoreleased objects (NSData, NSImage,
        // NSColor, NSWindow, NSView) created in build_window() are drained
        // before we return. Without it, the pool from the thread's implicit
        // NSApplication runloop hasn't been set up yet and autorelease
        // messages queue to a nil pool — leaking on pre-macOS-12 targets.
        let (window, image_view, stage_fields) = unsafe {
            let pool = send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0"));
            let pool = send(pool, sel(b"init\0"));
            let result = build_window();
            // NSApplication now exists — install the reopen handler so a Finder
            // double-click / `open` of the running app forwards a new window
            // (SPEC_MACOS_REOPEN_NEW_WINDOW_2026_06_22.md).
            install_reopen_handler();
            send_void(pool, sel(b"drain\0"));
            result
        };
        Splash {
            window,
            image_view,
            ready_file,
            stage_fields,
            startup_rx,
        }
    }

    /// Dev affordance (used by `--splash-selftest` + `AGENTMUX_SPLASH_DUMP_PNG`):
    /// pump the runloop briefly — draining any pending startup events into
    /// the stage panel along the way, same as run_until_dismissed's tick —
    /// so layout/draw settle, then render the splash's content view to a PNG
    /// at `path`. Lets us eyeball footer + stage-panel layout without Screen
    /// Recording permission. Offscreen `cacheDisplayInRect:` only.
    pub fn dump_png(&self, path: &str) {
        let mut stages: Vec<StageRow> = Vec::new();
        for _ in 0..60 {
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.016, 0);
            }
            let mut changed = false;
            while let Ok(ev) = self.startup_rx.try_recv() {
                apply_event(&mut stages, ev);
                changed = true;
            }
            if changed {
                unsafe {
                    self.update_stage_fields(&stages, None);
                }
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        unsafe { dump_window_png(self.window, path) };
    }

    /// Refresh the pre-allocated stage_fields rows from the current stage
    /// list. Runs every tick while the host is still starting, so its
    /// NSString/NSColor allocations are wrapped in their own autorelease
    /// pool — show()'s explicit pool has already drained by the time this
    /// runs, and this loop's manual `pump_app_events` (unlike a real
    /// `-[NSApplication run]`) doesn't establish a per-iteration pool itself.
    unsafe fn update_stage_fields(&self, stages: &[StageRow], total_ms: Option<u64>) {
        let pool = send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0"));
        let pool = send(pool, sel(b"init\0"));

        let rows = flatten_rows(stages, total_ms);
        for (i, (name_field, time_field)) in self.stage_fields.iter().enumerate() {
            match rows.get(i) {
                Some(row) => {
                    if row.indented {
                        let cur: CGRect = {
                            let f: extern "C" fn(id, SEL) -> CGRect =
                                std::mem::transmute(objc_msgSend as *const ());
                            f(*name_field, sel(b"frame\0"))
                        };
                        let indented = CGRect {
                            origin: CGPoint { x: STAGE_MARGIN_L + STAGE_INDENT, y: cur.origin.y },
                            size: cur.size,
                        };
                        send_void_rect(*name_field, sel(b"setFrame:\0"), indented);
                    } else {
                        let cur: CGRect = {
                            let f: extern "C" fn(id, SEL) -> CGRect =
                                std::mem::transmute(objc_msgSend as *const ());
                            f(*name_field, sel(b"frame\0"))
                        };
                        let flush = CGRect {
                            origin: CGPoint { x: STAGE_MARGIN_L, y: cur.origin.y },
                            size: cur.size,
                        };
                        send_void_rect(*name_field, sel(b"setFrame:\0"), flush);
                    }
                    send_void_id(*name_field, sel(b"setStringValue:\0"), nsstring(&row.label));
                    let (lr, lg, lb) = row.label_color;
                    send_void_id(*name_field, sel(b"setTextColor:\0"), color(lr, lg, lb));
                    send_void_bool(*name_field, sel(b"setHidden:\0"), 0);

                    send_void_id(*time_field, sel(b"setStringValue:\0"), nsstring(&row.time_text));
                    let (tr, tg, tb) = row.time_color;
                    send_void_id(*time_field, sel(b"setTextColor:\0"), color(tr, tg, tb));
                    send_void_bool(*time_field, sel(b"setHidden:\0"), 0);
                }
                None => {
                    send_void_bool(*name_field, sel(b"setHidden:\0"), 1);
                    send_void_bool(*time_field, sel(b"setHidden:\0"), 1);
                }
            }
        }

        send_void(pool, sel(b"drain\0"));
    }

    /// Pump the splash runloop on the main thread: animate the brain pulse,
    /// drain startup events into the stage panel, and once the host signals
    /// first paint (ready-file appears) — or the safety timeout elapses —
    /// hold on the completed timeline for AGENTMUX_SPLASH_HOLD_MS (mirrors
    /// splash.rs's Windows hold, default 3000ms / capped at 1000ms for very
    /// fast starts), then fade the whole splash out and order it away. Then
    /// keep the runloop turning (so the removal flushes) until the
    /// supervisor thread exits the process.
    pub fn run_until_dismissed(self) {
        let start = Instant::now();
        let mut fade_start: Option<Instant> = None;
        let mut ready_at: Option<Instant> = None;
        let mut hold_duration = Duration::ZERO;
        let mut total_ms: u64 = 0;
        let mut stages: Vec<StageRow> = Vec::new();
        // Events held back from a tick where their matching Begin *also*
        // landed, applied at the start of the next tick instead — see the
        // count-up note in the drain loop below.
        let mut deferred: Vec<StartupEvent> = Vec::new();

        loop {
            // NSApp event pump (not bare CFRunLoop) so a reopen Apple Event that
            // arrives mid-splash is delivered to our delegate. See pump_app_events.
            unsafe {
                pump_app_events(0.016);
            }

            // Drain pending startup events (non-blocking) into the stage
            // list. See `apply_tick`'s doc comment for why this isn't a
            // plain "apply everything as it arrives" drain.
            let mut fresh = Vec::new();
            while let Ok(ev) = self.startup_rx.try_recv() {
                fresh.push(ev);
            }
            let mut changed = apply_tick(&mut stages, &mut deferred, fresh);

            let t = start.elapsed().as_secs_f64();

            // Brain pulse: fade in 0→1 over 200 ms, then 1.1 Hz sine 0.73..1.0
            // (the Win32 splash pulses 160..220/255 ≈ the same band).
            let brain_alpha = if t < 0.2 {
                t / 0.2
            } else {
                let pulse = ((t - 0.2) * std::f64::consts::TAU * 1.1).sin() * 0.5 + 0.5;
                0.73 + pulse * 0.27
            };
            unsafe {
                send_void_f64(self.image_view, sel(b"setAlphaValue:\0"), brain_alpha);
            }

            // Detect first-paint (or timeout): capture total elapsed and
            // compute the hold duration once, same convention as Windows'
            // AGENTMUX_SPLASH_HOLD_MS handling.
            if ready_at.is_none()
                && (self.ready_file.exists() || start.elapsed() > DISMISS_TIMEOUT)
            {
                ready_at = Some(Instant::now());
                let _ = std::fs::remove_file(&self.ready_file);
                total_ms = start.elapsed().as_millis() as u64;
                let hold_ms = std::env::var("AGENTMUX_SPLASH_HOLD_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2000);
                let hold_ms = if total_ms < 500 { hold_ms.min(1000) } else { hold_ms };
                hold_duration = Duration::from_millis(hold_ms);
                changed = true; // force one final refresh showing the "total:" row
            }

            // Refresh the stage panel: live every tick while still running
            // (so the running-time counters visibly tick up), or only on
            // change once frozen for the hold.
            if changed || ready_at.is_none() {
                let total_for_display = ready_at.map(|_| total_ms);
                unsafe {
                    self.update_stage_fields(&stages, total_for_display);
                }
            }

            // Start the fade once the hold has elapsed.
            if let Some(r0) = ready_at {
                if fade_start.is_none() && r0.elapsed() >= hold_duration {
                    fade_start = Some(Instant::now());
                }
            }

            if let Some(f0) = fade_start {
                let p = (f0.elapsed().as_secs_f64() / FADE_OUT).min(1.0);
                unsafe {
                    send_void_f64(self.window, sel(b"setAlphaValue:\0"), 1.0 - p);
                }
                if p >= 1.0 {
                    unsafe {
                        send_void_id(self.window, sel(b"orderOut:\0"), NIL);
                    }
                    break;
                }
            }

            std::thread::sleep(Duration::from_millis(8));
        }

        // Keep pumping NSApp events so the order-out flushes, AppKit stays sane,
        // AND reopen Apple Events keep reaching our delegate for the rest of the
        // process lifetime (the common case: user double-clicks a long-running
        // app). The supervisor thread owns process lifetime and `process::exit`s
        // when the host exits, which ends this loop with the process.
        loop {
            unsafe {
                pump_app_events(0.2);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Pump the launcher's `NSApplication` event loop for up to `seconds` on the main
/// thread. Unlike a bare `CFRunLoopRunInMode`, NSApp's pump drains the AppKit
/// event queue **and the Apple Event Mach port** — the reopen (`kAEReopenApplication`,
/// `'rapp'`) event is delivered through this pump, not a plain CFRunLoop. Without
/// it the launcher's reopen delegate is never invoked and a Finder/Dock reopen
/// times out (`errAETimeout` → "AgentMux is not responding"). The AE is dispatched
/// to the delegate as a side effect of the run-loop service inside
/// `nextEventMatchingMask:`, so we don't need the returned event for reopen — we
/// still forward any returned UI event to keep the splash window responsive.
unsafe fn pump_app_events(seconds: f64) {
    let app = send(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
    if app.is_null() {
        return;
    }
    let until: id = {
        let f: extern "C" fn(id, SEL, f64) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(
            class(b"NSDate\0"),
            sel(b"dateWithTimeIntervalSinceNow:\0"),
            seconds,
        )
    };
    // NSEventMaskAny == NSUIntegerMax; dequeue: YES. Blocks until an event or
    // `until`; the AE port is serviced during that wait.
    let next: extern "C" fn(id, SEL, u64, id, CFStringRef, u8) -> id =
        std::mem::transmute(objc_msgSend as *const ());
    let evt = next(
        app,
        sel(b"nextEventMatchingMask:untilDate:inMode:dequeue:\0"),
        u64::MAX,
        until,
        kCFRunLoopDefaultMode,
        1,
    );
    if !evt.is_null() {
        send_void_id(app, sel(b"sendEvent:\0"), evt);
    }
}

/// Render `window`'s content view to a PNG at `path` (offscreen — no Screen
/// Recording permission needed). Best-effort; silently no-ops on any failure.
unsafe fn dump_window_png(window: id, path: &str) {
    let view = send(window, sel(b"contentView\0"));
    if view.is_null() {
        return;
    }
    let bounds: CGRect = {
        let f: extern "C" fn(id, SEL) -> CGRect = std::mem::transmute(objc_msgSend as *const ());
        f(view, sel(b"bounds\0"))
    };
    let rep = {
        let f: extern "C" fn(id, SEL, CGRect) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(
            view,
            sel(b"bitmapImageRepForCachingDisplayInRect:\0"),
            bounds,
        )
    };
    if rep.is_null() {
        return;
    }
    {
        let f: extern "C" fn(id, SEL, CGRect, id) =
            std::mem::transmute(objc_msgSend as *const ());
        f(view, sel(b"cacheDisplayInRect:toBitmapImageRep:\0"), bounds, rep);
    }
    let props = send(class(b"NSDictionary\0"), sel(b"dictionary\0"));
    // NSBitmapImageFileTypePNG == 4
    let data = {
        let f: extern "C" fn(id, SEL, u64, id) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(rep, sel(b"representationUsingType:properties:\0"), 4, props)
    };
    if data.is_null() {
        return;
    }
    let nspath = nsstring(path);
    let f: extern "C" fn(id, SEL, id, u8) -> u8 = std::mem::transmute(objc_msgSend as *const ());
    f(data, sel(b"writeToFile:atomically:\0"), nspath, 1);
}

/// Build the splash window: a dark rounded backdrop with the brain centered on
/// it. Returns (window, image_view, stage_fields) — the image_view's alpha is
/// pulsed; stage_fields are pre-allocated (label, time) NSTextField pairs
/// updated live by `update_stage_fields`.
unsafe fn build_window() -> (id, id, Vec<(id, id)>) {
    // NSApplication, as an accessory app: no Dock tile (the host sets .regular
    // and owns the single tile). Accessory == 1.
    let app = send(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
    send_void_i64(app, sel(b"setActivationPolicy:\0"), 1);

    // NSImage from the embedded PNG bytes via NSData.
    let data = {
        let f: extern "C" fn(id, SEL, *const std::ffi::c_void, usize) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(
            class(b"NSData\0"),
            sel(b"dataWithBytes:length:\0"),
            BRAIN_PNG.as_ptr() as *const std::ffi::c_void,
            BRAIN_PNG.len(),
        )
    };
    let image = send_id(
        send(class(b"NSImage\0"), sel(b"alloc\0")),
        sel(b"initWithData:\0"),
        data,
    );

    // Center on the main screen.
    let screen = send(class(b"NSScreen\0"), sel(b"mainScreen\0"));
    let screen_frame: CGRect = {
        let f: extern "C" fn(id, SEL) -> CGRect = std::mem::transmute(objc_msgSend as *const ());
        f(screen, sel(b"frame\0"))
    };
    let x = screen_frame.origin.x + (screen_frame.size.width - SPLASH_W) / 2.0;
    let y = screen_frame.origin.y + (screen_frame.size.height - SPLASH_H) / 2.0;
    let win_rect = CGRect {
        origin: CGPoint { x, y },
        size: CGSize {
            width: SPLASH_W,
            height: SPLASH_H,
        },
    };

    // NSWindow: borderless (styleMask 0), buffered (backing 2), defer NO.
    let window = {
        let win = send(class(b"NSWindow\0"), sel(b"alloc\0"));
        let f: extern "C" fn(id, SEL, CGRect, u64, u64, u8) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(
            win,
            sel(b"initWithContentRect:styleMask:backing:defer:\0"),
            win_rect,
            0,
            2,
            0,
        )
    };
    // The window itself is transparent; the dark card is the contentView's
    // layer (so we get rounded corners + a shadow around the card).
    send_void_bool(window, sel(b"setOpaque:\0"), 0);
    let clear = send(class(b"NSColor\0"), sel(b"clearColor\0"));
    send_void_id(window, sel(b"setBackgroundColor:\0"), clear);
    send_void_bool(window, sel(b"setHasShadow:\0"), 1);
    send_void_i64(window, sel(b"setLevel:\0"), 25); // NSStatusWindowLevel
    send_void_bool(window, sel(b"setIgnoresMouseEvents:\0"), 1);

    // Backdrop: layer-backed NSView filling the window, dark fill + rounded.
    let local = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: SPLASH_W,
            height: SPLASH_H,
        },
    };
    let backdrop = {
        let v = send(class(b"NSView\0"), sel(b"alloc\0"));
        let f: extern "C" fn(id, SEL, CGRect) -> id = std::mem::transmute(objc_msgSend as *const ());
        f(v, sel(b"initWithFrame:\0"), local)
    };
    send_void_bool(backdrop, sel(b"setWantsLayer:\0"), 1);
    let layer = send(backdrop, sel(b"layer\0"));
    // CGColorRef from NSColor (colorWithSRGBRed:green:blue:alpha:).
    let bg_color = {
        let f: extern "C" fn(id, SEL, f64, f64, f64, f64) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(
            class(b"NSColor\0"),
            sel(b"colorWithSRGBRed:green:blue:alpha:\0"),
            BG_R,
            BG_G,
            BG_B,
            1.0,
        )
    };
    let bg_cgcolor = send(bg_color, sel(b"CGColor\0"));
    send_void_id(layer, sel(b"setBackgroundColor:\0"), bg_cgcolor);
    send_void_f64(layer, sel(b"setCornerRadius:\0"), CORNER_RADIUS);
    send_void_bool(layer, sel(b"setMasksToBounds:\0"), 1);

    // Darkened 2px border, inset from the same rounded-rect path as the
    // corner radius above — SPEC_SPLASH_SCREEN_BORDER_2026_08_25.md. Native
    // CALayer support, no extra corner math needed.
    let border_color = {
        let f: extern "C" fn(id, SEL, f64, f64, f64, f64) -> id =
            std::mem::transmute(objc_msgSend as *const ());
        f(
            class(b"NSColor\0"),
            sel(b"colorWithSRGBRed:green:blue:alpha:\0"),
            BORDER_R,
            BORDER_G,
            BORDER_B,
            1.0,
        )
    };
    let border_cgcolor = send(border_color, sel(b"CGColor\0"));
    send_void_id(layer, sel(b"setBorderColor:\0"), border_cgcolor);
    send_void_f64(layer, sel(b"setBorderWidth:\0"), BORDER_WIDTH);

    // Brain image view in the upper (brain) region, above the stage panel
    // and footer band. macOS Y is bottom-up, so a larger y sits higher.
    let brain_rect = CGRect {
        origin: CGPoint {
            x: PAD_PX,
            y: FOOTER_BAND_H + STAGE_AREA_H + PAD_PX,
        },
        size: CGSize {
            width: BRAIN_PX,
            height: BRAIN_PX,
        },
    };
    let image_view = {
        let iv = send(class(b"NSImageView\0"), sel(b"alloc\0"));
        let f: extern "C" fn(id, SEL, CGRect) -> id = std::mem::transmute(objc_msgSend as *const ());
        f(iv, sel(b"initWithFrame:\0"), brain_rect)
    };
    send_void_id(image_view, sel(b"setImage:\0"), image);
    {
        // NSImageScaleProportionallyUpOrDown == 3
        let f: extern "C" fn(id, SEL, u64) = std::mem::transmute(objc_msgSend as *const ());
        f(image_view, sel(b"setImageScaling:\0"), 3);
    }
    send_void_f64(image_view, sel(b"setAlphaValue:\0"), 0.0); // fades in

    send_void_id(backdrop, sel(b"addSubview:\0"), image_view);

    // Footer: native NSTextField labels in the bottom band (muted, centered).
    // They fade with the window on dismiss (window alpha ramps) and do not
    // pulse. macOS Y is bottom-up: line[0] (user@host) sits above line[1] (version).
    {
        let info = crate::splash_info::SplashInfo::gather();
        let max_chars = ((SPLASH_W - 24.0) / 7.5) as usize; // ~7.5 px/char at 13px mono
        let lines = info.footer_lines(max_chars.max(8));
        let footer_color = {
            let f: extern "C" fn(id, SEL, f64, f64, f64, f64) -> id =
                std::mem::transmute(objc_msgSend as *const ());
            f(
                class(b"NSColor\0"),
                sel(b"colorWithSRGBRed:green:blue:alpha:\0"),
                FOOTER_R,
                FOOTER_G,
                FOOTER_B,
                1.0,
            )
        };
        let font = send_id_f64(class(b"NSFont\0"), sel(b"userFixedPitchFontOfSize:\0"), 13.0);
        let n = lines.len();
        for (i, line) in lines.iter().enumerate() {
            let ly = FOOTER_PAD + (n - 1 - i) as f64 * FOOTER_LINE_H;
            let rect = CGRect {
                origin: CGPoint { x: 0.0, y: ly },
                size: CGSize {
                    width: SPLASH_W,
                    height: FOOTER_LINE_H,
                },
            };
            // labelWithString: → a non-editable, borderless, transparent label.
            let label = send_id(
                class(b"NSTextField\0"),
                sel(b"labelWithString:\0"),
                nsstring(line),
            );
            send_void_rect(label, sel(b"setFrame:\0"), rect);
            send_void_id(label, sel(b"setTextColor:\0"), footer_color);
            send_void_id(label, sel(b"setFont:\0"), font);
            // NSTextAlignmentCenter == 1 in the *unified* NSTextAlignment enum
            // (macOS 10.12+ adopted UIKit's values: Left 0, Center 1, Right 2,
            // Justified 3, Natural 4). The pre-Sierra macOS value for center was
            // 2 — using it here right-aligns the footer instead of centering it.
            send_void_i64(label, sel(b"setAlignment:\0"), 1);
            send_void_id(backdrop, sel(b"addSubview:\0"), label);
        }
    }

    // Stage-telemetry rows: MAX_STAGE_ROWS pre-allocated (label, time) field
    // pairs between the footer band and the brain region, hidden/empty until
    // update_stage_fields fills them in. Row 0 sits at the top of the panel
    // (closest to the brain); later rows go downward (macOS Y decreases).
    let stage_font = send_id_f64(class(b"NSFont\0"), sel(b"userFixedPitchFontOfSize:\0"), 11.0);
    let area_top = FOOTER_BAND_H + STAGE_AREA_H;
    let name_w = SPLASH_W * STAGE_NAME_FRAC - STAGE_MARGIN_L;
    let time_x = SPLASH_W * STAGE_NAME_FRAC;
    let time_w = SPLASH_W - STAGE_MARGIN_R - time_x;
    let mut stage_fields: Vec<(id, id)> = Vec::with_capacity(MAX_STAGE_ROWS);
    for i in 0..MAX_STAGE_ROWS {
        let y = area_top - STAGE_PAD_TOP - (i as f64 + 1.0) * STAGE_ROW_H;
        let name_rect = CGRect {
            origin: CGPoint { x: STAGE_MARGIN_L, y },
            size: CGSize { width: name_w, height: STAGE_ROW_H },
        };
        let time_rect = CGRect {
            origin: CGPoint { x: time_x, y },
            size: CGSize { width: time_w, height: STAGE_ROW_H },
        };
        let name_field = send_id(class(b"NSTextField\0"), sel(b"labelWithString:\0"), nsstring(""));
        send_void_rect(name_field, sel(b"setFrame:\0"), name_rect);
        send_void_id(name_field, sel(b"setFont:\0"), stage_font);
        send_void_bool(name_field, sel(b"setHidden:\0"), 1);
        send_void_id(backdrop, sel(b"addSubview:\0"), name_field);

        let time_field = send_id(class(b"NSTextField\0"), sel(b"labelWithString:\0"), nsstring(""));
        send_void_rect(time_field, sel(b"setFrame:\0"), time_rect);
        send_void_id(time_field, sel(b"setFont:\0"), stage_font);
        send_void_i64(time_field, sel(b"setAlignment:\0"), 2); // NSTextAlignmentRight
        send_void_bool(time_field, sel(b"setHidden:\0"), 1);
        send_void_id(backdrop, sel(b"addSubview:\0"), time_field);

        stage_fields.push((name_field, time_field));
    }

    send_void_id(window, sel(b"setContentView:\0"), backdrop);

    // Bring the app + window up immediately.
    send_void(app, sel(b"finishLaunching\0"));
    send_void_bool(app, sel(b"activateIgnoringOtherApps:\0"), 1);
    send_void(window, sel(b"orderFrontRegardless\0"));
    (window, image_view, stage_fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_stage(stage: &'static str, label: &'static str, ms: u64) -> StageRow {
        StageRow {
            stage,
            label,
            started_at: Instant::now(),
            done: Some((ms, StartupStatus::Ok, None)),
            subs: Vec::new(),
        }
    }

    fn running_stage(stage: &'static str, label: &'static str) -> StageRow {
        StageRow {
            stage,
            label,
            started_at: Instant::now(),
            done: None,
            subs: Vec::new(),
        }
    }

    /// Pulls the "other: ..." row's time_text out of a flatten_rows() result,
    /// if present.
    fn other_row_text(rows: &[FlatRow]) -> Option<&str> {
        rows.iter()
            .find(|r| r.time_text.starts_with("other: "))
            .map(|r| r.time_text.as_str())
    }

    fn total_row_text(rows: &[FlatRow]) -> Option<&str> {
        rows.iter()
            .find(|r| r.time_text.starts_with("total: "))
            .map(|r| r.time_text.as_str())
    }

    #[test]
    fn no_other_or_total_row_while_total_ms_is_none() {
        let stages = vec![done_stage("prep", "Prep", 100)];
        let rows = flatten_rows(&stages, None);
        assert!(other_row_text(&rows).is_none());
        assert!(total_row_text(&rows).is_none());
    }

    #[test]
    fn other_row_is_the_gap_between_total_and_summed_stages() {
        let stages = vec![
            done_stage("prep", "Prep", 100),
            done_stage("migrations", "Migrations", 250),
        ];
        // 100 + 250 = 350 accounted; total is 500 -> 150ms unaccounted.
        let rows = flatten_rows(&stages, Some(500));
        assert_eq!(other_row_text(&rows), Some("other: 150ms"));
        assert_eq!(total_row_text(&rows), Some("total: 500ms"));
    }

    #[test]
    fn other_row_excludes_a_still_running_stage_from_the_sum() {
        // A stage with no StageEnd yet must not be counted as 0 duration
        // *or* as its still-changing partial elapsed time — it's simply
        // excluded from `accounted_ms`, same treatment either way here
        // since we only ever fold `done` stages.
        let stages = vec![done_stage("prep", "Prep", 100), running_stage("host", "Host")];
        let rows = flatten_rows(&stages, Some(300));
        assert_eq!(other_row_text(&rows), Some("other: 200ms"));
    }

    #[test]
    fn other_row_saturates_at_zero_rather_than_underflowing() {
        // Pathological: reported stage durations sum to more than the
        // wall-clock total (e.g. a race at the ready-detection instant).
        // Must not panic or wrap around via unsigned subtraction.
        let stages = vec![done_stage("prep", "Prep", 900)];
        let rows = flatten_rows(&stages, Some(500));
        assert_eq!(other_row_text(&rows), Some("other: 0ms"));
    }

    #[test]
    fn sub_row_durations_are_not_double_counted_into_other() {
        // A sub's time is already inside its parent stage's own reported
        // duration_ms (the stage spans stage-begin to stage-end, which
        // wraps its subs) — summing subs separately would double-count and
        // shrink "other" incorrectly. accounted_ms must come from top-level
        // stages only.
        let mut migrations = done_stage("migrations", "Migrations", 250);
        migrations.subs.push(SubRow {
            id: "001".into(),
            label: "001_init".into(),
            started_at: Instant::now(),
            done: Some((90, StartupStatus::Ok, None)),
        });
        let stages = vec![done_stage("prep", "Prep", 100), migrations];
        let rows = flatten_rows(&stages, Some(500));
        // If subs were double-counted this would be 500 - (100+250+90) = 60,
        // not 150.
        assert_eq!(other_row_text(&rows), Some("other: 150ms"));
    }

    #[test]
    fn total_row_still_shown_when_panel_is_nearly_full() {
        // MAX_STAGE_ROWS(12) - 1 stage rows leaves exactly one slot: "total"
        // must win that slot over "other" per the priority documented at
        // the call site.
        let stages: Vec<StageRow> = (0..MAX_STAGE_ROWS - 1)
            .map(|i| {
                let label: &'static str = Box::leak(format!("s{i}").into_boxed_str());
                let stage: &'static str = label;
                done_stage(stage, label, 10)
            })
            .collect();
        let rows = flatten_rows(&stages, Some(1000));
        assert_eq!(rows.len(), MAX_STAGE_ROWS);
        assert!(other_row_text(&rows).is_none());
        assert!(total_row_text(&rows).is_some());
    }

    fn stage_row<'a>(stages: &'a [StageRow], stage: &str) -> &'a StageRow {
        stages.iter().find(|s| s.stage == stage).unwrap()
    }

    #[test]
    fn a_same_tick_begin_and_end_pair_is_deferred_not_snapped_done() {
        // The count-up bug: if a step's Begin and End are both already
        // queued by the time a tick drains, applying both immediately would
        // create the row already-done — no "running" frame ever painted.
        let mut stages = Vec::new();
        let mut deferred = Vec::new();
        let fresh = vec![
            StartupEvent::StageBegin { stage: "prep", label: "Prep" },
            StartupEvent::StageEnd {
                stage: "prep",
                duration_ms: 5,
                status: StartupStatus::Ok,
                detail: None,
            },
        ];
        let changed = apply_tick(&mut stages, &mut deferred, fresh);
        assert!(changed);
        // Begin applied, End held back — row exists but is still "running".
        assert_eq!(stages.len(), 1);
        assert!(stage_row(&stages, "prep").done.is_none());
        assert_eq!(deferred.len(), 1);
    }

    #[test]
    fn the_deferred_end_applies_on_the_next_tick() {
        let mut stages = Vec::new();
        let mut deferred = Vec::new();
        apply_tick(
            &mut stages,
            &mut deferred,
            vec![
                StartupEvent::StageBegin { stage: "prep", label: "Prep" },
                StartupEvent::StageEnd {
                    stage: "prep",
                    duration_ms: 5,
                    status: StartupStatus::Ok,
                    detail: None,
                },
            ],
        );
        // Next tick: nothing fresh, just the deferred End flushing in.
        let changed = apply_tick(&mut stages, &mut deferred, Vec::new());
        assert!(changed);
        assert!(deferred.is_empty());
        assert_eq!(stage_row(&stages, "prep").done.as_ref().unwrap().0, 5);
    }

    #[test]
    fn a_begin_and_end_landing_in_different_ticks_is_never_deferred() {
        // The common/expected case (step genuinely takes longer than one
        // tick) must not be held back an *extra* tick on top of that.
        let mut stages = Vec::new();
        let mut deferred = Vec::new();
        apply_tick(
            &mut stages,
            &mut deferred,
            vec![StartupEvent::StageBegin { stage: "host", label: "Host" }],
        );
        assert!(deferred.is_empty());
        assert!(stage_row(&stages, "host").done.is_none());

        let changed = apply_tick(
            &mut stages,
            &mut deferred,
            vec![StartupEvent::StageEnd {
                stage: "host",
                duration_ms: 40,
                status: StartupStatus::Ok,
                detail: None,
            }],
        );
        assert!(changed);
        assert!(deferred.is_empty());
        assert_eq!(stage_row(&stages, "host").done.as_ref().unwrap().0, 40);
    }

    #[test]
    fn a_same_tick_sub_begin_and_end_pair_is_deferred_too() {
        let mut stages = vec![running_stage("migrations", "Migrations")];
        let mut deferred = Vec::new();
        let fresh = vec![
            StartupEvent::SubBegin {
                stage: "migrations",
                id: "001".into(),
                label: "001_init".into(),
            },
            StartupEvent::SubEnd {
                stage: "migrations",
                id: "001".into(),
                duration_ms: 3,
                status: StartupStatus::Ok,
                detail: None,
            },
        ];
        apply_tick(&mut stages, &mut deferred, fresh);
        let sub = &stage_row(&stages, "migrations").subs[0];
        assert!(sub.done.is_none());
        assert_eq!(deferred.len(), 1);
    }

    #[test]
    fn unrelated_deferred_and_fresh_events_do_not_interfere() {
        // A deferred End from a previous tick and a brand-new, unrelated
        // same-tick Begin+End pair in this tick must each be judged on their
        // own — the deferred flush must not seed `began_this_tick`.
        let mut stages = Vec::new();
        let mut deferred = vec![StartupEvent::StageEnd {
            stage: "prep",
            duration_ms: 5,
            status: StartupStatus::Ok,
            detail: None,
        }];
        stages.push(running_stage("prep", "Prep"));
        let fresh = vec![
            StartupEvent::StageBegin { stage: "migrations", label: "Migrations" },
            StartupEvent::StageEnd {
                stage: "migrations",
                duration_ms: 2,
                status: StartupStatus::Ok,
                detail: None,
            },
        ];
        apply_tick(&mut stages, &mut deferred, fresh);
        // "prep"'s deferred End flushed in this tick.
        assert_eq!(stage_row(&stages, "prep").done.as_ref().unwrap().0, 5);
        // "migrations" is new-and-fast this tick — its own End is deferred.
        assert!(stage_row(&stages, "migrations").done.is_none());
        assert_eq!(deferred.len(), 1);
    }
}
