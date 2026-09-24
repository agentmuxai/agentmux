// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux notification backend — `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §6.3.
//!
//! freedesktop Desktop Notifications (`org.freedesktop.Notifications`) spoken
//! directly over async zbus on the launcher's Tokio runtime — no libnotify,
//! no GTK, no blocking D-Bus calls. One task owns the session-bus connection:
//!
//! - `Notify` with `replaces_id` = the server id last used for this tag, so a
//!   same-group update replaces in place (spec §5.3);
//! - `CloseNotification` for retract / clear-all;
//! - `ActionInvoked("default")` → click, `NotificationClosed(reason 2)` →
//!   user dismissal (1 = expired, 3 = closed by us — neither is feedback).
//!
//! Clicks only reach a process still on the bus — fine, the launcher lives for
//! the whole session (and clears its notifications on exit).
//!
//! Content safety (§9.1): if the server advertises `body-markup`, the body is
//! escaped so agent-influenced text can't inject markup. The summary is never
//! markup per the spec.

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::zvariant::Value;

use super::{Notification, Presenter, UserAction};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

enum Cmd {
    Show(Notification),
    Retract(String),
    ClearAll(std_mpsc::Sender<()>),
}

pub struct LinuxPresenter {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl LinuxPresenter {
    pub fn spawn(actions: mpsc::UnboundedSender<UserAction>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(rx, actions));
        Self { tx }
    }
}

impl Presenter for LinuxPresenter {
    fn show(&self, n: &Notification) {
        let _ = self.tx.send(Cmd::Show(n.clone()));
    }
    fn retract(&self, tag: &str) {
        let _ = self.tx.send(Cmd::Retract(tag.to_string()));
    }
    fn clear_all(&self) {
        let (done_tx, done_rx) = std_mpsc::channel();
        if self.tx.send(Cmd::ClearAll(done_tx)).is_ok() {
            let _ = done_rx.recv_timeout(std::time::Duration::from_secs(2));
        }
    }
}

/// Escape for servers that parse the body as (a subset of) markup.
pub fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `(summary, body)` exactly as sent — pure, so the escaping rule is testable.
pub fn texts(n: &Notification, body_markup: bool) -> (String, String) {
    let body = n.body.clone().unwrap_or_default();
    (n.title.clone(), if body_markup { escape_markup(&body) } else { body })
}

/// `(urgency, expire_timeout, resident)` for a notification.
///
/// Attention (needs input / crashed / needs review) must not silently expire
/// before the user sees it (ReAgent P1 on #3668) — so it never expires
/// (`expire_timeout = 0`) and is `resident` (stays in the list after an
/// action). It deliberately stays at urgency **1**, not 2: "critical" breaks
/// through Do Not Disturb on GNOME, which the spec forbids (§5.1, §6.0) —
/// Windows' High priority doesn't bypass Focus either.
pub fn delivery(n: &Notification) -> (u8, i32, bool) {
    if n.is_attention() {
        (1, 0, true)
    } else {
        (1, -1, false)
    }
}

/// Map a `NotificationClosed` reason to user feedback: only 2 ("dismissed by
/// the user") counts.
pub fn closed_is_user_dismissal(reason: u32) -> bool {
    reason == 2
}

