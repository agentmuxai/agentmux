// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! macOS menu-bar backend — issue #2977 Workstream 1.
//!
//! ## No thread of its own
//!
//! AppKit requires an `NSStatusItem` to be created on the main thread and
//! driven by a main-thread event loop (`tray-icon` enforces the first half
//! with `MainThreadMarker` and returns `NotMainThread` otherwise). The
//! launcher's supervisor runs on a worker thread on macOS precisely so the
//! main thread can belong to AppKit (`main.rs`), and that main thread already
//! pumps `NSApplication` for the whole process lifetime — see
//! `splash_mac::pump_app_events` and design doc §7.5.
//!
//! So this backend is the inverse of `windows.rs`: it owns no thread. `spawn`
//! (called from the supervisor thread) only queues a request; the main-thread
//! pump calls `pump_tick` once per iteration, which services that request —
//! creating the item — and thereafter drains `muda`/`tray-icon` events and
//! forwards them as `TrayAction`s on the channel the supervisor holds.
//!
//! ## Why the pump must exist before `spawn` is called
//!
//! With the splash disabled the launcher used to run the supervisor on the
//! main thread with no `NSApplication` at all (design doc §7.5.1's macOS
//! gap). A queued request would then never be serviced, and the log would
//! claim "tray: started" for an icon that never appears. `main.rs` therefore
//! chooses the pump layout whenever background-service mode is on, and the
//! pump owner calls `mark_main_pump_available` *before* the supervisor thread
//! is spawned, so `spawn` can refuse honestly if that did not happen.
//!
//! ## Click model
//!
//! Left-click shows the menu (`tray-icon`'s macOS default and the platform
//! convention); there is no direct-action click as on Windows. Everything the
//! user can do is in `menu_model`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};

use super::TrayAction;

/// Set by the main thread once it is committed to pumping `NSApplication`
/// for the process lifetime. Read by `spawn` on the supervisor thread.
static MAIN_PUMP_AVAILABLE: AtomicBool = AtomicBool::new(false);

/// A create request handed from the supervisor thread to the main thread.
struct Request {
    tx: mpsc::Sender<TrayAction>,
    data_dir: PathBuf,
    dir_hash: String,
}

/// The pending request, if any. `spawn` fills it; `pump_tick` takes it.
static PENDING: Mutex<Option<Request>> = Mutex::new(None);

/// Declare that the calling (main) thread will pump AppKit for the rest of
/// the process lifetime and call `pump_tick` each iteration. Must run before
/// the supervisor thread is spawned — see the module docs.
pub fn mark_main_pump_available() {
    MAIN_PUMP_AVAILABLE.store(true, Ordering::SeqCst);
}

/// Queue creation of the menu-bar item. Returns the receiver for user actions.
///
/// Errors are returned rather than panicking: `start_if_enabled` degrades to
/// "no tray" on failure, because a cosmetic icon must never take down the
/// process that supervises `srv` and `host`.
pub fn spawn(
    data_dir: PathBuf,
    dir_hash: String,
) -> Result<mpsc::Receiver<TrayAction>, String> {
    if !MAIN_PUMP_AVAILABLE.load(Ordering::SeqCst) {
        return Err(
            "main thread is not pumping AppKit (no splash and not in background-service \
             mode), so a menu-bar item could never be created or clicked"
                .to_string(),
        );
    }
    let (tx, rx) = mpsc::channel::<TrayAction>();
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if pending.is_some() {
        return Err("a menu-bar item request is already queued".to_string());
    }
    *pending = Some(Request {
        tx,
        data_dir,
        dir_hash,
    });
    Ok(rx)
}

/// Everything that must live — and only be touched — on the main thread.
struct Live {
    tray: tray_icon::TrayIcon,
    /// Keeps the `muda` menu alive alongside the status item.
    _menu: muda::Menu,
    /// The first item's label tracks running-state, so keep a handle to it.
    open_item: muda::MenuItem,
    /// muda identifies selections by item id, not by index.
    items: Vec<(muda::MenuId, TrayAction)>,
    tx: mpsc::Sender<TrayAction>,
    /// Reachability changes from the poller thread, delivered here so the
    /// icon is only ever updated from the thread that created it.
    status_rx: mpsc::Receiver<bool>,
    running: bool,
    /// Whether the placement line has been logged (see `drain`).
    placed_logged: bool,
}

thread_local! {
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
}

