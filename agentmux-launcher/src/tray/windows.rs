// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows tray backend — issue #2977 Workstream 1.
//!
//! ## This introduces the launcher's first Win32 message pump
//!
//! §7.5 of the spec established that `agentmux-launcher` has **no** message
//! pump: zero `GetMessage`/`PeekMessage`/`DispatchMessage` calls anywhere. The
//! splash creates a real window but registers `DefWindowProcW` and polls,
//! which is only viable because it is `WS_EX_LAYERED | WS_EX_NOACTIVATE` and
//! takes no input — `UpdateLayeredWindow` composites with no `WM_PAINT`.
//!
//! A tray icon cannot work that way. `Shell_NotifyIcon` (which `tray-icon`
//! wraps) delivers clicks as window messages to a real window proc, and
//! `muda` menus need message dispatch to track selection. So this module owns
//! a dedicated thread that creates the icon and then pumps, for as long as the
//! process lives.
//!
//! ## Why a dedicated thread rather than the main thread
//!
//! The launcher's main thread runs the Tokio supervisor
//! (`supervisor/windows.rs`'s `select!` loop) which owns process lifetime —
//! restarts, Job Object teardown, the srv/host liveness probes. A blocking
//! `GetMessageW` loop cannot share that thread, and moving the supervisor
//! would be a far larger and riskier change than the tray warrants. Win32
//! requires only that the icon's messages be pumped by *the thread that
//! created it*, not specifically the main thread, so a dedicated thread is
//! both correct and the smallest change.
//!
//! The thread is intentionally detached and never joined: it exits with the
//! process. It communicates outward through an `mpsc::Sender<TrayAction>`,
//! which is its only contact with the supervisor — no shared locks, so it
//! cannot deadlock or stall the thing that supervises the app.

use std::sync::mpsc;

use super::TrayAction;

/// Spawn the tray thread. Returns the receiver for user actions.
///
/// Errors are returned rather than panicking: `start_if_enabled` degrades to
/// "no tray" on failure, because a cosmetic icon must never take down the
/// process that supervises `srv` and `host`.
pub fn spawn(
    data_dir: std::path::PathBuf,
    dir_hash: String,
) -> Result<mpsc::Receiver<TrayAction>, String> {
    let (tx, rx) = mpsc::channel::<TrayAction>();

    std::thread::Builder::new()
        .name("agentmux-tray".into())
        .spawn(move || {
            if let Err(e) = run(tx, data_dir, dir_hash) {
                crate::log(&format!("tray: thread exiting: {}", e));
            }
        })
        .map_err(|e| format!("spawn tray thread: {}", e))?;

    Ok(rx)
}

/// Custom message telling the tray thread that service reachability changed.
/// `WM_APP` is the documented base for application-private messages.
const WM_AGENTMUX_STATUS: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;
/// The srv Router published a new notification snapshot (`notify_menu::set`).
const WM_AGENTMUX_NOTIFY: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 2;

