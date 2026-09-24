// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure notification policy — `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §5.
//!
//! `PolicyState::step(input, now_ms, &settings) -> Vec<Action>` is the whole
//! decision surface: no clocks, no I/O, no broker. `router.rs` owns the side
//! effects (publishing, the tick task). Keeping it pure is what makes the
//! debounce / dedupe / focus-gate / retract rules table-testable.
//!
//! Phase 1: kinds `InputWaiting` / `TurnCompleted` / `TurnErrored` / `Test`,
//! per-kind enable, debounce, dedupe-by-group, focus gate, retract.
//! Phase 2: `AgentCrashed` / `MessageNeedsReview` / `Summary`, pause, rate
//! limits folding into a summary, and a tray state snapshot (§4.2, §5.1).

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
    /// srv-observed `agentfailure` (not a user stop).
    AgentCrashed,
    /// A jekt that requires a human's review before the agent acts
    /// (`ESCALATE=required`). Body is fixed text — never message content.
    MessageNeedsReview,
    /// Rate-limit overflow digest (§5.1 step 6).
    Summary,
    Test,
}

impl NotifyKind {
    /// `notify:os:<suffix>` per-kind enable key (§8).
    pub fn setting_suffix(self) -> Option<&'static str> {
        match self {
            NotifyKind::InputWaiting => Some("inputwaiting"),
            NotifyKind::TurnCompleted => Some("turncompleted"),
            NotifyKind::TurnErrored => Some("turnerrored"),
            NotifyKind::AgentCrashed => Some("agentcrashed"),
            NotifyKind::MessageNeedsReview => Some("messageneedsreview"),
            NotifyKind::Summary | NotifyKind::Test => None,
        }
    }

    /// Debounce before a notification may be shown (§5.1 step 5). Lets the
    /// user's own imminent action (answering, starting the next turn) cancel
    /// it before it ever interrupts them.
    pub fn delay_ms(self) -> i64 {
        match self {
            NotifyKind::InputWaiting => 6_000,
            // Same window as TurnErrored so the two coalesce into one toast.
            NotifyKind::TurnCompleted | NotifyKind::TurnErrored | NotifyKind::AgentCrashed => 10_000,
            NotifyKind::MessageNeedsReview => 2_000,
            NotifyKind::Summary | NotifyKind::Test => 0,
        }
    }

    pub fn priority(self) -> Priority {
        match self {
            NotifyKind::InputWaiting | NotifyKind::AgentCrashed | NotifyKind::MessageNeedsReview => {
                Priority::Attention
            }
            _ => Priority::Normal,
        }
    }

    /// Coalescing key: at most one live notification per (block, family).
    pub fn group_key(self, block_id: &str) -> String {
        match self {
            NotifyKind::InputWaiting => format!("input:{block_id}"),
            // AgentCrashed shares the turn group: one failure, one toast.
            NotifyKind::TurnCompleted | NotifyKind::TurnErrored | NotifyKind::AgentCrashed => {
                format!("turn:{block_id}")
            }
            NotifyKind::MessageNeedsReview => format!("review:{block_id}"),
            NotifyKind::Summary => "summary".to_string(),
            NotifyKind::Test => "test".to_string(),
        }
    }

    /// Within one group, a later event must not replace a more informative
    /// earlier one: srv's `AgentCrashed` (with a reason) outranks the
    /// renderer's generic `TurnErrored`, which often arrives just after it.
    fn rank(self) -> u8 {
        match self {
            NotifyKind::AgentCrashed => 2,
            _ => 1,
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
    /// `notify:pause:until` — epoch ms; toasts are held back until then.
    pub pause_until_ms: i64,
    /// `notify:pause:allowattention` — let Attention through while paused.
    pub pause_allow_attention: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            when: When::Unfocused,
            preview: Preview::Redacted,
            kind_enabled: HashMap::new(),
            pause_until_ms: 0,
            pause_allow_attention: false,
        }
    }
}

/// Rate limits (§5.1 step 6). Attention is exempt from the per-block limit
/// (an agent that finished and then immediately asks something must still
/// reach you) but not from the global one.
pub const PER_BLOCK_WINDOW_MS: i64 = 30_000;
pub const GLOBAL_WINDOW_MS: i64 = 60_000;
pub const GLOBAL_MAX: usize = 4;

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
            Family::Turn => {
                matches!(kind, NotifyKind::TurnCompleted | NotifyKind::TurnErrored | NotifyKind::AgentCrashed)
            }
        }
    }
}