/// One iteration of tray work, to be called by the main-thread pump after
/// each `pump_app_events`.
///
/// Services a pending create request, then drains status changes and user
/// events. Cheap when idle: three non-blocking `try_recv`s.
pub fn pump_tick() {
    LIVE.with(|cell| {
        let mut live = cell.borrow_mut();
        match live.as_mut() {
            None => {
                let request = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(req) = request {
                    match create(req) {
                        Ok(l) => *live = Some(l),
                        Err(e) => crate::log(&format!("tray: creating menu-bar item failed: {}", e)),
                    }
                }
            }
            Some(l) => {
                if !l.drain() {
                    // Receiver gone — the supervisor is shutting down. Dropping
                    // the `TrayIcon` removes the status item, so the icon can't
                    // outlive the thing it claims is running.
                    crate::log("tray: action receiver gone; removing menu-bar item");
                    *live = None;
                }
            }
        }
    });
}

/// Build the menu and status item. Main thread only.
fn create(req: Request) -> Result<Live, String> {
    use muda::{Menu, MenuItem as MudaItem};
    use tray_icon::TrayIconBuilder;

    // Start from the REAL state, not an assumption: the request is queued
    // before `srv`/`host` are spawned, so hard-coding "running" would make
    // the icon lie for the whole startup window. Same rule as Windows.
    let running = super::service_reachable(&req.data_dir, &req.dir_hash);
    let model = super::menu_model(running);
    let menu = Menu::new();
    let mut items: Vec<(muda::MenuId, TrayAction)> = Vec::new();
    let mut open_item: Option<MudaItem> = None;
    for entry in &model {
        let item = MudaItem::new(&entry.label, true, None);
        items.push((item.id().clone(), entry.action));
        menu.append(&item)
            .map_err(|e| format!("append menu item {:?}: {}", entry.label, e))?;
        if entry.action == TrayAction::OpenWindow {
            open_item = Some(item);
        }
    }
    let open_item = open_item.ok_or("menu model has no OpenWindow item")?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_tooltip(super::tooltip(running))
        .with_icon(icon())
        // Template: macOS renders the alpha mask in the menu bar's own tint,
        // so the mark is correct in light and dark menu bars and when the
        // item is highlighted. See `menubar-template.png`.
        .with_icon_as_template(true)
        .build()
        .map_err(|e| format!("build status item: {}", e))?;

    // Status poller. Blocking I/O, so off the main thread; it only ever sends
    // a bool over a channel that `drain` reads on the main thread.
    let (status_tx, status_rx) = mpsc::channel::<bool>();
    {
        let poll_dir = req.data_dir.clone();
        let poll_hash = req.dir_hash.clone();
        std::thread::Builder::new()
            .name("agentmux-tray-status".into())
            .spawn(move || {
                let mut last = running;
                loop {
                    std::thread::sleep(super::STATUS_POLL);
                    let now = super::service_reachable(&poll_dir, &poll_hash);
                    if now != last {
                        last = now;
                        if status_tx.send(now).is_err() {
                            return; // item gone; nothing left to update
                        }
                    }
                }
            })
            .map_err(|e| format!("spawn status poller: {}", e))?;
    }

    crate::log(&format!(
        "tray: menu-bar item created (service_reachable={})",
        running
    ));

    Ok(Live {
        tray,
        _menu: menu,
        open_item,
        items,
        tx: req.tx,
        status_rx,
        running,
        placed_logged: false,
    })
}

impl Live {
    /// Apply pending status changes and forward user events. Returns `false`
    /// when the action receiver has gone away.
    fn drain(&mut self) -> bool {
        use muda::MenuEvent;
        use tray_icon::TrayIconEvent;

        // Placement evidence. At creation the status window has no layout
        // yet (height 0, origin at the screen bottom), so the rect is only
        // meaningful once AppKit has ordered it in. A screen position with
        // non-zero height is the best headless proof that the menu bar gave
        // the item a slot; logged once so a log reader can tell "created"
        // from "placed" without a screenshot.
        if !self.placed_logged {
            if let Some(r) = self.tray.rect() {
                if r.size.height > 0 {
                    self.placed_logged = true;
                    crate::log(&format!(
                        "tray: menu-bar item placed at x={} y={} w={} h={} (physical px)",
                        r.position.x, r.position.y, r.size.width, r.size.height
                    ));
                }
            }
        }

        while let Ok(now) = self.status_rx.try_recv() {
            if now == self.running {
                continue;
            }
            self.running = now;
            let _ = self.tray.set_tooltip(Some(super::tooltip(now)));
            // Reuse the shared model so the label wording stays in one place
            // (and stays covered by `tray_model_tests`).
            if let Some(entry) = super::menu_model(now)
                .into_iter()
                .find(|e| e.action == TrayAction::OpenWindow)
            {
                self.open_item.set_text(entry.label);
            }
            match self.tray.rect() {
                Some(r) => crate::log(&format!(
                    "tray: service_reachable -> {} (item at x={} y={} w={} h={})",
                    now, r.position.x, r.position.y, r.size.width, r.size.height
                )),
                None => crate::log(&format!("tray: service_reachable -> {}", now)),
            }
        }

        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if let Some(action) = self.items.iter().find(|(id, _)| *id == ev.id).map(|(_, a)| *a) {
                if self.tx.send(action).is_err() {
                    return false;
                }
            }
        }