/// What a menu id maps back to: the original launcher actions, or the
/// notification hub's (SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24 §4.2).
#[derive(Clone)]
enum Act {
    Tray(TrayAction),
    Notify(super::notify_menu::NotifyMenuAction),
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Build the whole menu: notification section (if any), then the original
/// launcher items. Rebuilt wholesale on every change — it is a handful of
/// items, and rebuilding avoids tracking muda handles across submenus.
fn build_menu(
    running: bool,
    nstate: &super::notify_menu::NotifyTrayState,
) -> Result<(muda::Menu, Vec<(muda::MenuId, Act)>), String> {
    use super::notify_menu::NotifyMenuEntry;
    use muda::{Menu, MenuItem as MudaItem, PredefinedMenuItem, Submenu};

    let menu = Menu::new();
    let mut ids: Vec<(muda::MenuId, Act)> = Vec::new();
    for entry in super::notify_menu::menu(nstate, now_ms()) {
        match entry {
            NotifyMenuEntry::Item { label, action } => {
                let item = MudaItem::new(&label, true, None);
                ids.push((item.id().clone(), Act::Notify(action)));
                menu.append(&item).map_err(|e| e.to_string())?;
            }
            NotifyMenuEntry::Submenu { label, items } => {
                let sub = Submenu::new(&label, true);
                for child in items {
                    if let NotifyMenuEntry::Item { label, action } = child {
                        let item = MudaItem::new(&label, true, None);
                        ids.push((item.id().clone(), Act::Notify(action)));
                        sub.append(&item).map_err(|e| e.to_string())?;
                    }
                }
                menu.append(&sub).map_err(|e| e.to_string())?;
            }
            NotifyMenuEntry::Separator => {
                menu.append(&PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
            }
        }
    }
    for entry in super::menu_model(running) {
        let item = MudaItem::new(&entry.label, true, None);
        ids.push((item.id().clone(), Act::Tray(entry.action)));
        menu.append(&item).map_err(|e| format!("append menu item {:?}: {}", entry.label, e))?;
    }
    Ok((menu, ids))
}

fn full_tooltip(running: bool, nstate: &super::notify_menu::NotifyTrayState) -> String {
    let base = super::tooltip(running);
    match super::notify_menu::tooltip_suffix(nstate, now_ms()) {
        Some(extra) => format!("{base}\n{extra}"),
        None => base,
    }
}

/// Create the icon and pump messages until the process ends.
///
/// Everything here runs on the tray thread. `tray-icon` and `muda` both
/// require that their objects be created on, and pumped by, the same thread —
/// hence construction happens inside this function rather than being passed
/// in from `spawn`.
fn run(
    tx: mpsc::Sender<TrayAction>,
    data_dir: std::path::PathBuf,
    dir_hash: String,
) -> Result<(), String> {
    use muda::MenuEvent;
    use tray_icon::{TrayIconBuilder, TrayIconEvent};

    // Start from the REAL state, not an assumption. The tray is created
    // before `srv`/`host` are spawned, so hard-coding "running" here would
    // make the icon claim the service is up during the entire startup window
    // — and again during every restart. WS4 requires this icon be a
    // *reliable* indicator, so it is driven from `service_reachable`
    // throughout (Codex P2 on PR #2996).
    let mut running = super::service_reachable(&data_dir, &dir_hash);
    let mut nstate = super::notify_menu::get();
    let (menu, mut items) = build_menu(running, &nstate)?;
    let base_icon = icon();
    let badge_icon = attention_icon();
    let pick_icon = |n: &super::notify_menu::NotifyTrayState| -> tray_icon::Icon {
        match (&badge_icon, n.attention.is_empty()) {
            (Some(b), false) => b.clone(),
            _ => base_icon.clone(),
        }
    };

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(full_tooltip(running, &nstate))
        .with_icon(pick_icon(&nstate))
        .build()
        .map_err(|e| format!("build tray icon: {}", e))?;

    // Wake this thread when srv publishes a new notification snapshot.
    unsafe {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        let tray_thread = GetCurrentThreadId();
        super::notify_menu::set_wake(Box::new(move || {
            windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(tray_thread, WM_AGENTMUX_NOTIFY, 0, 0);
        }));
    }

    // Status poller. Runs off the tray thread (it does blocking I/O and must
    // not stall the pump), and nudges the pump via a thread message when the
    // answer changes. `PostThreadMessageW` is delivered to `GetMessageW` with
    // a null hwnd, which is exactly what the pump below already does — so the
    // pump we had to introduce for the icon also earns its keep here.
    unsafe {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        let tray_thread = GetCurrentThreadId();
        let poll_dir = data_dir.clone();
        let poll_hash = dir_hash.clone();
        std::thread::Builder::new()
            .name("agentmux-tray-status".into())
            .spawn(move || {
                let mut last = running;
                loop {
                    std::thread::sleep(super::STATUS_POLL);
                    let now = super::service_reachable(&poll_dir, &poll_hash);
                    if now != last {
                        last = now;
                        // wparam carries the new state; the pump reads it
                        // rather than re-probing, so the displayed value is
                        // exactly the one that was observed.
                        PostThreadMessageW(
                            tray_thread,
                            WM_AGENTMUX_STATUS,
                            now as usize,
                            0,
                        );
                    }
                }
            })
            .ok();
    }

    crate::log(&format!(
        "tray: icon created (service_reachable={}); entering message pump",
        running
    ));

    // The pump. This is the launcher's first, and deliberately only, Win32
    // message loop (§7.5). `GetMessageW` blocks, so this thread costs nothing
    // while idle — it is not a spin loop like the splash's.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG,
        };
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let got = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            // 0 = WM_QUIT, -1 = error. Either way, stop pumping; the process
            // is going down or the queue is unusable.
            if got == 0 || got == -1 {
                break;
            }

            // Reachability or notification state changed — refresh every
            // surface the user reads. Handled here (not in the poller) because
            // muda/tray-icon objects must only be touched on the thread that
            // created them.
            if msg.message == WM_AGENTMUX_STATUS || msg.message == WM_AGENTMUX_NOTIFY {
                if msg.message == WM_AGENTMUX_STATUS {
                    running = msg.wParam != 0;
                    crate::log(&format!("tray: service_reachable -> {}", running));
                }
                nstate = super::notify_menu::get();
                match build_menu(running, &nstate) {
                    Ok((menu, ids)) => {
                        tray.set_menu(Some(Box::new(menu)));
                        items = ids;
                    }
                    Err(e) => crate::log(&format!("tray: menu rebuild failed: {e}")),
                }
                let _ = tray.set_tooltip(Some(full_tooltip(running, &nstate)));
                let _ = tray.set_icon(Some(pick_icon(&nstate)));
                continue;
            }

            TranslateMessage(&msg);
            DispatchMessageW(&msg);

            // Drain whatever the dispatch produced. Both crates deliver
            // through process-global channels rather than the message itself,
            // so they are polled here after dispatch rather than decoded from
            // `msg`.
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                match items.iter().find(|(id, _)| *id == ev.id).map(|(_, a)| a.clone()) {
                    Some(Act::Tray(action)) => {
                        if tx.send(action).is_err() {
                            // Receiver gone — the supervisor is shutting down.
                            return Ok(());
                        }
                    }
                    Some(Act::Notify(action)) => crate::notify::tray_request(action),
                    None => {}
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                // A left click on the icon is the same intent as the first
                // menu item; right-click opens the menu and is handled by the
                // menu path above.
                if let TrayIconEvent::Click { button, button_state, .. } = ev {
                    if button == tray_icon::MouseButton::Left && button_state == tray_icon::MouseButtonState::Up {
                        // Something needs you → go straight to the oldest such
                        // pane, same as clicking its toast (spec §4.2).
                        if let Some(first) = nstate.attention.first() {
                            crate::notify::tray_request(super::notify_menu::NotifyMenuAction::Open(first.id.clone()));
                        } else if tx.send(TrayAction::OpenWindow).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Resource ordinal of the brand icon inside this exe.
///
/// `agentmux-launcher/build.rs` hands `agentmux-cef/resources/win/agentmux.ico`
/// to winres, which emits `1 ICON "..."` — ordinal 1. Kept as a named constant
/// so the coupling to build.rs is visible from here;
/// `the_icon_asset_build_rs_embeds_actually_exists`
/// guards the half of that coupling a test can actually reach.
const BRAND_ICON_ORDINAL: u16 = 1;

/// The tray icon.
///
/// Loaded from this exe own resource table rather than generated or read from
/// disk. Three reasons this beats the alternatives:
///
/// - The brand icon is ALREADY embedded — `build.rs` gives the same
///   `agentmux.ico` to winres that the exe uses for Explorer/Alt-Tab. Nothing
///   new to ship, and the tray cannot drift from the app icon.
/// - A portable install has no icon file next to the binary, so a
///   disk-loading variant would work in dev and fail once packaged. This is
///   the failure mode the previous placeholder existed to avoid.
/// - `.ico` carries several sizes; passing the small-icon metrics lets Windows
///   pick the frame that matches the current DPI instead of us downscaling a
///   256x256 PNG and getting a blurry mark on HiDPI.
///
/// Falls back to a generated square if the resource is missing (see
/// `fallback_icon`), because a cosmetic icon must never take down the tray —
/// but that fallback is logged loudly, since a silent fallback would look
/// exactly like "the brand icon just doesn't work".
fn icon() -> tray_icon::Icon {
    match brand_icon() {
        Some(icon) => icon,
        None => {
            crate::log(
                "tray: brand icon resource not found in this exe — falling back \
                 to the generated mark (check build.rs res.set_icon)",
            );
            fallback_icon()
        }
    }
}

/// Load ordinal `BRAND_ICON_ORDINAL` at the system small-icon size.
///
/// Size is requested explicitly rather than passing `None`: `None` maps to
/// `LR_DEFAULTSIZE`, which resolves to `SM_CXICON` (the 32x32 *large* icon
/// metric), leaving Windows to shrink a 32px frame into a 16px tray slot. The
/// small-icon metrics select the frame the `.ico` already contains at that
/// size, and follow DPI scaling on their own.
fn brand_icon() -> Option<tray_icon::Icon> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXSMICON, SM_CYSMICON,
    };

    let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
    // Guard the metrics: a zero/negative value would ask LoadImageW for a
    // degenerate size. Falling back to LR_DEFAULTSIZE is still better than
    // losing the brand icon entirely.
    let size = if cx > 0 && cy > 0 { Some((cx as u32, cy as u32)) } else { None };

    match tray_icon::Icon::from_resource(BRAND_ICON_ORDINAL, size) {
        Ok(icon) => Some(icon),
        Err(e) => {
            crate::log(&format!("tray: loading brand icon resource failed: {}", e));
            None
        }
    }
}

/// Brand mark with an attention dot, for "an agent needs you" (spec §4.2).
///
/// Built from the embedded brand PNG (the tray's normal icon comes from the
/// exe's `.ico` resource, which exposes no pixels), box-downscaled to the
/// small-icon metric, with an orange dot in the bottom-right quadrant and a
/// dark ring so it reads on light and dark taskbars. `None` on any decode
/// failure — the tray then just keeps the normal icon (the tooltip and menu
/// still carry the state).
fn attention_icon() -> Option<tray_icon::Icon> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    const PNG: &[u8] = include_bytes!("../../../assets/favicon-150x150.png");
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as u32;
    let rgba = badge_rgba(PNG, size)?;
    tray_icon::Icon::from_rgba(rgba, size, size).ok()
}

/// Decode `png_bytes`, downscale to `size`², and paint the attention dot.
pub(super) fn badge_rgba(png_bytes: &[u8], size: u32) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (sw, sh) = (info.width as usize, info.height as usize);
    let chans = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::GrayscaleAlpha => 2,
        _ => return None,
    };
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            // Box filter over the source rectangle this pixel covers.
            let (x0, x1) = (x * sw / n, ((x + 1) * sw / n).max(x * sw / n + 1));
            let (y0, y1) = (y * sh / n, ((y + 1) * sh / n).max(y * sh / n + 1));
            let mut acc = [0u32; 4];
            let mut cnt = 0u32;
            for sy in y0..y1.min(sh) {
                for sx in x0..x1.min(sw) {
                    let i = (sy * sw + sx) * chans;
                    let px = if chans == 4 {
                        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
                    } else {
                        [buf[i], buf[i], buf[i], buf[i + 1]]
                    };
                    for c in 0..4 {
                        acc[c] += px[c] as u32;
                    }
                    cnt += 1;
                }
            }
            let o = (y * n + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / cnt.max(1)) as u8;
            }
        }
    }
    // Dot: centre in the bottom-right, radius ~ size/4, 1px dark ring.
    let r = (n as f32) * 0.26;
    let (cx, cy) = (n as f32 - r - 0.5, n as f32 - r - 0.5);
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let o = (y * n + x) * 4;
            if d <= r - 1.0 {
                out[o..o + 4].copy_from_slice(&[0xFF, 0x8A, 0x00, 0xFF]);
            } else if d <= r {
                out[o..o + 4].copy_from_slice(&[0x1A, 0x1A, 0x1A, 0xFF]);
            }
        }
    }
    Some(out)
}

