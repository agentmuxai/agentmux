// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows toast backend — `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §6.1.
//!
//! Plain WinRT `Windows.UI.Notifications` through the `windows` crate — not
//! the Windows App SDK (archived Rust bindings, `Register()` broken for
//! unpackaged apps) and not `tauri-winrt-notification` (drops the toast
//! object, so `Activated` never fires).
//!
//! ## Identity
//!
//! An unpackaged Win32 app needs a registered AppUserModelID or Windows
//! silently drops its toasts (Warp #10187). We self-register
//! `HKCU\Software\Classes\AppUserModelId\AgentMuxCorp.AgentMux` — the same
//! AUMID the host sets for taskbar grouping — with a display name and a PNG
//! icon written next to the data dir. No elevation needed. MSIX installs have
//! their own package identity; that path is not handled here yet.
//!
//! ## Threading
//!
//! One dedicated MTA thread owns every WinRT object. Toast event handlers
//! fire on WinRT pool threads; they only forward into a channel. The thread
//! keeps each live `ToastNotification` in a map — dropping it would drop the
//! `Activated` handler with it.
//!
//! ## Safety of content
//!
//! The XML skeleton is a constant; title/body are inserted with
//! `CreateTextNode`, so no agent-influenced text is ever parsed as XML
//! (spec §9.1/§9.3). The `launch` argument carries only our own notification
//! id, and it isn't what activation trusts anyway — the handler closure
//! captures the id.
//!
//! ## A pending shutdown's toast
//!
//! It stays on screen for the whole window (`scenario="reminder"`), has one
//! Keep running button (`arguments="keep"`, a constant: the id of the
//! notification comes from the handler's closure, never from the arguments),
//! and counts down: the title and a progress bar are data-bound, and the
//! shared ticker (`notify/mod.rs`) redraws them once a second with
//! `UpdateWithTagAndGroup`. The bound values are plain strings in
//! `NotificationData`, never parsed as XML.
//! (docs/specs/SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md §3.8)

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;

use tokio::sync::mpsc;
use windows::core::{IInspectable, Interface, HSTRING};
use windows::Data::Xml::Dom::{XmlDocument, XmlElement};
use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
use windows::UI::Notifications::{
    NotificationData, NotificationUpdateResult, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
    ToastNotification, ToastNotificationManager, ToastNotificationPriority, ToastNotifier,
};

use super::{countdown_update, CountdownUpdate, Notification, Presenter, UserAction};

/// Must match the host's `SetCurrentProcessExplicitAppUserModelID`
/// (`crates/cef/src/lib.rs`) so toasts and taskbar grouping agree.
pub const AUMID: &str = "AgentMuxCorp.AgentMux";

/// This instance's toast group. Every AgentMux instance on the machine (the
/// installed app, a second channel, a `task dev` build) shares `AUMID`, so a
/// shared group let one instance's startup clear, or a same-tag "N more
/// updates" toast, wipe or replace another instance's notifications. Keyed by
/// the instance's data dir, so a restart of the SAME instance still clears
/// what its crashed predecessor left behind. 15 chars: under the 16-char
/// Group limit of older Windows 10 builds.
pub fn group_for(data_dir: &std::path::Path) -> String {
    let key = data_dir.to_string_lossy().to_lowercase();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("am-{:012x}", h & 0xffff_ffff_ffff)
}
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/favicon-150x150.png");
/// Upper bound on waiting for the WinRT thread at startup (see `spawn`).
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

enum Cmd {
    Show(Notification),
    Update(String, CountdownUpdate),
    Retract(String),
    ClearAll(std_mpsc::Sender<()>),
}

pub struct WindowsPresenter {
    tx: std_mpsc::Sender<Cmd>,
}