        // Clicks on the item itself open the menu (handled inside AppKit's
        // `sendEvent:`); nothing to act on here, but the process-global
        // channel must still be drained so it never grows unbounded.
        while TrayIconEvent::receiver().try_recv().is_ok() {}

        true
    }
}

/// The menu-bar glyph, embedded at build time.
///
/// 36x36 = 18pt at 2x, the height `tray-icon` sizes the `NSImage` to, so the
/// bitmap is used 1:1 on Retina instead of being resampled. Black-on-alpha:
/// with `with_icon_as_template(true)` only the alpha channel matters — macOS
/// supplies the colour. Derived from `frontend/app/asset/agentmux-icon.png`
/// by keeping the bright panels and dropping the dark backdrop, so it reads
/// as the mark rather than a solid rounded square.
static MENUBAR_TEMPLATE_PNG: &[u8] = include_bytes!("../../resources/menubar-template.png");

/// The tray icon: the embedded brand glyph, or a loud fallback.
fn icon() -> tray_icon::Icon {
    match decode_rgba8(MENUBAR_TEMPLATE_PNG)
        .and_then(|(rgba, w, h)| {
            tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| format!("from_rgba: {}", e))
        }) {
        Ok(icon) => icon,
        Err(e) => {
            crate::log(&format!(
                "tray: embedded menu-bar glyph unusable ({}) — falling back to the generated mark",
                e
            ));
            fallback_icon()
        }
    }
}

/// Decode a PNG that must already be 8-bit RGBA. No colour-space
/// transformations are requested: the asset is produced in that exact form,
/// and anything else is a build-input mistake worth failing loudly on.
fn decode_rgba8(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| format!("png header: {}", e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "png output size overflow".to_string())?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("png frame: {}", e))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "expected RGBA8, got {:?}/{:?}",
            info.color_type, info.bit_depth
        ));
    }
    buf.truncate(info.buffer_size());
    Ok((buf, info.width, info.height))
}

/// Last-resort mark, used only when the embedded glyph cannot be decoded.
/// A solid square — as a template image it renders as a filled block, which
/// is obviously not the brand mark, so a broken asset can't pass for the real
/// thing.
fn fallback_icon() -> tray_icon::Icon {
    const S: u32 = 16;
    let rgba = std::iter::repeat([0, 0, 0, 0xFF])
        .take((S * S) as usize)
        .flatten()
        .collect::<Vec<u8>>();
    tray_icon::Icon::from_rgba(rgba, S, S).expect("16x16 RGBA buffer is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_glyph_is_rgba8_at_2x_and_is_a_glyph_not_a_slab() {
        let (rgba, w, h) = decode_rgba8(MENUBAR_TEMPLATE_PNG).expect("decodes");
        // 18pt @2x — the size tray-icon renders the NSImage at.
        assert_eq!((w, h), (36, 36));
        assert_eq!(rgba.len(), (36 * 36 * 4) as usize);
        let alphas = rgba.chunks(4).map(|px| px[3]);
        let opaque = alphas.clone().filter(|&a| a == 0xFF).count();
        let clear = alphas.filter(|&a| a == 0).count();
        // Template images are pure alpha masks. A brand icon that was
        // embedded as-is would be opaque edge to edge and render as a black
        // rounded square; require real transparency AND real ink.
        assert!(clear > 200, "too little transparency ({clear} clear px) — did the backdrop get in?");
        assert!(opaque > 200, "too little ink ({opaque} opaque px) — is the mask empty?");
    }

    #[test]
    fn spawn_is_refused_when_no_main_pump_was_declared() {
        // No test in this module calls `mark_main_pump_available`, so the
        // flag is false here. A refusal is the honest answer: a queued
        // request nobody services would log "started" for an icon that
        // never appears.
        let r = spawn(std::env::temp_dir(), "deadbeef".into());
        assert!(r.is_err());
        assert!(PENDING.lock().unwrap().is_none(), "a refused spawn must not leave a request queued");
    }
}
