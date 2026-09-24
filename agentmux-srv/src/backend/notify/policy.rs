// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure notification policy — `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §5.
//!
//! `PolicyState::step(input, now_ms, &settings) -> Vec<Action>` is the whole
//! decision surface: no clocks, no I/O, no broker. `router.rs` owns the side
//! effects (publishing, the tick task). Keeping it pure is what makes the
//! debounce / dedupe / focus-gate / retract rules table-testable.
//!
//! Phase 1 scope: kinds `InputWaiting` / `TurnCompleted` / `TurnErrored` /
//! `Test`, per-kind enable, debounce, dedupe-by-group, focus gate, retract.
//! Rate limiting + summaries are Phase 2 (§10).

use std::collections::HashMap;

/// Closed set of notification kinds. A source cannot invent a new visual
/// treatment — every kind has an app-controlled title template (§9.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub enum NotifyKind {
    InputWaiting,
    TurnCompleted,
    TurnErrored,
    Test,
}

impl NotifyKind {
    /// `notify:os:<suffix>` per-kind enable key (§8).
    pub fn setting_suffix(self) -> Option<&'static str> {
        match self {
            NotifyKind::InputWaiting => Some("inputwaiting"),
            NotifyKind::TurnCompleted => Some("turncompleted"),
            NotifyKind::TurnErrored => Some("turnerrored"),
            NotifyKind::Test => None,
        }
    }

    /// Debounce before a notification may be shown (§5.1 step 5). Lets the
    /// user's own imminent action (answering, starting the next turn) cancel
    /// it before it ever interrupts them.
    pub fn delay_ms(self) -> i64 {
        match self {
            NotifyKind::InputWaiting => 6_000,
            NotifyKind::TurnCompleted | NotifyKind::TurnErrored => 10_000,
            NotifyKind::Test => 0,
        }
    }

    pub fn priority(self) -> Priority {
        match self {
            NotifyKind::InputWaiting => Priority::Attention,
            _ => Priority::Normal,
        }
    }

    /// Coalescing key: at most one live notification per (block, family).
    pub fn group_key(self, block_id: &str) -> String {
        match self {
            NotifyKind::InputWaiting => format!("input:{block_id}"),
            NotifyKind::TurnCompleted | NotifyKind::TurnErrored => format!("turn:{block_id}"),
            NotifyKind::Test => "test".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NotifyPriority")]
pub enum Priority {
    Attention,
    Normal,
}

/// `notify:os:when` (§6.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Unfocused,
    Always,
    Never,
}

/// `notify:os:preview` (§9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preview {
    Full,
    Redacted,
    None,
}

/// Resolved settings snapshot the policy reads. Built by the router from
/// `settings.json` on every step (cheap; the config watcher holds it in memory).
#[derive(Clone, Debug)]
pub struct Settings {
    pub enabled: bool,
    pub when: When,
    pub preview: Preview,
    pub kind_enabled: HashMap<NotifyKind, bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, when: When::Unfocused, preview: Preview::Redacted, kind_enabled: HashMap::new() }
    }
}

impl Settings {
    fn kind_on(&self, kind: NotifyKind) -> bool {
        *self.kind_enabled.get(&kind).unwrap_or(&true)
    }
}

/// What one frontend window last reported about focus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusReport {
    pub window_focused: bool,
    pub block_id: Option<String>,
}

/// A request from a source. `agent_name` / `body` are already resolved and
/// sanitized by the router — the policy never sees untrusted free text it
/// would render as a title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: NotifyKind,
    pub block_id: String,
    pub agent_name: String,
    pub body: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Input {
    /// A source says something notification-worthy happened.
    Emit(Request),
    /// The condition behind a family cleared for a block (question answered,
    /// new turn started) — cancel pending, retract shown.
    Resolve { block_id: String, family: Family },
    /// A frontend window's focus changed.
    Focus { conn_id: String, report: FocusReport },
    /// A frontend window went away.
    Disconnect { conn_id: String },
    /// The user acted on a notification (clicked it / dismissed it).
    Ack { id: String, clicked: bool },
    /// Time passed; release any debounced notification that is now due.
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Input,
    Turn,
}

impl Family {
    fn matches(self, kind: NotifyKind) -> bool {
        match self {
            Family::Input => kind == NotifyKind::InputWaiting,
            Family::Turn => matches!(kind, NotifyKind::TurnCompleted | NotifyKind::TurnErrored),
        }
    }
}