/// Last-resort mark, used only when the exe carries no icon resource.
///
/// Deliberately NOT the brand colour: if this ever shows up it means the
/// resource lookup failed, and it should be obvious at a glance rather than
/// passing for the real icon.
fn fallback_icon() -> tray_icon::Icon {
    const S: u32 = 16;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            let corner = (x < 2 || x >= S - 2) && (y < 2 || y >= S - 2);
            if corner {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                rgba.extend_from_slice(&[0xFF, 0x6B, 0x00, 0xFF]);
            }
        }
    }
    // Dimensions match the buffer by construction, so this cannot fail; if it
    // somehow did, an unwrap here would kill the tray thread only, which
    // `spawn`'s caller already treats as "no tray".
    tray_icon::Icon::from_rgba(rgba, S, S).expect("16x16 RGBA buffer is well-formed")
}

#[cfg(test)]
mod brand_icon_tests {
    /// `build.rs` embeds the icon only `if icon_path.exists()` — a silent skip.
    /// If the asset moves, the exe loses its icon resource, `from_resource`
    /// fails at runtime, and the tray quietly shows the fallback mark instead.
    /// Nothing else in the build would complain, so assert the path here.
    ///
    /// This is the only half of the build.rs coupling a unit test can reach:
    /// the ordinal itself is decided by winres at build time and is verified
    /// live (the icon either renders as the brand mark or it does not).
    #[test]
    fn the_icon_asset_build_rs_embeds_actually_exists() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agentmux-cef/resources/win/agentmux.ico");
        assert!(
            path.exists(),
            "build.rs embeds {} only if it exists; it does not, so the tray \
             would silently fall back",
            path.display()
        );
    }
}

#[cfg(test)]
mod attention_badge_tests {
    #[test]
    fn badge_decodes_scales_and_paints_the_dot() {
        let png = include_bytes!("../../../assets/favicon-150x150.png");
        for size in [16u32, 20, 24, 32] {
            let px = super::badge_rgba(png, size).expect("decodes");
            assert_eq!(px.len(), (size * size * 4) as usize);
            // Bottom-right-ish pixel inside the dot is the attention orange.
            let n = size as usize;
            let (x, y) = (n - 1 - n / 5, n - 1 - n / 5);
            let o = (y * n + x) * 4;
            assert_eq!(&px[o..o + 4], &[0xFF, 0x8A, 0x00, 0xFF], "size {size}");
        }
    }
}
