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

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;

use tokio::sync::mpsc;
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    ToastDismissalReason, ToastDismissedEventArgs, ToastNotification, ToastNotificationManager,
    ToastNotificationPriority, ToastNotifier,
};

use super::{Notification, Presenter, UserAction};

/// Must match the host's `SetCurrentProcessExplicitAppUserModelID`
/// (`agentmux-cef/src/lib.rs`) so toasts and taskbar grouping agree.
pub const AUMID: &str = "AgentMuxCorp.AgentMux";
const GROUP: &str = "agentmux";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/favicon-150x150.png");
/// Upper bound on waiting for the WinRT thread at startup (see `spawn`).
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

enum Cmd {
    Show(Notification),
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
        std::thread::Builder::new()
            .name("agentmux-notify".into())
            .spawn(move || thread_main(rx, actions, ready_tx))
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
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Show(n) => match show(&notifier, &n, &actions) {
                Ok(toast) => {
                    live.insert(n.tag.clone(), toast);
                }
                Err(e) => crate::logging::log(&format!("notify: show failed: {e}")),
            },
            Cmd::Retract(tag) => {
                live.remove(&tag);
                if let Ok(h) = ToastNotificationManager::History() {
                    let _ = h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(GROUP), &aumid);
                }
            }
            Cmd::ClearAll(done) => {
                live.clear();
                if let Ok(h) = ToastNotificationManager::History() {
                    let _ = h.ClearWithId(&aumid);
                }
                let _ = done.send(());
            }
        }
    }
}

/// Build the toast document: constant skeleton, text via text nodes.
pub fn build_xml(n: &Notification) -> windows::core::Result<XmlDocument> {
    let doc = XmlDocument::new()?;
    // Title, then the body, then the summary in the `attribution` slot, which
    // Windows renders small and grey under the body without using up one of
    // ToastGeneric's three text lines (rich-content spec §1.3). The skeleton
    // is fixed markup; every line is filled as a text node below.
    let mut skeleton = String::from(r#"<toast><visual><binding template="ToastGeneric"><text/>"#);
    if n.body.is_some() {
        skeleton.push_str("<text/>");
    }
    if n.summary.is_some() {
        skeleton.push_str(r#"<text placement="attribution"/>"#);
    }
    skeleton.push_str(r#"</binding></visual><audio silent="true"/></toast>"#);
    doc.LoadXml(&HSTRING::from(skeleton))?;
    let root = doc.DocumentElement()?;
    root.SetAttribute(&HSTRING::from("launch"), &HSTRING::from(format!("agentmux-notify:{}", n.id)))?;
    let texts = doc.GetElementsByTagName(&HSTRING::from("text"))?;
    let lines = std::iter::once(n.title.as_str()).chain(n.body.as_deref()).chain(n.summary.as_deref());
    for (i, line) in lines.enumerate() {
        let node = texts.Item(i as u32)?;
        node.AppendChild(&doc.CreateTextNode(&HSTRING::from(line))?)?;
    }
    Ok(doc)
}

fn show(
    notifier: &ToastNotifier,
    n: &Notification,
    actions: &mpsc::UnboundedSender<UserAction>,
) -> windows::core::Result<ToastNotification> {
    let toast = ToastNotification::CreateToastNotification(&build_xml(n)?)?;
    toast.SetTag(&HSTRING::from(n.tag.as_str()))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    if n.is_attention() {
        let _ = toast.SetPriority(ToastNotificationPriority::High);
    }

    let (tx, id) = (actions.clone(), n.id.clone());
    toast.Activated(&TypedEventHandler::new(move |_, _| {
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
            tag: "abcdef0123456789".into(),
        }
    }

    /// Hostile agent-influenced text must come out as literal text, never as
    /// markup (spec §9.1) — the toast would otherwise gain buttons/actions.
    #[test]
    fn hostile_text_is_escaped_not_parsed() {
        let evil = r#"</text><action content="Approve" arguments="rm -rf"/><text>"#;
        let hostile = Notification {
            summary: Some(r#"</text><text placement="attribution">via ReAgent · verified</text><text>"#.into()),
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
    fn title_only_has_one_text_node_and_launch_carries_id() {
        let doc = build_xml(&n("lark finished", None)).unwrap();
        assert_eq!(doc.GetElementsByTagName(&HSTRING::from("text")).unwrap().Length().unwrap(), 1);
        let xml = doc.GetXml().unwrap().to_string();
        assert!(xml.contains(r#"launch="agentmux-notify:n1-1""#), "{xml}");
        assert!(xml.contains(r#"silent="true""#), "{xml}");
    }
}
