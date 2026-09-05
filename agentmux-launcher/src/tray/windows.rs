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
pub fn spawn() -> Result<mpsc::Receiver<TrayAction>, String> {
    let (tx, rx) = mpsc::channel::<TrayAction>();

    std::thread::Builder::new()
        .name("agentmux-tray".into())
        .spawn(move || {
            if let Err(e) = run(tx) {
                crate::log(&format!("tray: thread exiting: {}", e));
            }
        })
        .map_err(|e| format!("spawn tray thread: {}", e))?;

    Ok(rx)
}

/// Create the icon and pump messages until the process ends.
///
/// Everything here runs on the tray thread. `tray-icon` and `muda` both
/// require that their objects be created on, and pumped by, the same thread —
/// hence construction happens inside this function rather than being passed
/// in from `spawn`.
fn run(tx: mpsc::Sender<TrayAction>) -> Result<(), String> {
    use muda::{Menu, MenuEvent, MenuItem as MudaItem};
    use tray_icon::{TrayIconBuilder, TrayIconEvent};

    // Build the menu from the platform-neutral model so the labels/ordering
    // are the ones covered by `tray_model_tests`, not a second copy.
    // `running` is true here: the tray only starts when background-service
    // mode is on and the launcher is up, which is exactly "running".
    let model = super::menu_model(true);
    let menu = Menu::new();
    // Keep the muda items alive alongside their action, so a menu event's id
    // can be mapped back to the action it came from. muda identifies
    // selections by item id, not by index.
    let mut items: Vec<(muda::MenuId, TrayAction)> = Vec::new();
    for entry in &model {
        let item = MudaItem::new(&entry.label, true, None);
        items.push((item.id().clone(), entry.action));
        menu.append(&item)
            .map_err(|e| format!("append menu item {:?}: {}", entry.label, e))?;
    }

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(super::tooltip(true))
        .with_icon(icon())
        .build()
        .map_err(|e| format!("build tray icon: {}", e))?;

    crate::log("tray: icon created; entering message pump");

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
            TranslateMessage(&msg);
            DispatchMessageW(&msg);

            // Drain whatever the dispatch produced. Both crates deliver
            // through process-global channels rather than the message itself,
            // so they are polled here after dispatch rather than decoded from
            // `msg`.
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                if let Some(action) = items.iter().find(|(id, _)| *id == ev.id).map(|(_, a)| *a) {
                    if tx.send(action).is_err() {
                        // Receiver gone — the supervisor is shutting down.
                        return Ok(());
                    }
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                // A left click on the icon is the same intent as the first
                // menu item; right-click opens the menu and is handled by the
                // menu path above.
                if let TrayIconEvent::Click { button, .. } = ev {
                    if button == tray_icon::MouseButton::Left
                        && tx.send(TrayAction::OpenWindow).is_err()
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

/// The icon bitmap.
///
/// Generated rather than loaded from disk: the launcher has no icon asset it
/// can rely on at runtime (the exe's own icon is injected post-build by
/// `scripts/inject-exe-icon.sh`, and is not readable as a file next to the
/// binary in a portable install). A flat, recognizable mark avoids adding a
/// packaging dependency for the prototype; replacing it with the real brand
/// icon is a packaging task, not a code one.
fn icon() -> tray_icon::Icon {
    const S: u32 = 16;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            // A filled rounded square in AgentMux's accent orange, transparent
            // at the corners so it reads as a mark rather than a block.
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