impl WindowsPresenter {
    pub fn spawn(actions: mpsc::UnboundedSender<UserAction>, data_dir: &std::path::Path) -> Result<Self, String> {
        let icon_path = data_dir.join("notify-icon.png");
        if let Err(e) = std::fs::write(&icon_path, ICON_PNG) {
            crate::logging::log(&format!("notify: could not write toast icon: {e}"));
        }
        if let Err(e) = register_aumid(&icon_path) {
            crate::logging::log(&format!("notify: AUMID registration failed: {e}"));
        }
        let (tx, rx) = std_mpsc::channel();
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<(), String>>();
        let group = group_for(data_dir);
        std::thread::Builder::new()
            .name("agentmux-notify".into())
            .spawn(move || thread_main(rx, actions, ready_tx, group))
            .map_err(|e| e.to_string())?;
        // Bounded: this runs on the supervisor's startup path before the host
        // is spawned. If WinRT wedges (RoInitialize / notifier creation never
        // returns), fall back to "no toasts" rather than hang the launcher.
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(e)) => Err(e),
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                Err(format!("toast backend not ready after {READY_TIMEOUT:?}; continuing without toasts"))
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

impl Presenter for WindowsPresenter {
    fn show(&self, n: &Notification) {
        let _ = self.tx.send(Cmd::Show(n.clone()));
    }
    fn retract(&self, tag: &str) {
        let _ = self.tx.send(Cmd::Retract(tag.to_string()));
    }
    fn update(&self, tag: &str, update: &CountdownUpdate) {
        let _ = self.tx.send(Cmd::Update(tag.to_string(), update.clone()));
    }
    /// This instance's toasts only (`group_for`) — never another instance's.
    fn clear_all(&self) {
        // Synchronous (bounded): called on launcher exit, which must not race
        // past it.
        let (done_tx, done_rx) = std_mpsc::channel();
        if self.tx.send(Cmd::ClearAll(done_tx)).is_ok() {
            let _ = done_rx.recv_timeout(std::time::Duration::from_secs(2));
        }
    }
}

fn thread_main(
    rx: std_mpsc::Receiver<Cmd>,
    actions: mpsc::UnboundedSender<UserAction>,
    ready: std_mpsc::Sender<Result<(), String>>,
    group: String,
) {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        let _ = ready.send(Err(format!("RoInitialize: {e}")));
        return;
    }
    let aumid = HSTRING::from(AUMID);
    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&aumid) {
        Ok(n) => n,
        Err(e) => {
            let _ = ready.send(Err(format!("CreateToastNotifierWithId: {e}")));
            return;
        }
    };
    if let Ok(setting) = notifier.Setting() {
        crate::logging::log(&format!("notify: Windows toast backend ready (setting={:?})", setting.0));
    }
    let _ = ready.send(Ok(()));

    let mut live: HashMap<String, ToastNotification> = HashMap::new();
    // tag → the sequence number of the last countdown update (Windows drops an
    // update that isn't newer), and the tags the user already took down.
    let mut seq: HashMap<String, u32> = HashMap::new();
    let mut gone: std::collections::HashSet<String> = Default::default();
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Show(n) => match show(&notifier, &n, &actions, &group) {
                Ok(toast) => {
                    gone.remove(&n.tag);
                    seq.insert(n.tag.clone(), 1);
                    live.insert(n.tag.clone(), toast);
                }
                Err(e) => crate::logging::log(&format!("notify: show failed: {e}")),
            },
            Cmd::Update(tag, update) => {
                if gone.contains(&tag) || !live.contains_key(&tag) {
                    continue;
                }
                let next = seq.entry(tag.clone()).and_modify(|n| *n += 1).or_insert(2);
                match update_toast(&notifier, &tag, &group, &update, *next) {
                    Ok(NotificationUpdateResult::NotificationNotFound) => {
                        // Swiped away: stop redrawing a toast that isn't there.
                        crate::logging::log("notify: countdown toast was dismissed; no more updates");
                        gone.insert(tag);
                    }
                    Ok(_) => {}
                    Err(e) => crate::logging::log(&format!("notify: countdown update failed: {e}")),
                }
            }
            Cmd::Retract(tag) => {
                live.remove(&tag);
                seq.remove(&tag);
                gone.remove(&tag);
                if let Ok(h) = ToastNotificationManager::History() {
                    let _ = h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(group.as_str()), &aumid);
                }
            }
            Cmd::ClearAll(done) => {
                live.clear();
                seq.clear();
                gone.clear();
                if let Ok(h) = ToastNotificationManager::History() {
                    let _ = h.RemoveGroupWithId(&HSTRING::from(group.as_str()), &aumid);
                }
                let _ = done.send(());
            }
        }
    }
}