/// One shown Attention item, for the tray's "needs you" submenu. Titles are
/// app-controlled templates, so they are safe to show in a native menu.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NotifyAttentionItem")]
pub struct AttentionItem {
    pub id: String,
    pub block_id: String,
    pub title: String,
    /// Lets surfaces treat kinds differently — e.g. only `InputWaiting`
    /// flashes the taskbar (spec Phase 4).
    pub kind: NotifyKind,
}

/// Snapshot the tray renders (§4.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NotifyTrayState")]
pub struct TrayState {
    pub attention: Vec<AttentionItem>,
    #[ts(type = "number")]
    pub paused_until_ms: i64,
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
    /// Replaces a toast of the same group that was already on screen — it
    /// swaps one toast for another rather than adding one, so the per-block
    /// limit doesn't apply.
    replaces_shown: bool,
}

#[derive(Default)]
pub struct PolicyState {
    /// group_key → the one live (pending or shown) notification for it.
    live: HashMap<String, Live>,
    focus: HashMap<String, FocusReport>,
    next_seq: u64,
    /// block_id → last time a toast for it was shown (per-block limit).
    last_shown: HashMap<String, i64>,
    /// Show times inside the global window.
    recent: std::collections::VecDeque<i64>,
    /// Notifications folded into the current summary.
    summarized: u32,
}

/// Title from a closed template set (§9.1). Only the agent name is
/// interpolated, and it arrives pre-sanitized.
pub fn title_for(kind: NotifyKind, agent_name: &str) -> String {
    match kind {
        NotifyKind::InputWaiting => format!("{agent_name} needs your input"),
        NotifyKind::TurnCompleted => format!("{agent_name} finished"),
        NotifyKind::TurnErrored => format!("{agent_name} stopped with an error"),
        NotifyKind::AgentCrashed => format!("{agent_name} stopped unexpectedly"),
        NotifyKind::MessageNeedsReview => format!("A message for {agent_name} needs your review"),
        // Count-bearing; see `summary_title`.
        NotifyKind::Summary => "More agent updates".to_string(),
        NotifyKind::Test => "AgentMux notifications are working".to_string(),
    }
}