/// The payload Presenters render. Every string here is app-controlled or
/// router-sanitized. Exported to TS as `OsNotification` so it can't shadow the
/// DOM's global `Notification`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "OsNotification")]
pub struct Notification {
    pub id: String,
    pub kind: NotifyKind,
    pub priority: Priority,
    pub block_id: String,
    pub agent_name: String,
    pub title: String,
    pub body: Option<String>,
    /// Stable per group: OS toast replace/remove key (≤16 chars, Win10-safe).
    pub tag: String,
    #[ts(type = "number")]
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Show(Notification),
    Retract { id: String, tag: String },
    Activate { id: String, block_id: String },
}

#[derive(Clone, Debug)]
struct Live {
    notification: Notification,
    due_at_ms: i64,
    shown: bool,
}

#[derive(Default)]
pub struct PolicyState {
    /// group_key → the one live (pending or shown) notification for it.
    live: HashMap<String, Live>,
    focus: HashMap<String, FocusReport>,
    next_seq: u64,
}

/// Title from a closed template set (§9.1). Only the agent name is
/// interpolated, and it arrives pre-sanitized.
pub fn title_for(kind: NotifyKind, agent_name: &str) -> String {
    match kind {
        NotifyKind::InputWaiting => format!("{agent_name} needs your input"),
        NotifyKind::TurnCompleted => format!("{agent_name} finished"),
        NotifyKind::TurnErrored => format!("{agent_name} stopped with an error"),
        NotifyKind::Test => "AgentMux notifications are working".to_string(),
    }
}