/// Build the toast document: constant skeleton, text via text nodes.
pub fn build_xml(n: &Notification) -> windows::core::Result<XmlDocument> {
    let doc = XmlDocument::new()?;
    let live = n.countdown.is_some();
    let buttons = n.buttons();
    // Title, then the body, then the summary in the `attribution` slot, which
    // Windows renders small and grey under the body without using up one of
    // ToastGeneric's three text lines (rich-content spec §1.3). The skeleton
    // is fixed markup; every line is filled as a text node below. A toast with
    // a countdown binds its title and bar to `NotificationData` instead
    // (`data_for`), and one with buttons stays up for its whole
    // window (`scenario="reminder"`).
    let mut skeleton = String::from("<toast");
    if !buttons.is_empty() {
        skeleton.push_str(r#" scenario="reminder""#);
    }
    skeleton.push_str(r#"><visual><binding template="ToastGeneric">"#);
    skeleton.push_str(if live { "<text>{title}</text>" } else { "<text/>" });
    if n.body.is_some() {
        skeleton.push_str("<text/>");
    }
    if n.summary.is_some() {
        skeleton.push_str(r#"<text placement="attribution"/>"#);
    }
    if live {
        skeleton.push_str(r#"<progress value="{progressValue}" valueStringOverride="{progressValueString}" status="Closing"/>"#);
    }
    skeleton.push_str("</binding></visual>");
    if !buttons.is_empty() {
        skeleton.push_str("<actions>");
        for (id, _) in &buttons {
            skeleton.push_str(&format!(r#"<action content="" arguments="{id}" activationType="foreground"/>"#));
        }
        skeleton.push_str("</actions>");
    }
    skeleton.push_str(r#"<audio silent="true"/></toast>"#);
    doc.LoadXml(&HSTRING::from(skeleton))?;
    let root = doc.DocumentElement()?;
    root.SetAttribute(&HSTRING::from("launch"), &HSTRING::from(format!("agentmux-notify:{}", n.id)))?;
    let texts = doc.GetElementsByTagName(&HSTRING::from("text"))?;
    let lines = std::iter::once(n.title.as_str()).chain(n.body.as_deref()).chain(n.summary.as_deref());
    for (i, line) in lines.enumerate() {
        if live && i == 0 {
            continue; // the title is data-bound
        }
        let node = texts.Item(i as u32)?;
        node.AppendChild(&doc.CreateTextNode(&HSTRING::from(line))?)?;
    }
    for (i, (_, label)) in buttons.iter().enumerate() {
        let button: XmlElement = doc.GetElementsByTagName(&HSTRING::from("action"))?.Item(i as u32)?.cast()?;
        button.SetAttribute(&HSTRING::from("content"), &HSTRING::from(*label))?;
    }
    Ok(doc)
}

/// What the buttons' `arguments` are. Constants: which notification a press belongs to comes from
/// the handler's closure.
const KEEP_ARGUMENT: &str = "keep";
const NOW_ARGUMENT: &str = "now";

/// What a toast activation was.
#[derive(Debug, PartialEq, Eq)]
pub enum Activation {
    /// The cancel button (Keep running, ...).
    Keep,
    /// The do-it-now button (Close now, ...).
    Now,
    /// The toast itself: open the pane.
    Body,
}

/// Only the two constant arguments are a button press; anything else is a click
/// on the body.
pub fn activation_for(arguments: &str) -> Activation {
    match arguments {
        KEEP_ARGUMENT => Activation::Keep,
        NOW_ARGUMENT => Activation::Now,
        _ => Activation::Body,
    }
}

/// The bound values of a countdown toast, as `NotificationData`.
fn data_for(u: &CountdownUpdate, sequence: u32) -> windows::core::Result<NotificationData> {
    let data = NotificationData::new()?;
    let values = data.Values()?;
    values.Insert(&HSTRING::from("title"), &HSTRING::from(u.title.as_str()))?;
    values.Insert(&HSTRING::from("progressValue"), &HSTRING::from(format!("{:.3}", u.fraction_left)))?;
    values.Insert(&HSTRING::from("progressValueString"), &HSTRING::from(u.seconds_text.as_str()))?;
    data.SetSequenceNumber(sequence)?;
    Ok(data)
}

fn update_toast(
    notifier: &ToastNotifier,
    tag: &str,
    group: &str,
    u: &CountdownUpdate,
    sequence: u32,
) -> windows::core::Result<NotificationUpdateResult> {
    notifier.UpdateWithTagAndGroup(&data_for(u, sequence)?, &HSTRING::from(tag), &HSTRING::from(group))
}

/// Take the toast out of the Action Center on its own shortly after the window
/// ends, so one nobody answered doesn't linger there once the agent is gone.
fn expire_after(toast: &ToastNotification, ms: i64) -> windows::core::Result<()> {
    // FILETIME epoch (1601) in 100 ns ticks.
    const UNIX_TO_FILETIME_TICKS: i64 = 116_444_736_000_000_000;
    let unix_ms = agentmux_common::time::now_ms();
    let at = DateTime { UniversalTime: (unix_ms + ms) * 10_000 + UNIX_TO_FILETIME_TICKS };
    let at: IReference<DateTime> = PropertyValue::CreateDateTime(at)?.cast()?;
    toast.SetExpirationTime(&at)
}

fn show(
    notifier: &ToastNotifier,
    n: &Notification,
    actions: &mpsc::UnboundedSender<UserAction>,
    group: &str,
) -> windows::core::Result<ToastNotification> {
    let toast = ToastNotification::CreateToastNotification(&build_xml(n)?)?;
    toast.SetTag(&HSTRING::from(n.tag.as_str()))?;
    toast.SetGroup(&HSTRING::from(group))?;
    if n.is_attention() {
        let _ = toast.SetPriority(ToastNotificationPriority::High);
    }
    if let Some(c) = &n.countdown {
        toast.SetData(&data_for(&countdown_update(&c.title_template, c.remaining_ms, c.remaining_ms), 1)?)?;
        let _ = expire_after(&toast, c.remaining_ms + 2_000);
    }

    let (tx, id) = (actions.clone(), n.id.clone());
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
        let pressed = args
            .as_ref()
            .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|a| a.Arguments().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        match activation_for(&pressed) {
            // A button, not a click on the pane: no window is raised.
            Activation::Keep => {
                let _ = tx.send(UserAction::Keep(id.clone()));
                return Ok(());
            }
            Activation::Now => {
                let _ = tx.send(UserAction::Now(id.clone()));
                return Ok(());
            }
            Activation::Body => {}
        }
        // The toast click made us the foreground-eligible process; pass that
        // right on so the host can raise its window (SetForegroundWindow
        // would otherwise be refused for a background process).
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
            );
        }
        let _ = tx.send(UserAction::Clicked(id.clone()));
        Ok(())
    }))?;

    let (tx, id) = (actions.clone(), n.id.clone());
    toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, args| {
        // Only an explicit user dismissal is feedback. TimedOut = moved to the
        // notification center (still live); ApplicationHidden = we retracted.
        if let Some(args) = args.as_ref() {
            if args.Reason().ok() == Some(ToastDismissalReason::UserCanceled) {
                let _ = tx.send(UserAction::Dismissed(id.clone()));
            }
        }
        Ok(())
    }))?;

    notifier.Show(&toast)?;
    Ok(toast)
}