pub fn summary_title(count: u32) -> String {
    if count == 1 {
        "1 more agent update".to_string()
    } else {
        format!("{count} more agent updates")
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
                if let Some(existing) = self.live.get_mut(&group) {
                    if existing.notification.kind.rank() > req.kind.rank() {
                        return out;
                    }
                    // The same question reported again — srv detected it, then
                    // the renderer mounts the pane and reports it too (the one
                    // kind with two reporters). Idempotent: no retract, no
                    // re-debounce; only fill a body we didn't have. Other kinds
                    // are distinct events (a second crash is news) and replace
                    // as before (Codex P2s on #3662).
                    if existing.notification.kind == req.kind && req.kind == NotifyKind::InputWaiting {
                        if existing.notification.body.is_none() && s.preview != Preview::None {
                            existing.notification.body = req.body;
                        }
                        return out;
                    }
                }
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
                    Live {
                        notification,
                        due_at_ms: now_ms + req.kind.delay_ms(),
                        shown: false,
                        replaces_shown: replaced_shown,
                    },
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
                let app_focused = report.window_focused;
                self.focus.insert(conn_id, report);
                // Back in the app: the digest of what you missed is moot.
                if app_focused {
                    self.summarized = 0;
                    self.drop_where(&mut out, |n| n.kind == NotifyKind::Summary);
                }
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
                    if l.notification.kind == NotifyKind::Summary {
                        self.summarized = 0;
                    }
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
        while self.recent.front().is_some_and(|t| now_ms - *t >= GLOBAL_WINDOW_MS) {
            self.recent.pop_front();
        }
        let mut due: Vec<String> = self
            .live
            .iter()
            .filter(|(_, l)| !l.shown && l.due_at_ms <= now_ms)
            .map(|(g, _)| g.clone())
            .collect();
        // Deterministic order: oldest first, so the limits keep the earliest.
        due.sort_by_key(|g| (self.live[g].notification.created_at_ms, g.clone()));
        let mut folded = 0u32;
        for g in due {
            let n = self.live[&g].notification.clone();
            if !self.should_show(&n, s) || self.paused(&n, now_ms, s) {
                // Gated out at due time → gone for good. Not re-offered later:
                // a "finished" toast an hour after the user already saw the
                // pane would be noise.
                self.live.remove(&g);
                continue;
            }
            if self.rate_limited(&n, now_ms, self.live[&g].replaces_shown) {
                self.live.remove(&g);
                folded += 1;
                continue;
            }
            let l = self.live.get_mut(&g).expect("present");
            l.shown = true;
            if n.kind != NotifyKind::Test && n.kind != NotifyKind::Summary {
                self.last_shown.insert(n.block_id.clone(), now_ms);
                self.recent.push_back(now_ms);
            }
            out.push(Action::Show(n));
        }
        if folded > 0 {
            self.summarized += folded;
            out.extend(self.show_summary(now_ms));
        }
        out
    }

    fn paused(&self, n: &Notification, now_ms: i64, s: &Settings) -> bool {
        n.kind != NotifyKind::Test
            && now_ms < s.pause_until_ms
            && !(s.pause_allow_attention && n.priority == Priority::Attention)
    }

    fn rate_limited(&self, n: &Notification, now_ms: i64, replaces_shown: bool) -> bool {
        if matches!(n.kind, NotifyKind::Test | NotifyKind::Summary) {
            return false;
        }
        if self.recent.len() >= GLOBAL_MAX {
            return true;
        }
        n.priority != Priority::Attention
            && !replaces_shown
            && self.last_shown.get(&n.block_id).is_some_and(|t| now_ms - *t < PER_BLOCK_WINDOW_MS)
    }

    /// Show (or replace in place) the one summary toast.
    fn show_summary(&mut self, now_ms: i64) -> Vec<Action> {
        let group = NotifyKind::Summary.group_key("");
        self.next_seq += 1;
        let n = Notification {
            id: format!("n{}-{}", now_ms, self.next_seq),
            kind: NotifyKind::Summary,
            priority: Priority::Normal,
            block_id: String::new(),
            agent_name: String::new(),
            title: summary_title(self.summarized),
            body: Some("Open AgentMux to catch up.".to_string()),
            tag: tag_for(&group),
            created_at_ms: now_ms,
        };
        self.live.insert(group, Live { notification: n.clone(), due_at_ms: now_ms, shown: true, replaces_shown: false });
        vec![Action::Show(n)]
    }

    /// What the tray shows: every on-screen Attention notification, oldest
    /// first, plus the pause deadline.
    pub fn tray_state(&self, s: &Settings, now_ms: i64) -> TrayState {
        let mut attention: Vec<&Notification> = self
            .live
            .values()
            .filter(|l| l.shown && l.notification.priority == Priority::Attention)
            .map(|l| &l.notification)
            .collect();
        attention.sort_by_key(|n| (n.created_at_ms, n.id.clone()));
        TrayState {
            attention: attention
                .into_iter()
                .map(|n| AttentionItem { id: n.id.clone(), block_id: n.block_id.clone(), title: n.title.clone(), kind: n.kind })
                .collect(),
            paused_until_ms: if s.pause_until_ms > now_ms { s.pause_until_ms } else { 0 },
        }
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
    fn req_b(kind: NotifyKind, block: &str) -> Request {
        Request { kind, block_id: block.into(), agent_name: "lark".into(), body: None }
    }

    #[test]
    fn crash_outranks_later_generic_error_in_same_group() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(Request { kind: NotifyKind::AgentCrashed, block_id: "b1".into(), agent_name: "lark".into(), body: Some("Needs you to sign in again".into()) }), 0, &s);
        p.step(Input::Emit(req_b(NotifyKind::TurnErrored, "b1")), 500, &s);
        let a = p.step(Input::Tick, 10_000, &s);
        let n = shows(&a);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].kind, NotifyKind::AgentCrashed);
        assert_eq!(n[0].title, "lark stopped unexpectedly");
        assert_eq!(n[0].body.as_deref(), Some("Needs you to sign in again"));
    }

    #[test]
    fn generic_error_is_upgraded_by_crash() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::TurnErrored, "b1")), 0, &s);
        p.step(Input::Emit(req_b(NotifyKind::AgentCrashed, "b1")), 500, &s);
        let n = shows(&p.step(Input::Tick, 10_500, &s)).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].kind, NotifyKind::AgentCrashed);
    }

    #[test]
    fn pause_holds_back_everything_but_optionally_attention() {
        let mut s = Settings { pause_until_ms: 100_000, ..Settings::default() };
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b1")), 0, &s);
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b2")), 0, &s);
        assert!(shows(&p.step(Input::Tick, 20_000, &s)).is_empty());

        s.pause_allow_attention = true;
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b3")), 30_000, &s);
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b4")), 30_000, &s);
        let a = p.step(Input::Tick, 45_000, &s);
        let n = shows(&a);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].block_id, "b4");

        // Pause expired → normal again.
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b5")), 100_000, &s);
        assert_eq!(shows(&p.step(Input::Tick, 110_000, &s)).len(), 1);
    }

    #[test]
    fn burst_folds_into_one_summary_and_focus_clears_it() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        for i in 0..20 {
            p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, &format!("b{i:02}"))), i, &s);
        }
        let a = p.step(Input::Tick, 10_100, &s);
        let n = shows(&a);
        let normal: Vec<_> = n.iter().filter(|x| x.kind == NotifyKind::TurnCompleted).collect();
        let summaries: Vec<_> = n.iter().filter(|x| x.kind == NotifyKind::Summary).collect();
        assert_eq!(normal.len(), GLOBAL_MAX, "global limit");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "16 more agent updates");

        // A second burst replaces the same summary (same tag), counting up.
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "c1")), 11_000, &s);
        let n2 = shows(&p.step(Input::Tick, 21_000, &s)).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(n2.len(), 1);
        assert_eq!(n2[0].title, "17 more agent updates");
        assert_eq!(n2[0].tag, summaries[0].tag);

        // Focusing the app retracts the summary and resets the count.
        let a = p.step(focus(true, None), 22_000, &s);
        assert_eq!(retracts(&a), 1);
        p.step(focus(false, None), 23_000, &s);
        for i in 0..6 {
            p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, &format!("d{i}"))), 100_000 + i, &s);
        }
        let n3 = shows(&p.step(Input::Tick, 110_100, &s)).into_iter().cloned().collect::<Vec<_>>();
        assert!(n3.iter().any(|x| x.title == "2 more agent updates"), "{n3:?}");
    }

    #[test]
    fn per_block_limit_spares_attention() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b1")), 0, &s);
        assert_eq!(shows(&p.step(Input::Tick, 10_000, &s)).len(), 1);
        // Same block, 5s later: a Normal would be limited, Attention is not.
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 9_000, &s);
        let a = p.step(Input::Tick, 15_000, &s);
        assert_eq!(shows(&a).len(), 1);
        assert_eq!(shows(&a)[0].kind, NotifyKind::InputWaiting);
    }

    #[test]
    fn tray_state_lists_shown_attention_only() {
        let s = Settings { pause_until_ms: 1_000_000, pause_allow_attention: true, ..Settings::default() };
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 0, &s);
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b2")), 0, &s);
        assert!(p.tray_state(&s, 1_000).attention.is_empty(), "pending isn't shown yet");
        p.step(Input::Tick, 10_000, &s);
        let t = p.tray_state(&s, 10_000);
        assert_eq!(t.attention.len(), 1);
        assert_eq!(t.attention[0].block_id, "b1");
        assert_eq!(t.attention[0].title, "lark needs your input");
        assert_eq!(t.attention[0].kind, NotifyKind::InputWaiting);
        assert_eq!(t.paused_until_ms, 1_000_000);
        assert_eq!(p.tray_state(&s, 2_000_000).paused_until_ms, 0);
    }

    #[test]
    fn repeated_same_kind_report_is_idempotent() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        // srv detects the question (no text), then the renderer reports it too.
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 0, &s);
        let a = p.step(Input::Tick, 6_000, &s);
        let first = shows(&a)[0].clone();
        let a = p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 7_000, &s);
        assert!(a.is_empty(), "no retract, no new toast: {a:?}");
        assert!(p.step(Input::Tick, 20_000, &s).is_empty(), "not re-debounced/re-shown");
        let t = p.tray_state(&s, 20_000);
        assert_eq!(t.attention.len(), 1);
        assert_eq!(t.attention[0].id, first.id, "same notification stays live");

        // Pending (not yet shown) duplicate keeps its original due time and
        // picks up the body it was missing.
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b2")), 30_000, &s);
        p.step(Input::Emit(req(NotifyKind::InputWaiting, "b2")), 35_000, &s);
        let n = shows(&p.step(Input::Tick, 36_000, &s)).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(n.len(), 1, "due at 36s from the FIRST report");
        assert_eq!(n[0].body.as_deref(), Some("Which branch?"));
    }

    #[test]
    fn a_second_crash_is_news_not_a_duplicate() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::AgentCrashed, "b1")), 0, &s);
        let first = shows(&p.step(Input::Tick, 10_000, &s))[0].clone();
        let a = p.step(Input::Emit(req_b(NotifyKind::AgentCrashed, "b1")), 60_000, &s);
        assert_eq!(retracts(&a), 1, "old toast pulled for the new crash");
        let second = shows(&p.step(Input::Tick, 70_000, &s))[0].clone();
        assert_ne!(first.id, second.id);
        assert_eq!(first.tag, second.tag);
    }

    #[test]
    fn needs_review_is_attention_with_its_own_group() {
        let s = Settings::default();
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::MessageNeedsReview, "b1")), 0, &s);
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 0, &s);
        let a = p.step(Input::Tick, 6_000, &s);
        assert_eq!(shows(&a).len(), 2);
        assert!(shows(&a).iter().any(|n| n.title == "A message for lark needs your review"));
    }
}
