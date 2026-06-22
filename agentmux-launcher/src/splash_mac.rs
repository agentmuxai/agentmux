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
use std::time::{Duration, Instant};

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
const SPLASH_H: f64 = SPLASH_PX + FOOTER_BAND_H;
// Muted footer text color #8A8A93.
const FOOTER_R: f64 = 0x8A as f64 / 255.0;
const FOOTER_G: f64 = 0x8A as f64 / 255.0;
const FOOTER_B: f64 = 0x93 as f64 / 255.0;

// Backdrop color — matches the Win32 splash's BG (R=0x1A, G=0x1A, B=0x1F).
const BG_R: f64 = 26.0 / 255.0;
const BG_G: f64 = 26.0 / 255.0;
const BG_B: f64 = 31.0 / 255.0;

/// Safety net: if the host never signals (crash before first paint), tear the
/// splash down anyway so it can't get stuck on screen.
const DISMISS_TIMEOUT: Duration = Duration::from_secs(10);
const FADE_OUT: f64 = 0.16; // seconds

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

/// A live splash window plus the path the host touches when it's ready.
pub struct Splash {
    window: id,
    image_view: id,
    ready_file: PathBuf,
}

impl Splash {
    /// Create the `AGENTMUX_SPLASH_READY_FILE` env path and the window, paint
    /// it, and return the handle. MUST be called on the process main thread,
    /// before the supervisor thread spawns the host (so the host inherits the
    /// env var).
    pub fn show() -> Splash {
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
        let (window, image_view) = unsafe {
            let pool = send(class(b"NSAutoreleasePool\0"), sel(b"alloc\0"));
            let pool = send(pool, sel(b"init\0"));
            let result = build_window();
            send_void(pool, sel(b"drain\0"));
            result
        };
        Splash {
            window,
            image_view,
            ready_file,
        }
    }

    /// Dev affordance (used by `--splash-selftest` + `AGENTMUX_SPLASH_DUMP_PNG`):
    /// pump the runloop briefly so layout/draw settle, then render the splash's
    /// content view to a PNG at `path` — lets us eyeball footer centering without
    /// Screen Recording permission. Offscreen `cacheDisplayInRect:` only.
    pub fn dump_png(&self, path: &str) {
        for _ in 0..25 {
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.016, 0);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        unsafe { dump_window_png(self.window, path) };
    }

    /// Pump the splash runloop on the main thread: animate the brain pulse, and
    /// once the host signals first paint (ready-file appears) — or the safety
    /// timeout elapses — fade the whole splash out and order it away. Then keep
    /// the runloop turning (so the removal flushes) until the supervisor thread
    /// exits the process.
    pub fn run_until_dismissed(self) {
        let start = Instant::now();
        let mut fade_start: Option<Instant> = None;

        loop {
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.016, 0);
            }
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

            // Trigger the fade-out once the host is ready (or we time out).
            if fade_start.is_none()
                && (self.ready_file.exists() || start.elapsed() > DISMISS_TIMEOUT)
            {
                fade_start = Some(Instant::now());
                let _ = std::fs::remove_file(&self.ready_file);
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

        // Keep pumping so the order-out flushes and AppKit stays sane. The
        // supervisor thread owns process lifetime and `process::exit`s when the
        // host exits, which ends this loop with the process.
        loop {
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.2, 0);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
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
/// it. Returns (window, image_view) — the image_view's alpha is pulsed.
unsafe fn build_window() -> (id, id) {
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

    // Brain image view in the upper (brain) region, above the footer band.
    // macOS Y is bottom-up, so a larger y sits higher.
    let brain_rect = CGRect {
        origin: CGPoint {
            x: PAD_PX,
            y: FOOTER_BAND_H + PAD_PX,
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

    send_void_id(window, sel(b"setContentView:\0"), backdrop);

    // Bring the app + window up immediately.
    send_void(app, sel(b"finishLaunching\0"));
    send_void_bool(app, sel(b"activateIgnoringOtherApps:\0"), 1);
    send_void(window, sel(b"orderFrontRegardless\0"));
    (window, image_view)
}