async fn run(mut rx: mpsc::UnboundedReceiver<Cmd>, actions: mpsc::UnboundedSender<UserAction>) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            crate::logging::log(&format!("notify: no D-Bus session bus ({e}); notifications disabled"));
            // Keep draining so callers (clear_all's bounded wait) never stall.
            while let Some(cmd) = rx.recv().await {
                if let Cmd::ClearAll(done) = cmd {
                    let _ = done.send(());
                }
            }
            return;
        }
    };

    let body_markup = match conn.call_method(Some(DEST), PATH, Some(IFACE), "GetCapabilities", &()).await {
        Ok(reply) => reply.body().deserialize::<Vec<String>>().map(|c| c.iter().any(|x| x == "body-markup")).unwrap_or(false),
        Err(e) => {
            crate::logging::log(&format!("notify: no notification server ({e})"));
            false
        }
    };

    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(IFACE)
        .map(|b| b.build());
    let mut signals = match rule {
        Ok(rule) => zbus::MessageStream::for_match_rule(rule, &conn, None).await.ok(),
        Err(_) => None,
    };

    // tag → (server id, our notification id); server id → our id.
    let mut by_tag: HashMap<String, (u32, String)> = HashMap::new();
    let mut by_server: HashMap<u32, String> = HashMap::new();

    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    Cmd::Show(n) => {
                        let replaces = by_tag.get(&n.tag).map(|(sid, _)| *sid).unwrap_or(0);
                        let (summary, body) = texts(&n, body_markup);
                        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
                        hints.insert("desktop-entry", Value::from("agentmux"));
                        let (urgency, timeout, resident) = delivery(&n);
                        hints.insert("urgency", Value::from(urgency));
                        if resident {
                            hints.insert("resident", Value::from(true));
                        }
                        let args = (
                            "AgentMux",
                            replaces,
                            "agentmux",
                            summary.as_str(),
                            body.as_str(),
                            vec!["default", "Open"],
                            hints,
                            timeout,
                        );
                        match conn.call_method(Some(DEST), PATH, Some(IFACE), "Notify", &args).await {
                            Ok(reply) => {
                                if let Ok(sid) = reply.body().deserialize::<u32>() {
                                    if let Some((old, _)) = by_tag.insert(n.tag.clone(), (sid, n.id.clone())) {
                                        by_server.remove(&old);
                                    }
                                    by_server.insert(sid, n.id.clone());
                                }
                            }
                            Err(e) => crate::logging::log(&format!("notify: Notify failed: {e}")),
                        }
                    }
                    Cmd::Retract(tag) => {
                        if let Some((sid, _)) = by_tag.remove(&tag) {
                            by_server.remove(&sid);
                            let _ = conn.call_method(Some(DEST), PATH, Some(IFACE), "CloseNotification", &(sid,)).await;
                        }
                    }
                    Cmd::ClearAll(done) => {
                        for (_, (sid, _)) in by_tag.drain() {
                            let _ = conn.call_method(Some(DEST), PATH, Some(IFACE), "CloseNotification", &(sid,)).await;
                        }
                        by_server.clear();
                        let _ = done.send(());
                    }
                }
            }
            msg = async {
                match signals.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(Ok(msg)) = msg else { continue };
                let header = msg.header();
                let member = header.member().map(|m| m.as_str().to_string()).unwrap_or_default();
                match member.as_str() {
                    "ActionInvoked" => {
                        if let Ok((sid, action)) = msg.body().deserialize::<(u32, String)>() {
                            if action == "default" {
                                if let Some(id) = by_server.get(&sid) {
                                    let _ = actions.send(UserAction::Clicked(id.clone()));
                                }
                            }
                        }
                    }
                    "NotificationClosed" => {
                        if let Ok((sid, reason)) = msg.body().deserialize::<(u32, u32)>() {
                            if let Some(id) = by_server.remove(&sid) {
                                by_tag.retain(|_, (s, _)| *s != sid);
                                if closed_is_user_dismissal(reason) {
                                    let _ = actions.send(UserAction::Dismissed(id));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(body: Option<&str>) -> Notification {
        Notification {
            id: "n1".into(),
            kind: "input_waiting".into(),
            priority: "attention".into(),
            title: "lark needs your input".into(),
            body: body.map(Into::into),
            tag: "t1".into(),
        }
    }

    #[test]
    fn body_is_escaped_only_for_markup_servers() {
        let evil = "<a href='x'>click</a> & more";
        assert_eq!(texts(&n(Some(evil)), true).1, "&lt;a href='x'&gt;click&lt;/a&gt; &amp; more");
        assert_eq!(texts(&n(Some(evil)), false).1, evil);
        assert_eq!(texts(&n(None), true), ("lark needs your input".to_string(), String::new()));
    }

    #[test]
    fn attention_never_expires_but_never_goes_critical() {
        let mut a = n(None);
        a.priority = "attention".into();
        assert_eq!(delivery(&a), (1, 0, true));
        let mut normal = n(None);
        normal.priority = "normal".into();
        assert_eq!(delivery(&normal), (1, -1, false));
    }

    #[test]
    fn only_user_dismissal_is_feedback() {
        assert!(closed_is_user_dismissal(2));
        assert!(!closed_is_user_dismissal(1)); // expired
        assert!(!closed_is_user_dismissal(3)); // closed by us
        assert!(!closed_is_user_dismissal(4));
    }
}
