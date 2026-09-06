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

/// How often to re-check whether the background service is actually
/// reachable. Cheap (a loopback connect with a short timeout) and slow enough
/// to be invisible; the icon is a status indicator, not a monitor.
const STATUS_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// Is the background service actually reachable *right now*?
///
/// Deliberately the same question `forward_host_cmd` has to answer — port file
/// readable AND the host's IPC port accepting a connection — because that is
/// what the indicator should mean: "would Open work if you clicked it?".
/// Checking only that the port file exists would keep claiming "running" after
/// a host crash, since the file outlives a hard exit (see `lib.rs`, which
/// removes it only on a clean `run_message_loop` return).
fn service_reachable(data_dir: &std::path::Path, dir_hash: &str) -> bool {
    let port_file = data_dir.join(format!("ipc-port-{}", dir_hash));
    let Ok(contents) = std::fs::read_to_string(&port_file) else {
        return false;
    };
    let Some((port_str, _token)) = contents.trim().split_once(':') else {
        return false;
    };
    let Ok(port) = port_str.parse::<u16>() else {
        return false;
    };
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
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
    use muda::{Menu, MenuEvent, MenuItem as MudaItem};
    use tray_icon::{TrayIconBuilder, TrayIconEvent};

    // Start from the REAL state, not an assumption. The tray is created
    // before `srv`/`host` are spawned, so hard-coding "running" here would
    // make the icon claim the service is up during the entire startup window
    // — and again during every restart. WS4 requires this icon be a
    // *reliable* indicator, so it is driven from `service_reachable`
    // throughout (Codex P2 on PR #2996).
    let mut running = service_reachable(&data_dir, &dir_hash);
    let model = super::menu_model(running);
    let menu = Menu::new();
    // Keep the muda items alive alongside their action, so a menu event's id
    // can be mapped back to the action it came from. muda identifies
    // selections by item id, not by index.
    let mut items: Vec<(muda::MenuId, TrayAction)> = Vec::new();
    // The first item's label tracks running-state, so keep a handle to it.
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

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(super::tooltip(running))
        .with_icon(icon())
        .build()
        .map_err(|e| format!("build tray icon: {}", e))?;

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
                    std::thread::sleep(STATUS_POLL);
                    let now = service_reachable(&poll_dir, &poll_hash);
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

            // Reachability changed — refresh both surfaces the user reads.
            // Handled here (not in the poller) because muda/tray-icon objects
            // must only be touched on the thread that created them.
            if msg.message == WM_AGENTMUX_STATUS {
                running = msg.wParam != 0;
                let _ = tray.set_tooltip(Some(super::tooltip(running)));
                if let Some(item) = &open_item {
                    // Reuse the shared model so the label wording stays in one
                    // place (and stays covered by `tray_model_tests`).
                    if let Some(entry) = super::menu_model(running)
                        .into_iter()
                        .find(|e| e.action == TrayAction::OpenWindow)
                    {
                        item.set_text(entry.label);
                    }
                }
                crate::log(&format!("tray: service_reachable -> {}", running));
                continue;
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

/// Resource ordinal of the brand icon inside this exe.
///
/// `agentmux-launcher/build.rs` hands `agentmux-cef/resources/win/agentmux.ico`
/// to winres, which emits `1 ICON "..."` — ordinal 1. Kept as a named constant
/// so the coupling to build.rs is visible from here; `brand_icon_is_embedded`
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