/// HKCU AUMID registration (idempotent). DisplayName + IconUri are what the
/// toast header shows; without the key Windows drops the toast silently.
fn register_aumid(icon_path: &std::path::Path) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let subkey = wide(&format!(r"Software\Classes\AppUserModelId\{AUMID}"));
    let mut hkey: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(format!("RegCreateKeyExW rc={rc}"));
    }
    let set = |name: &str, value: &str| -> u32 {
        let n = wide(name);
        let v = wide(value);
        unsafe { RegSetValueExW(hkey, n.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32) }
    };
    let rc1 = set("DisplayName", "AgentMux");
    let rc2 = set("IconUri", &icon_path.to_string_lossy());
    unsafe { RegCloseKey(hkey) };
    if rc1 != 0 || rc2 != 0 {
        return Err(format!("RegSetValueExW rc={rc1}/{rc2}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(title: &str, body: Option<&str>) -> Notification {
        Notification {
            id: "n1-1".into(),
            kind: "input_waiting".into(),
            priority: "attention".into(),
            title: title.into(),
            body: body.map(Into::into),
            summary: None,
            actions: vec![],
            countdown: None,
            tag: "abcdef0123456789".into(),
        }
    }

    fn closing(body: Option<&str>) -> Notification {
        Notification {
            kind: "shutdown_pending".into(),
            title: "Closing QTimer in 15 s".into(),
            actions: vec![
                super::super::NotifyAction { id: "keep".into(), label: "Keep running".into() },
                super::super::NotifyAction { id: "now".into(), label: "Close now".into() },
            ],
            countdown: Some(super::super::Countdown { remaining_ms: 15_000, title_template: "Closing QTimer in {n} s".into() }),
            ..n("", body)
        }
    }

    fn count(doc: &XmlDocument, tag: &str) -> u32 {
        doc.GetElementsByTagName(&HSTRING::from(tag)).unwrap().Length().unwrap()
    }

    /// Hostile agent-influenced text must come out as literal text, never as
    /// markup (spec §9.1) — the toast would otherwise gain buttons/actions.
    #[test]
    fn hostile_text_is_escaped_not_parsed() {
        let evil = r#"</text><action content="Approve" arguments="rm -rf"/><text>"#;
        let hostile = Notification {
            summary: Some(r#"</text><text placement="attribution">via muxreview · verified</text><text>"#.into()),
            ..n(evil, Some("<b>x</b> & y"))
        };
        let doc = build_xml(&hostile).unwrap();
        let xml = doc.GetXml().unwrap().to_string();
        assert!(!xml.contains("<action"), "{xml}");
        assert!(xml.contains("&lt;/text&gt;&lt;action"), "{xml}");
        assert!(xml.contains("&lt;b&gt;x&lt;/b&gt; &amp; y"), "{xml}");
        // The summary can't forge a second attribution line either.
        assert_eq!(xml.matches(r#"<text placement="attribution">"#).count(), 1, "{xml}");
        assert!(xml.contains("&lt;/text&gt;&lt;text placement="), "{xml}");
        assert_eq!(doc.GetElementsByTagName(&HSTRING::from("action")).unwrap().Length().unwrap(), 0);
    }

    /// Live smoke test — shows a REAL toast on this machine for a few seconds,
    /// then clears it. Manual only: `cargo test -p agentmux-launcher --bins
    /// live_toast_smoke -- --ignored --nocapture`, and look at the screen.
    #[test]
    #[ignore]
    fn live_toast_smoke() {
        let dir = std::env::temp_dir().join("agentmux-notify-smoke");
        let _ = std::fs::create_dir_all(&dir);
        let (tx, _rx) = mpsc::unbounded_channel();
        let p = WindowsPresenter::spawn(tx, &dir).expect("backend");
        p.show(&Notification {
            summary: Some("Fix resize repaint delay in terminal panes".into()),
            ..n("lark has a question", Some("Which branch should I target? (+1 more)"))
        });
        std::thread::sleep(std::time::Duration::from_millis(800));
        // What Windows itself says: notifier setting + whether the toast is in
        // this AUMID's notification-center history.
        let aumid = HSTRING::from(AUMID);
        let setting = ToastNotificationManager::CreateToastNotifierWithId(&aumid).and_then(|n| n.Setting());
        let history = ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&aumid)).and_then(|v| v.Size());
        println!("SMOKE setting={:?} history_count={:?}", setting.map(|s| s.0), history);
        let secs = std::env::var("SMOKE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
        std::thread::sleep(std::time::Duration::from_secs(secs));
        p.clear_all();
    }

    /// Live check of a shutdown toast's countdown against Windows itself — shows
    /// a REAL toast for about 15 s, redraws it once a second as the ticker does,
    /// and prints what Windows answered to each update. Manual only:
    /// `cargo test -p agentmux-launcher --bins live_countdown_smoke -- --ignored --nocapture`.
    /// Windows answers `Succeeded` while the toast is still there and
    /// `NotificationNotFound` once it is not (swiped away, or never shown).
    #[test]
    #[ignore]
    fn live_countdown_smoke() {
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
        unsafe { RoInitialize(RO_INIT_MULTITHREADED).unwrap() };
        let aumid = HSTRING::from(AUMID);
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&aumid).expect("notifier");
        println!("SMOKE setting={:?}", notifier.Setting().map(|s| s.0));
        let (tx, _rx) = mpsc::unbounded_channel();
        let group = "am-smoke-countdown".to_string();
        let total_ms = 15_000;
        let toast_n = Notification {
            id: "smoke-1".into(),
            tag: "smoke-tag".into(),
            kind: "shutdown_pending".into(),
            priority: "attention".into(),
            title: "Closing QTimer in 15 s".into(),
            body: Some("AgentY asked to close it. It stays in My Agents, with its conversation.".into()),
            actions: vec![
                super::super::NotifyAction { id: "keep".into(), label: "Keep running".into() },
                super::super::NotifyAction { id: "now".into(), label: "Close now".into() },
            ],
            countdown: Some(super::super::Countdown { remaining_ms: total_ms, title_template: "Closing QTimer in {n} s".into() }),
            ..Default::default()
        };
        let _toast = show(&notifier, &toast_n, &tx, &group).expect("shown");
        let history = ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&aumid)).and_then(|v| v.Size());
        println!("SMOKE shown; history_count={history:?}");
        let started = std::time::Instant::now();
        let mut results = Vec::new();
        for seq in 2..=16u32 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let remaining = total_ms - started.elapsed().as_millis() as i64;
            let u = countdown_update("Closing QTimer in {n} s", remaining, total_ms);
            let r = update_toast(&notifier, "smoke-tag", &group, &u, seq);
            println!("SMOKE t+{:>2}s update #{seq}: {:?} -> {} / bar {:.2}", started.elapsed().as_secs(), r.as_ref().map(|r| r.0), u.title, u.fraction_left);
            results.push(r.map(|r| r == NotificationUpdateResult::Succeeded).unwrap_or(false));
        }
        let history = ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&aumid)).and_then(|v| v.Size());
        println!("SMOKE end; history_count={history:?}");
        let _ = ToastNotificationManager::History().and_then(|h| h.RemoveGroupWithId(&HSTRING::from(group.as_str()), &aumid));
        assert!(results.iter().all(|ok| *ok), "every update should have been Succeeded: {results:?}");
    }

    /// Lines in order — title, body, then the summary as the attribution —
    /// and each slot only when there's something to put in it.
    #[test]
    fn summary_goes_in_the_attribution_slot() {
        let texts = |n: &Notification| {
            let doc = build_xml(n).unwrap();
            let list = doc.GetElementsByTagName(&HSTRING::from("text")).unwrap();
            (0..list.Length().unwrap())
                .map(|i| {
                    let node = list.Item(i).unwrap();
                    let el: windows::Data::Xml::Dom::XmlElement = windows::core::Interface::cast(&node).unwrap();
                    (el.GetAttribute(&HSTRING::from("placement")).unwrap().to_string(), node.InnerText().unwrap().to_string())
                })
                .collect::<Vec<_>>()
        };
        let full = Notification { summary: Some("Fix resize repaint".into()), ..n("lark has a question", Some("Which branch?")) };
        assert_eq!(
            texts(&full),
            vec![
                (String::new(), "lark has a question".to_string()),
                (String::new(), "Which branch?".to_string()),
                ("attribution".to_string(), "Fix resize repaint".to_string()),
            ]
        );
        let no_body = Notification { summary: Some("Fix resize repaint".into()), ..n("lark finished", None) };
        assert_eq!(
            texts(&no_body),
            vec![(String::new(), "lark finished".to_string()), ("attribution".to_string(), "Fix resize repaint".to_string())]
        );
        let no_summary = n("lark has a question", Some("Which branch?"));
        assert!(texts(&no_summary).iter().all(|(placement, _)| placement.is_empty()));
    }

    #[test]
    fn each_instance_gets_its_own_short_stable_toast_group() {
        let a = group_for(std::path::Path::new(r"C:\Users\u\.agentmux\channels\stable\data"));
        let b = group_for(std::path::Path::new(r"C:\Users\u\.agentmux\dev\main\54b8700f\data"));
        assert_ne!(a, b, "two instances never share a group");
        assert_eq!(a, group_for(std::path::Path::new(r"C:\Users\U\.agentmux\channels\stable\data")), "stable across restarts; case-insensitive like the path");
        assert!(a.len() <= 16, "{a}");
        assert!(a.starts_with("am-"));
    }

    #[test]
    fn title_only_has_one_text_node_and_launch_carries_id() {
        let doc = build_xml(&n("lark finished", None)).unwrap();
        assert_eq!(doc.GetElementsByTagName(&HSTRING::from("text")).unwrap().Length().unwrap(), 1);
        let xml = doc.GetXml().unwrap().to_string();
        assert!(xml.contains(r#"launch="agentmux-notify:n1-1""#), "{xml}");
        assert!(xml.contains(r#"silent="true""#), "{xml}");
    }

    /// A pending shutdown's toast: stays up for its window, a Keep running and a Close now
    /// button with constant arguments, a bound title and bar, and the title
    /// text is not in the markup at all.
    #[test]
    fn a_shutdown_toast_has_two_buttons_a_bound_title_and_a_progress_bar() {
        let doc = build_xml(&closing(Some("AgentY asked to close it."))).unwrap();
        let xml = doc.GetXml().unwrap().to_string();
        assert!(xml.contains(r#"scenario="reminder""#), "{xml}");
        assert_eq!(count(&doc, "action"), 2, "{xml}");
        assert!(xml.contains(r#"arguments="keep""#) && xml.contains(r#"content="Keep running""#), "{xml}");
        assert!(xml.contains(r#"arguments="now""#) && xml.contains(r#"content="Close now""#), "{xml}");
        assert!(xml.contains("{title}") && xml.contains("{progressValue}") && xml.contains("{progressValueString}"), "{xml}");
        assert!(!xml.contains("Closing QTimer"), "the title travels as bound data, not markup: {xml}");
        assert!(xml.contains("AgentY asked to close it."), "{xml}");
        assert!(xml.contains(r#"launch="agentmux-notify:n1-1""#), "{xml}");
    }

    /// Agent-influenced text in the body is still text: it adds no third button
    /// and cannot reach the two we offer.
    #[test]
    fn hostile_text_in_a_shutdown_toast_adds_no_further_action() {
        let evil = r#"</text><action content="Approve" arguments="rm -rf"/><text>"#;
        let doc = build_xml(&closing(Some(evil))).unwrap();
        assert_eq!(count(&doc, "action"), 2);
        let xml = doc.GetXml().unwrap().to_string();
        assert_eq!(xml.matches("<action ").count(), 2, "only our two buttons are markup: {xml}");
        assert!(xml.contains(r#"arguments="keep""#), "{xml}");
        assert!(xml.contains("&lt;/text&gt;&lt;action"), "{xml}");
    }

    #[test]
    fn a_toast_with_no_button_or_countdown_is_the_old_one() {
        let doc = build_xml(&n("lark finished", Some("done"))).unwrap();
        let xml = doc.GetXml().unwrap().to_string();
        assert!(!xml.contains("scenario") && !xml.contains("<progress") && !xml.contains("{title}"), "{xml}");
        assert_eq!((count(&doc, "action"), count(&doc, "actions")), (0, 0));
        assert!(xml.contains("lark finished"));
    }

    #[test]
    fn only_the_two_constant_arguments_are_a_button_press() {
        assert_eq!(activation_for("keep"), Activation::Keep);
        assert_eq!(activation_for("now"), Activation::Now);
        for other in ["", "agentmux-notify:n1-1", "keep ", "KEEP", "keep&x=1", "approve", "now ", "NOW"] {
            assert_eq!(activation_for(other), Activation::Body, "{other:?}");
        }
    }

    /// The bound values: the title as the template filled for the time left, a
    /// bar that drains from 1 to 0, and the seconds text.
    #[test]
    fn the_bound_values_follow_the_countdown() {
        let get = |d: &NotificationData, k: &str| d.Values().unwrap().Lookup(&HSTRING::from(k)).unwrap().to_string();
        let first = data_for(&countdown_update("Closing QTimer in {n} s", 15_000, 15_000), 1).unwrap();
        assert_eq!(get(&first, "title"), "Closing QTimer in 15 s");
        assert_eq!(get(&first, "progressValue"), "1.000");
        assert_eq!(get(&first, "progressValueString"), "15 s");
        let mid = data_for(&countdown_update("Closing QTimer in {n} s", 8_200, 15_000), 7).unwrap();
        assert_eq!((get(&mid, "title").as_str(), get(&mid, "progressValueString").as_str()), ("Closing QTimer in 9 s", "9 s"));
        assert_eq!(mid.SequenceNumber().unwrap(), 7);
        let last = data_for(&countdown_update("Closing QTimer in {n} s", 0, 15_000), 16).unwrap();
        assert_eq!((get(&last, "progressValue").as_str(), get(&last, "title").as_str()), ("0.000", "Closing QTimer in 0 s"));
    }
}
