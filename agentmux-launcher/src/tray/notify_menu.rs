// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The tray as a notification hub — `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §4.2.
//!
//! Platform-neutral half: the state the srv Router publishes
//! (`notification:state`), the menu model built from it, pause resolution,
//! and the tooltip line. Backends render it; `notify::tray_request` carries
//! the user's picks back to srv.
//!
//! Kept separate from `TrayAction` / `menu_model` on purpose: those are
//! `Copy` and shared with the macOS backend, which this phase does not touch
//! (Phase 3 brings the same hub to the menu bar).

use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

/// Mirrors srv's `NotifyTrayState` (`backend/notify/policy.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct NotifyTrayState {
    #[serde(default)]
    pub attention: Vec<AttentionItem>,
    #[serde(default)]
    pub paused_until_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AttentionItem {
    pub id: String,
    pub block_id: String,
    /// App-controlled template ("lark needs your input") — safe for a menu.
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseChoice {
    Minutes(i64),
    /// Until 08:00 local time tomorrow.
    UntilTomorrow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyMenuAction {
    /// Same as clicking that notification's toast.
    Open(String),
    Pause(PauseChoice),
    Resume,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyMenuEntry {
    Item { label: String, action: NotifyMenuAction },
    Submenu { label: String, items: Vec<NotifyMenuEntry> },
    Separator,
}

static STATE: Mutex<Option<NotifyTrayState>> = Mutex::new(None);
type Wake = Box<dyn Fn() + Send + Sync>;
static WAKE: OnceLock<Wake> = OnceLock::new();

/// Called by the notify presenter when srv publishes a new snapshot.
pub fn set(state: NotifyTrayState) {
    {
        let mut cur = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if cur.as_ref() == Some(&state) {
            return;
        }
        *cur = Some(state);
    }
    if let Some(w) = WAKE.get() {
        w();
    }
}

pub fn get() -> NotifyTrayState {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// A backend registers how to wake its UI thread when the state changes.
pub fn set_wake(f: Wake) {
    let _ = WAKE.set(f);
}

/// muda treats `&` as a mnemonic marker on Windows; agent names are
/// user-chosen, so escape it rather than let "R&D" render as "RD".
pub fn menu_label(s: &str) -> String {
    let mut out: String = s.chars().take(60).collect();
    if s.chars().count() > 60 {
        out.push('…');
    }
    out.replace('&', "&&")
}

fn format_local_hm(epoch_ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(epoch_ms)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// The notification section of the tray menu (placed above New Window/Quit).
pub fn menu(state: &NotifyTrayState, now_ms: i64) -> Vec<NotifyMenuEntry> {
    let mut out = Vec::new();
    if !state.attention.is_empty() {
        let n = state.attention.len();
        out.push(NotifyMenuEntry::Submenu {
            label: if n == 1 { "1 agent needs you".to_string() } else { format!("{n} agents need you") },
            items: state
                .attention
                .iter()
                .map(|a| NotifyMenuEntry::Item { label: menu_label(&a.title), action: NotifyMenuAction::Open(a.id.clone()) })
                .collect(),
        });
    }
    if state.paused_until_ms > now_ms {
        out.push(NotifyMenuEntry::Item {
            label: format!("Resume notifications (paused until {})", format_local_hm(state.paused_until_ms)),
            action: NotifyMenuAction::Resume,
        });
    } else {
        out.push(NotifyMenuEntry::Submenu {
            label: "Pause notifications".to_string(),
            items: vec![
                NotifyMenuEntry::Item { label: "For 30 minutes".into(), action: NotifyMenuAction::Pause(PauseChoice::Minutes(30)) },
                NotifyMenuEntry::Item { label: "For 1 hour".into(), action: NotifyMenuAction::Pause(PauseChoice::Minutes(60)) },
                NotifyMenuEntry::Item { label: "For 4 hours".into(), action: NotifyMenuAction::Pause(PauseChoice::Minutes(240)) },
                NotifyMenuEntry::Item { label: "Until tomorrow".into(), action: NotifyMenuAction::Pause(PauseChoice::UntilTomorrow) },
            ],
        });
    }
    out.push(NotifyMenuEntry::Separator);
    out
}

/// Absolute epoch-ms deadline for a pause choice, resolved at click time.
pub fn resolve_pause(choice: PauseChoice, now: chrono::DateTime<chrono::Local>) -> i64 {
    match choice {
        PauseChoice::Minutes(m) => now.timestamp_millis() + m * 60_000,
        PauseChoice::UntilTomorrow => {
            use chrono::TimeZone;
            let tomorrow = now.date_naive() + chrono::Days::new(1);
            let at = tomorrow.and_hms_opt(8, 0, 0).expect("valid time");
            chrono::Local
                .from_local_datetime(&at)
                .earliest()
                .map(|t| t.timestamp_millis())
                .unwrap_or(now.timestamp_millis() + 12 * 3_600_000)
        }
    }
}

/// Extra tooltip line, if any.
pub fn tooltip_suffix(state: &NotifyTrayState, now_ms: i64) -> Option<String> {
    let n = state.attention.len();
    let mut parts = Vec::new();
    if n == 1 {
        parts.push("1 agent needs you".to_string());
    } else if n > 1 {
        parts.push(format!("{n} agents need you"));
    }
    if state.paused_until_ms > now_ms {
        parts.push("notifications paused".to_string());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str) -> AttentionItem {
        AttentionItem { id: id.into(), block_id: format!("b-{id}"), title: title.into() }
    }

    #[test]
    fn idle_menu_offers_pause_only() {
        let m = menu(&NotifyTrayState::default(), 0);
        assert_eq!(m.len(), 2);
        match &m[0] {
            NotifyMenuEntry::Submenu { label, items } => {
                assert_eq!(label, "Pause notifications");
                assert_eq!(items.len(), 4);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m[1], NotifyMenuEntry::Separator);
    }

    #[test]
    fn attention_submenu_lists_items_and_opens_by_id() {
        let s = NotifyTrayState { attention: vec![item("n1", "lark needs your input"), item("n2", "R&D stopped unexpectedly")], paused_until_ms: 0 };
        let m = menu(&s, 0);
        match &m[0] {
            NotifyMenuEntry::Submenu { label, items } => {
                assert_eq!(label, "2 agents need you");
                assert_eq!(items[0], NotifyMenuEntry::Item { label: "lark needs your input".into(), action: NotifyMenuAction::Open("n1".into()) });
                match &items[1] {
                    NotifyMenuEntry::Item { label, .. } => assert_eq!(label, "R&&D stopped unexpectedly"),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn paused_menu_offers_resume() {
        let s = NotifyTrayState { attention: vec![], paused_until_ms: 10_000 };
        let m = menu(&s, 5_000);
        match &m[0] {
            NotifyMenuEntry::Item { label, action } => {
                assert!(label.starts_with("Resume notifications (paused until "), "{label}");
                assert_eq!(*action, NotifyMenuAction::Resume);
            }
            other => panic!("{other:?}"),
        }
        // Expired pause → back to the Pause submenu.
        assert!(matches!(&menu(&s, 20_000)[0], NotifyMenuEntry::Submenu { .. }));
    }

    #[test]
    fn pause_resolution() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 9, 24, 22, 15, 0).unwrap();
        assert_eq!(resolve_pause(PauseChoice::Minutes(30), now), now.timestamp_millis() + 1_800_000);
        let t = resolve_pause(PauseChoice::UntilTomorrow, now);
        let expect = chrono::Local.with_ymd_and_hms(2026, 9, 25, 8, 0, 0).unwrap().timestamp_millis();
        assert_eq!(t, expect);
    }

    #[test]
    fn tooltip_line() {
        assert_eq!(tooltip_suffix(&NotifyTrayState::default(), 0), None);
        let s = NotifyTrayState { attention: vec![item("n1", "x")], paused_until_ms: 100 };
        assert_eq!(tooltip_suffix(&s, 0).as_deref(), Some("1 agent needs you · notifications paused"));
    }

    #[test]
    fn long_titles_are_truncated() {
        let l = menu_label(&"x".repeat(100));
        assert_eq!(l.chars().count(), 61);
    }

    #[test]
    fn parses_srv_payload() {
        let s: NotifyTrayState = serde_json::from_str(r#"{"attention":[{"id":"n1","block_id":"b1","title":"lark needs your input"}],"paused_until_ms":0}"#).unwrap();
        assert_eq!(s.attention[0].title, "lark needs your input");
    }
}