/// 16 hex chars of FNV-1a over the group key — stable across restarts,
/// short enough for every Windows toast Tag limit.
pub fn tag_for(group_key: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in group_key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

impl PolicyState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Any window focused at all?
    fn app_focused(&self) -> bool {
        self.focus.values().any(|f| f.window_focused)
    }

    /// Is the user looking at exactly this block right now?
    fn looking_at(&self, block_id: &str) -> bool {
        self.focus
            .values()
            .any(|f| f.window_focused && f.block_id.as_deref() == Some(block_id))
    }

    /// Whether any frontend window is connected (for click-with-no-window).
    pub fn has_window(&self) -> bool {
        !self.focus.is_empty()
    }

    /// §6.0 gate, evaluated at the moment a notification becomes due.
    fn should_show(&self, n: &Notification, s: &Settings) -> bool {
        if n.kind == NotifyKind::Test {
            return true;
        }
        if self.looking_at(&n.block_id) {
            return false;
        }
        match s.when {
            When::Never => false,
            When::Always => true,
            // Unfocused: OS toast only when the app isn't foreground — except
            // Attention, which still reaches a user looking at another pane.
            When::Unfocused => !self.app_focused() || n.priority == Priority::Attention,
        }
    }

    pub fn step(&mut self, input: Input, now_ms: i64, s: &Settings) -> Vec<Action> {
        let mut out = Vec::new();
        match input {
            Input::Emit(req) => {
                let is_test = req.kind == NotifyKind::Test;
                if !is_test && (!s.enabled || !s.kind_on(req.kind)) {
                    return out;
                }
                let group = req.kind.group_key(&req.block_id);
                let tag = tag_for(&group);
                // Replace in place: a newer event in the same family supersedes
                // the older one (e.g. completed → errored), same OS tag.
                let replaced_shown = self.live.get(&group).map(|l| l.shown).unwrap_or(false);
                self.next_seq += 1;
                let id = format!("n{}-{}", now_ms, self.next_seq);
                let notification = Notification {
                    id,
                    kind: req.kind,
                    priority: req.kind.priority(),
                    block_id: req.block_id,
                    title: title_for(req.kind, &req.agent_name),
                    agent_name: req.agent_name,
                    body: match s.preview {
                        Preview::None => None,
                        _ => req.body,
                    },
                    tag,
                    created_at_ms: now_ms,
                };
                // If the old one was on screen and the new one is still
                // debouncing, pull the stale one now rather than leaving it up.
                if replaced_shown && req.kind.delay_ms() > 0 {
                    if let Some(old) = self.live.get(&group) {
                        out.push(Action::Retract { id: old.notification.id.clone(), tag: old.notification.tag.clone() });
                    }
                }
                self.live.insert(
                    group,
                    Live { notification, due_at_ms: now_ms + req.kind.delay_ms(), shown: false },
                );
                if is_test {
                    out.extend(self.release_due(now_ms, s));
                }
            }
            Input::Resolve { block_id, family } => {
                self.drop_where(&mut out, |n| n.block_id == block_id && family.matches(n.kind));
            }
            Input::Focus { conn_id, report } => {
                let looking = report.window_focused.then(|| report.block_id.clone()).flatten();
                self.focus.insert(conn_id, report);
                // The user just looked at the block: its notifications are moot.
                if let Some(b) = looking {
                    self.drop_where(&mut out, |n| n.block_id == b && n.kind != NotifyKind::Test);
                }
            }
            Input::Disconnect { conn_id } => {
                self.focus.remove(&conn_id);
            }
            Input::Ack { id, clicked } => {
                let group = self
                    .live
                    .iter()
                    .find(|(_, l)| l.notification.id == id)
                    .map(|(g, _)| g.clone());
                if let Some(g) = group {
                    let l = self.live.remove(&g).expect("present");
                    out.push(Action::Retract { id: l.notification.id.clone(), tag: l.notification.tag.clone() });
                    if clicked && !l.notification.block_id.is_empty() {
                        out.push(Action::Activate { id: l.notification.id, block_id: l.notification.block_id });
                    }
                }
            }
            Input::Tick => out.extend(self.release_due(now_ms, s)),
        }
        out
    }

    fn release_due(&mut self, now_ms: i64, s: &Settings) -> Vec<Action> {
        let mut out = Vec::new();
        let due: Vec<String> = self
            .live
            .iter()
            .filter(|(_, l)| !l.shown && l.due_at_ms <= now_ms)
            .map(|(g, _)| g.clone())
            .collect();
        for g in due {
            let show = self.should_show(&self.live[&g].notification, s);
            if show {
                let l = self.live.get_mut(&g).expect("present");
                l.shown = true;
                out.push(Action::Show(l.notification.clone()));
            } else {
                // Gated out at due time → gone for good. Not re-offered later:
                // a "finished" toast an hour after the user already saw the
                // pane would be noise.
                self.live.remove(&g);
            }
        }
        out
    }

    fn drop_where(&mut self, out: &mut Vec<Action>, pred: impl Fn(&Notification) -> bool) {
        let groups: Vec<String> =
            self.live.iter().filter(|(_, l)| pred(&l.notification)).map(|(g, _)| g.clone()).collect();
        for g in groups {
            let l = self.live.remove(&g).expect("present");
            if l.shown {
                out.push(Action::Retract { id: l.notification.id, tag: l.notification.tag });
            }
        }
    }

    /// Number of live (pending or shown) notifications — for tests/diagnostics.
    pub fn live_len(&self) -> usize {
        self.live.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(kind: NotifyKind, block: &str) -> Request {
        Request { kind, block_id: block.into(), agent_name: "lark".into(), body: Some("Which branch?".into()) }
    }
    fn focus(window: bool, block: Option<&str>) -> Input {
        Input::Focus { conn_id: "c1".into(), report: FocusReport { window_focused: window, block_id: block.map(Into::into) } }
    }
    fn shows(a: &[Action]) -> Vec<&Notification> {
        a.iter().filter_map(|x| if let Action::Show(n) = x { Some(n) } else { None }).collect()
    }
    fn retracts(a: &[Action]) -> usize {
        a.iter().filter(|x| matches!(x, Action::Retract { .. })).count()
    }

    #[test]
    fn debounces_then_shows_when_app_unfocused() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(focus(false, None), 0, &s);
        assert!(p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s).is_empty());
        assert!(shows(&p.step(Input::Tick, 9_999, &s)).is_empty());
        let a = p.step(Input::Tick, 10_000, &s);
        let n = shows(&a);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "lark finished");
        // Already shown → not shown again on later ticks.
        assert!(p.step(Input::Tick, 20_000, &s).is_empty());
    }

    #[test]
    fn resolve_inside_debounce_drops_silently() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
        let a = p.step(Input::Resolve { block_id: "b1".into(), family: Family::Input }, 3_000, &s);
        assert!(a.is_empty(), "never shown → nothing to retract");
        assert!(p.step(Input::Tick, 10_000, &s).is_empty());
        assert_eq!(p.live_len(), 0);
    }

    #[test]
    fn resolve_after_show_retracts() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
        assert_eq!(shows(&p.step(Input::Tick, 6_000, &s)).len(), 1);
        let a = p.step(Input::Resolve { block_id: "b1".into(), family: Family::Input }, 7_000, &s);
        assert_eq!(retracts(&a), 1);
    }

    #[test]
    fn looking_at_the_block_suppresses_even_attention() {
        let s = Settings { when: When::Always, ..Settings::default() };
        let mut p = PolicyState::new();
        p.step(focus(true, Some("b1")), 0, &s);
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
        assert!(shows(&p.step(Input::Tick, 6_000, &s)).is_empty());
    }

    #[test]
    fn unfocused_mode_suppresses_normal_but_not_attention_when_app_focused_elsewhere() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(focus(true, Some("other")), 0, &s);
        p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s);
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b2")), 0, &s);
        let a = p.step(Input::Tick, 10_000, &s);
        let n = shows(&a);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].kind, NotifyKind::InputWaiting);
    }

    #[test]
    fn focusing_the_block_retracts_its_shown_toasts() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s);
        assert_eq!(shows(&p.step(Input::Tick, 10_000, &s)).len(), 1);
        let a = p.step(focus(true, Some("b1")), 11_000, &s);
        assert_eq!(retracts(&a), 1);
        assert_eq!(p.live_len(), 0);
    }

    #[test]
    fn same_family_replaces_in_place_with_same_tag() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s);
        let first = shows(&p.step(Input::Tick, 10_000, &s))[0].clone();
        // New turn errors: old shown toast is pulled, new one debounces.
        let a = p.step(Input::Emit(req(NotifyKind::TurnErrored, "b1")), 12_000, &s);
        assert_eq!(retracts(&a), 1);
        let second = shows(&p.step(Input::Tick, 22_000, &s))[0].clone();
        assert_eq!(first.tag, second.tag);
        assert_ne!(first.id, second.id);
        assert_eq!(second.title, "lark stopped with an error");
        assert_eq!(p.live_len(), 1);
    }

    #[test]
    fn disabled_master_or_kind_or_never_mode_shows_nothing() {
        let mut p = PolicyState::new();
        let off = Settings { enabled: false, ..Settings::default() };
        p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &off);
        assert_eq!(p.live_len(), 0);

        let mut kinds = HashMap::new();
        kinds.insert(NotifyKind::TurnCompleted, false);
        let kind_off = Settings { kind_enabled: kinds, ..Settings::default() };
        p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &kind_off);
        assert_eq!(p.live_len(), 0);

        let never = Settings { when: When::Never, ..Settings::default() };
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &never);
        assert!(shows(&p.step(Input::Tick, 6_000, &never)).is_empty());
    }

    #[test]
    fn test_kind_is_immediate_and_bypasses_gates() {
        let s = Settings { enabled: false, when: When::Never, ..Settings::default() };
        let mut p = PolicyState::new();
        p.step(focus(true, Some("")), 0, &s);
        let a = p.step(Input::Emit(Request { kind: NotifyKind::Test, block_id: String::new(), agent_name: String::new(), body: None }), 0, &s);
        assert_eq!(shows(&a).len(), 1);
        assert_eq!(shows(&a)[0].title, "AgentMux notifications are working");
    }

    #[test]
    fn click_ack_retracts_and_activates_dismiss_only_retracts() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
        let id = shows(&p.step(Input::Tick, 6_000, &s))[0].id.clone();
        let a = p.step(Input::Ack { id: id.clone(), clicked: true }, 7_000, &s);
        assert_eq!(retracts(&a), 1);
        assert!(a.contains(&Action::Activate { id, block_id: "b1".into() }));

        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b2")), 8_000, &s);
        let id2 = shows(&p.step(Input::Tick, 14_000, &s))[0].id.clone();
        let a = p.step(Input::Ack { id: id2, clicked: false }, 15_000, &s);
        assert_eq!(retracts(&a), 1);
        assert!(!a.iter().any(|x| matches!(x, Action::Activate { .. })));
    }

    #[test]
    fn unknown_ack_is_a_noop() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        assert!(p.step(Input::Ack { id: "forged".into(), clicked: true }, 0, &s).is_empty());
    }

    #[test]
    fn preview_none_strips_body() {
        let s = Settings { preview: Preview::None, ..Settings::default() };
        let mut p = PolicyState::new();
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
        assert_eq!(shows(&p.step(Input::Tick, 6_000, &s))[0].body, None);
    }

    #[test]
    fn tag_is_stable_and_short() {
        assert_eq!(tag_for("input:b1"), tag_for("input:b1"));
        assert_ne!(tag_for("input:b1"), tag_for("turn:b1"));
        assert_eq!(tag_for("x").len(), 16);
    }
}
