// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux tray backend — StatusNotifierItem over D-Bus via `ksni`
//! (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §2, §7.6;
//! notification hub per `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §4.2).
//!
//! `ksni` rather than `tray-icon`'s GTK path: no second toolkit loop next to
//! CEF's. The service itself runs on ksni's thread; we add one small thread
//! to own its blocking handle (see `spawn` for why it can't live in Tokio).
//!
//! Host caveats (documented, not fixable in-app): GNOME needs the
//! "AppIndicator and KStatusNotifierItem Support" extension (Ubuntu ships it
//! on); some SNI hosts never deliver left-click `activate`, and the GNOME
//! extension shows no tooltips — so everything important is in the MENU.

use std::sync::mpsc;

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};

use super::notify_menu::{self, NotifyMenuEntry, NotifyTrayState};
use super::TrayAction;

struct AgentMuxTray {
    tx: mpsc::Sender<TrayAction>,
    running: bool,
    nstate: NotifyTrayState,
    /// `app:startatlogin`, refreshed with the notification state.
    start_at_login: Option<bool>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// SNI menu labels treat `_` as a mnemonic marker (the `&` of muda) —
/// agent names are user-chosen, so double it. `notify_menu::menu_label`
/// already escaped `&` for muda; SNI shows `&&` literally, so undo that.
fn sni_label(s: &str) -> String {
    s.replace("&&", "&").replace('_', "__")
}

fn notify_items(entries: Vec<NotifyMenuEntry>) -> Vec<MenuItem<AgentMuxTray>> {
    entries
        .into_iter()
        .map(|e| match e {
            NotifyMenuEntry::Item { label, action } => StandardItem {
                label: sni_label(&label),
                activate: Box::new(move |_: &mut AgentMuxTray| crate::notify::tray_request(action.clone())),
                ..Default::default()
            }
            .into(),
            NotifyMenuEntry::Submenu { label, items } => SubMenu {
                label: sni_label(&label),
                submenu: notify_items(items),
                ..Default::default()
            }
            .into(),
            NotifyMenuEntry::Separator => MenuItem::Separator,
        })
        .collect()
}

impl ksni::Tray for AgentMuxTray {
    fn id(&self) -> String {
        "agentmux".into()
    }

    fn title(&self) -> String {
        "AgentMux".into()
    }

    fn icon_name(&self) -> String {
        // Installed by the .desktop / AppImage integration; the pixmap below
        // is the fallback for hosts that can't resolve it.
        "agentmux".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        brand_pixmap().into_iter().collect()
    }

    fn status(&self) -> ksni::Status {
        if self.nstate.attention.is_empty() {
            ksni::Status::Active
        } else {
            ksni::Status::NeedsAttention
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: super::tooltip(self.running),
            description: notify_menu::tooltip_suffix(&self.nstate, now_ms()).unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Left-click: the oldest thing that needs you, else a new window.
    fn activate(&mut self, _x: i32, _y: i32) {
        match self.nstate.attention.first() {
            Some(first) => crate::notify::tray_request(notify_menu::NotifyMenuAction::Open(first.id.clone())),
            None => {
                let _ = self.tx.send(TrayAction::OpenWindow);
            }
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = notify_items(notify_menu::menu(&self.nstate, now_ms()));
        for entry in super::menu_model(self.running, self.start_at_login) {
            let action = entry.action;
            let activate = Box::new(move |t: &mut AgentMuxTray| {
                let _ = t.tx.send(action);
            });
            items.push(match entry.check {
                Some(check) => CheckmarkItem {
                    label: sni_label(&entry.label),
                    enabled: check.enabled,
                    checked: check.checked,
                    activate,
                    ..Default::default()
                }
                .into(),
                None => StandardItem { label: sni_label(&entry.label), activate, ..Default::default() }.into(),
            });
        }
        items
    }
}

/// The brand mark as an SNI pixmap (ARGB32, network byte order).
fn brand_pixmap() -> Option<ksni::Icon> {
    const PNG: &[u8] = include_bytes!("../../../assets/favicon-71x71.png");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(PNG));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    let data = rgba_to_argb(&buf[..(info.width * info.height * 4) as usize]);
    Some(ksni::Icon { width: info.width as i32, height: info.height as i32, data })
}

/// RGBA → ARGB (big-endian), as the SNI `IconPixmap` property wants.
pub fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    out
}

/// Updates the ksni thread applies to the live tray.
enum Update {
    Notify,
    Running(bool),
}

/// ksni's `blocking` API drives its own current-thread Tokio runtime
/// (`RUNTIME.block_on`), which panics if entered from inside another runtime —
/// and both our callers are (`run_unix` for spawn, the notify task for
/// updates). So the tray lives entirely on its own std thread; everyone else
/// only sends it `Update`s.
pub fn spawn(data_dir: std::path::PathBuf, dir_hash: String) -> Result<mpsc::Receiver<TrayAction>, String> {
    let (tx, rx) = mpsc::channel();
    let (utx, urx) = mpsc::channel::<Update>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let running = super::service_reachable(&data_dir, &dir_hash);

    std::thread::Builder::new()
        .name("agentmux-tray".into())
        .spawn(move || {
            let tray = AgentMuxTray {
                tx,
                running,
                nstate: notify_menu::get(),
                start_at_login: crate::start_at_login::current(),
            };
            let handle = match tray.spawn() {
                Ok(h) => h,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("ksni spawn: {e}")));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            while let Ok(u) = urx.recv() {
                let applied = match u {
                    Update::Notify => handle.update(|t: &mut AgentMuxTray| {
                        t.nstate = notify_menu::get();
                        t.start_at_login = crate::start_at_login::current();
                    }),
                    Update::Running(r) => handle.update(|t: &mut AgentMuxTray| t.running = r),
                };
                if applied.is_none() {
                    return; // tray service gone
                }
            }
        })
        .map_err(|e| e.to_string())?;
    // Bounded, like the Windows toast backend: a missing/wedged session bus
    // must not stall the supervisor.
    match ready_rx.recv_timeout(super::READY_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("tray did not start within 5s".into()),
    }

    let wake_tx = utx.clone();
    notify_menu::set_wake(Box::new(move || {
        let _ = wake_tx.send(Update::Notify);
    }));

    // Reachability poller — same contract as the Windows backend.
    std::thread::Builder::new()
        .name("agentmux-tray-status".into())
        .spawn(move || {
            let mut last = running;
            loop {
                std::thread::sleep(super::STATUS_POLL);
                let now = super::service_reachable(&data_dir, &dir_hash);
                if now != last {
                    last = now;
                    if utx.send(Update::Running(now)).is_err() {
                        return;
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_conversion() {
        assert_eq!(rgba_to_argb(&[1, 2, 3, 4, 5, 6, 7, 8]), vec![4, 1, 2, 3, 8, 5, 6, 7]);
    }

    #[test]
    fn sni_labels_escape_underscores_and_unescape_ampersands() {
        assert_eq!(sni_label("my_agent needs you"), "my__agent needs you");
        assert_eq!(sni_label("R&&D stopped"), "R&D stopped");
    }

    #[test]
    fn brand_pixmap_decodes() {
        let icon = brand_pixmap().expect("embedded PNG decodes");
        assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
    }
}
