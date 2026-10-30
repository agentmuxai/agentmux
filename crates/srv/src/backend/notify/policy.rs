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
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum NotifyKind {
    InputWaiting,
    TurnCompleted,
    TurnErrored,
    /// srv-observed `agentfailure` (not a user stop).
    AgentCrashed,
    /// A jekt that requires a human's review before the agent acts
    /// (`ESCALATE=required`). Body is fixed text — never message content.
    MessageNeedsReview,
    /// Something other than the user asked to shut the agent down; the user
    /// has 30 s to keep it (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5).
    ShutdownPending,
    /// Rate-limit overflow digest (§5.1 step 6).
    Summary,
    Test,
    /// This channel's MuxBus sign-in stopped working, so agents stopped
    /// getting cloud messages (`muxbus::delivery_status`). About no agent;
    /// once per episode.
    CloudSignedOut,
    /// Agents on one account are out of quota (a weekly or unknown limit, not
    /// a session limit) or need a sign-in: one per account episode
    /// (`server::account_episodes`). About no agent; its key is the
    /// account's (`account:<hash>`), never the account itself.
    AccountUnavailable,
    /// A job failed: one per incident, not per run (`jobs::incidents`,
    /// SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.4). Its key is the job's
    /// (`job:<id>`); the job's name stands where an agent's would.
    JobFailed,
    /// A Vault entry's expiry date is near or here: 14 days, 3 days and on the
    /// day, once each (`vault::expiry`, SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md
    /// §15.2). Its key is the entry's (`vault:<id>`); the entry's label stands
    /// where an agent's name would.
    VaultExpiring,
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
            NotifyKind::CloudSignedOut => Some("cloudsignedout"),
            NotifyKind::AccountUnavailable => Some("accountunavailable"),
            NotifyKind::JobFailed => Some("jobfailed"),
            NotifyKind::VaultExpiring => Some("vaultexpiring"),
            // Follows the master switch only: a shutdown the user can still
            // stop must not be silenceable per kind.
            NotifyKind::ShutdownPending => None,
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
            // The window is short: tell the user at once.
            // The episode's scan already waited for the failure to settle.
            NotifyKind::ShutdownPending | NotifyKind::CloudSignedOut | NotifyKind::AccountUnavailable | NotifyKind::JobFailed | NotifyKind::VaultExpiring => 0,
            NotifyKind::Summary | NotifyKind::Test => 0,
        }
    }

    pub fn priority(self) -> Priority {
        match self {
            NotifyKind::InputWaiting
            | NotifyKind::AgentCrashed
            | NotifyKind::MessageNeedsReview
            | NotifyKind::ShutdownPending
            | NotifyKind::CloudSignedOut
            | NotifyKind::AccountUnavailable
            | NotifyKind::JobFailed
            // Recorded as sent when it is emitted, so it must not be gated out.
            | NotifyKind::VaultExpiring => Priority::Attention,
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
            NotifyKind::ShutdownPending => format!("shutdown:{block_id}"),
            NotifyKind::Summary => "summary".to_string(),
            NotifyKind::Test => "test".to_string(),
            NotifyKind::CloudSignedOut => "cloud".to_string(),
            // One per account, or per job: the key already names it.
            NotifyKind::AccountUnavailable | NotifyKind::JobFailed | NotifyKind::VaultExpiring => block_id.to_string(),
        }
    }

    /// Kinds about one agent carry its summary line; the digest and the test
    /// toast are about no agent.
    fn has_summary(self) -> bool {
        !matches!(
            self,
            NotifyKind::Summary
                | NotifyKind::Test
                | NotifyKind::CloudSignedOut
                | NotifyKind::AccountUnavailable
                | NotifyKind::JobFailed
                | NotifyKind::VaultExpiring
        )
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
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "NotifyPriority")]
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
    /// End of the quiet-hours window we're currently inside (epoch ms), or 0.
    /// Behaves like a pause, but is NOT shown as one in the tray (it isn't
    /// something "Resume" can undo — it's a schedule).
    pub quiet_until_ms: i64,
    /// `notify:os:summary` — show the agent's session summary line.
    pub show_summary: bool,
    /// `notify:os:turncompleted:external` — also notify when a turn something
    /// other than the user started finishes (a jekt, a service notice, a
    /// schedule, a background task). Off: such a turn ends quietly unless it
    /// needs the user (a question, a failure), which notifies as always.
    pub external_turns: bool,
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
            quiet_until_ms: 0,
            show_summary: true,
            external_turns: false,
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

    /// The summary line is decoration: off with its own setting, and with
    /// preview=none like the body (rich-content spec §1.2).
    fn summary_on(&self) -> bool {
        self.show_summary && self.preview != Preview::None
    }
}

/// What one frontend window last reported about focus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusReport {
    pub window_focused: bool,
    pub block_id: Option<String>,
    /// The reporting window's own `Window` oid (rich-content spec §3.3 A):
    /// what makes "is that window open" answerable. `None` from an older
    /// frontend.
    pub window_id: Option<String>,
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
    /// The agent's session summary (`term:ambient_summary`), sanitized.
    pub summary: Option<String>,
    /// For `ShutdownPending`: when the window ends and what happens then.
    pub shutdown: Option<ShutdownInfo>,
}

/// What a pending shutdown's notification needs beyond the agent's name: it
/// says what will happen, counts down, and offers Keep running
/// (docs/specs/SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md §3.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownInfo {
    /// Epoch ms the user's window ends.
    pub deadline_ms: i64,
    /// `PendingView::effect`: `stop`, `close_tab` or `close_pane`.
    pub effect: &'static str,
    /// The agent's session is running (the button says "Keep running"), not
    /// already stopped ("Keep open").
    pub running: bool,
    /// The request this notification is about. Kept with the live notification
    /// so its Keep running button can only cancel this request, never a later
    /// one for the same block (#5397).
    pub request_id: String,
    /// The two buttons' words when the countdown has its own (a sensitive operation); absent, the agent close's.
    pub labels: Option<(String, String)>,
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
    /// A pending external shutdown was kept or went ahead (§6.5).
    Shutdown,
    /// This channel's MuxBus sign-in works again, or the user signed out.
    Cloud,
    /// An account episode ended (`server::account_episodes`).
    Account,
    /// A failing job recovered (`jobs::incidents`).
    Job,
}

impl Family {
    fn matches(self, kind: NotifyKind) -> bool {
        match self {
            Family::Input => kind == NotifyKind::InputWaiting,
            Family::Turn => {
                matches!(kind, NotifyKind::TurnCompleted | NotifyKind::TurnErrored | NotifyKind::AgentCrashed)
            }
            Family::Shutdown => kind == NotifyKind::ShutdownPending,
            Family::Cloud => kind == NotifyKind::CloudSignedOut,
            Family::Account => kind == NotifyKind::AccountUnavailable,
            Family::Job => kind == NotifyKind::JobFailed,
        }
    }
}

/// One shown Attention item, for the tray's "needs you" submenu. Titles are
/// app-controlled templates, so they are safe to show in a native menu.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "NotifyAttentionItem")]
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
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "NotifyTrayState")]
pub struct TrayState {
    pub attention: Vec<AttentionItem>,
    #[ts(type = "number")]
    pub paused_until_ms: i64,
}

/// A button on a toast. `id` is one of a closed set (`keep`); `label` is one of
/// two app strings.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "NotifyAction")]
pub struct NotifyAction {
    pub id: String,
    pub label: String,
}

/// What a presenter needs to tick a toast's countdown without asking srv again:
/// time left now (relative, so clock skew never matters) and the title with
/// `{n}` where the whole seconds go.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "NotifyCountdown")]
pub struct Countdown {
    #[ts(type = "number")]
    pub remaining_ms: i64,
    pub title_template: String,
}

/// The payload Presenters render. Every string here is app-controlled or
/// router-sanitized. Exported to TS as `OsNotification` so it can't shadow the
/// DOM's global `Notification`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "OsNotification")]
pub struct Notification {
    pub id: String,
    pub kind: NotifyKind,
    pub priority: Priority,
    pub block_id: String,
    pub agent_name: String,
    pub title: String,
    pub body: Option<String>,
    /// What the agent is working on — a small third line under the body
    /// (rich-content spec §1). Absent when the agent has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<String>,
    /// Buttons the toast offers. App-controlled ids and labels, never text
    /// from an agent. Absent for every kind but a pending shutdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actions: Option<Vec<NotifyAction>>,
    /// A live countdown the presenter ticks, where the OS can show one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub countdown: Option<Countdown>,
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
    /// For a pending shutdown: the request it belongs to (`ShutdownInfo`), and
    /// how much its effect does (`effect_rank`), so a stale emission can't
    /// replace a stronger one.
    request_id: Option<String>,
    effect_rank: u8,
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
    /// The connection whose window most recently reported being focused —
    /// where a click with no pane of its own raises (§3.3 F).
    last_focused_conn: Option<String>,
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
        NotifyKind::InputWaiting => format!("{agent_name} has a question"),
        NotifyKind::TurnCompleted => format!("{agent_name} finished"),
        NotifyKind::TurnErrored => format!("{agent_name} stopped with an error"),
        NotifyKind::AgentCrashed => format!("{agent_name} stopped unexpectedly"),
        NotifyKind::MessageNeedsReview => format!("A message for {agent_name} needs your review"),
        NotifyKind::ShutdownPending => format!("{agent_name} will shut down soon"),
        // Count-bearing; see `summary_title`.
        NotifyKind::Summary => "More agent updates".to_string(),
        NotifyKind::Test => "AgentMux notifications are working".to_string(),
        NotifyKind::CloudSignedOut => "AgentMux is signed out of MuxBus".to_string(),
        NotifyKind::AccountUnavailable => "An account can't run your agents".to_string(),
        NotifyKind::JobFailed => format!("Job failed: {agent_name}"),
        NotifyKind::VaultExpiring => format!("Expiring in the Vault: {agent_name}"),
    }
}

/// `ShutdownInfo::effect` of a sensitive operation's countdown
/// (`SPEC_SUPER_AGENT_2026_10_25.md` §5): it shares the shutdown toast, with its own two words.
pub const SENSITIVE_EFFECT: &str = "sensitive";

/// A pending shutdown's title with `{n}` for the seconds left
/// (spec §3.8): "Closing QTimer in {n} s". The name arrives pre-sanitized.
pub fn shutdown_title_template(effect: &str, agent_name: &str) -> String {
    // A sensitive operation counting down: `agent_name` is what will happen ("Upgrade AgentMux").
    if effect == SENSITIVE_EFFECT {
        return format!("{agent_name} in {{n}} s");
    }
    let verb = if effect == "stop" { "Stopping" } else { "Closing" };
    format!("{verb} {agent_name} in {{n}} s")
}

/// How much a shutdown's effect does, to order emissions for one request.
fn effect_rank(effect: &str) -> u8 {
    match effect {
        "close_pane" => 2,
        "close_tab" => 1,
        _ => 0,
    }
}

/// `template` with its last `{n}` replaced by the seconds: the one that is ours.
/// An agent's name earlier in it may hold `{n}` too; the launcher fills the
/// same one (`fill_countdown`), so the first frame and the ticked ones agree.
pub fn fill_seconds(template: &str, secs: i64) -> String {
    match template.rfind("{n}") {
        Some(i) => format!("{}{}{}", &template[..i], secs, &template[i + 3..]),
        None => template.to_string(),
    }
}

/// Whole seconds left, rounded up, never negative.
pub fn seconds_left(remaining_ms: i64) -> i64 {
    (remaining_ms.max(0) + 999) / 1000
}

/// The two button labels (spec §3.1): "Keep running" for an agent whose
/// session is running, "Keep open" for one that is already stopped.
pub fn keep_label(running: bool) -> &'static str {
    if running { "Keep running" } else { "Keep open" }
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

    /// Whether a connected frontend reports being window `window_id`.
    pub fn window_open(&self, window_id: &str) -> bool {
        self.focus.values().any(|f| f.window_id.as_deref() == Some(window_id))
    }

    /// The window a click should raise when it names none of its own: the
    /// most recently focused one, else any connected window with an id.
    pub fn raise_window(&self) -> Option<String> {
        self.last_focused_conn
            .as_ref()
            .and_then(|c| self.focus.get(c))
            .and_then(|f| f.window_id.clone())
            .or_else(|| {
                let mut ids: Vec<&String> = self.focus.values().filter_map(|f| f.window_id.as_ref()).collect();
                ids.sort();
                ids.first().map(|s| (*s).clone())
            })
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
                let effect_rank = req.shutdown.as_ref().map_or(0, |i| effect_rank(i.effect));
                if let Some(existing) = self.live.get_mut(&group) {
                    if existing.notification.kind.rank() > req.kind.rank() {
                        return out;
                    }
                    // Emissions for one request are made on separate threads and
                    // can arrive out of order: an older, weaker one (still saying
                    // "Stopping" after the request became a close) must not
                    // replace the newer (#5397).
                    if let (Some(have), Some(info)) = (&existing.request_id, &req.shutdown) {
                        if *have == info.request_id && existing.effect_rank > effect_rank {
                            return out;
                        }
                    }
                    // The same question reported again — srv detected it, then
                    // the renderer mounts the pane and reports it too (the one
                    // kind with two reporters). Idempotent: no retract, no
                    // re-debounce; only fill a body or summary we didn't have.
                    // Other kinds are distinct events (a second crash is news)
                    // and replace as before (Codex P2s on #3662).
                    if existing.notification.kind == req.kind && req.kind == NotifyKind::InputWaiting {
                        if existing.notification.body.is_none() && s.preview != Preview::None {
                            existing.notification.body = req.body;
                        }
                        if existing.notification.summary.is_none() && s.summary_on() {
                            existing.notification.summary = req.summary;
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
                let (title, actions, countdown) = match (req.kind, &req.shutdown) {
                    (NotifyKind::ShutdownPending, Some(info)) => {
                        let remaining_ms = (info.deadline_ms - now_ms).max(0);
                        let template = shutdown_title_template(info.effect, &req.agent_name);
                        (
                            fill_seconds(&template, seconds_left(remaining_ms)),
                            // Cancel and do-it-now. A press that skips or stops the wait is proven
                            // in `notify.ack`, not here: any client holding the key can send one.
                            Some(super::shutdown_buttons::actions(info)),
                            Some(Countdown { remaining_ms, title_template: template }),
                        )
                    }
                    _ => (title_for(req.kind, &req.agent_name), None, None),
                };
                let notification = Notification {
                    id,
                    kind: req.kind,
                    priority: req.kind.priority(),
                    block_id: req.block_id,
                    title,
                    actions,
                    countdown,
                    agent_name: req.agent_name,
                    body: match s.preview {
                        Preview::None => None,
                        _ => req.body,
                    },
                    summary: if s.summary_on() && req.kind.has_summary() { req.summary } else { None },
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
                        request_id: req.shutdown.as_ref().map(|i| i.request_id.clone()),
                        effect_rank,
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
                if app_focused {
                    self.last_focused_conn = Some(conn_id.clone());
                }
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
                if self.last_focused_conn.as_deref() == Some(conn_id.as_str()) {
                    self.last_focused_conn = None;
                }
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

    /// A pending shutdown is never paused or rate-limited: it is the only
    /// warning that an agent stops in 30 s, so holding it back lets the agent
    /// stop unannounced. Only the master switch silences it.
    fn paused(&self, n: &Notification, now_ms: i64, s: &Settings) -> bool {
        !matches!(n.kind, NotifyKind::Test | NotifyKind::ShutdownPending)
            && now_ms < s.pause_until_ms.max(s.quiet_until_ms)
            && !(s.pause_allow_attention && n.priority == Priority::Attention)
    }

    fn rate_limited(&self, n: &Notification, now_ms: i64, replaces_shown: bool) -> bool {
        // A Vault reminder is recorded as sent when emitted, and folding it into the
        // summary would lose it; `vault::expiry` sends at most a few per look.
        if matches!(n.kind, NotifyKind::Test | NotifyKind::Summary | NotifyKind::ShutdownPending | NotifyKind::VaultExpiring) {
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
            summary: None,
            actions: None,
            countdown: None,
            tag: tag_for(&group),
            created_at_ms: now_ms,
        };
        self.live.insert(group, Live { notification: n.clone(), request_id: None, effect_rank: 0, due_at_ms: now_ms, shown: true, replaces_shown: false });
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

    /// The kind, block and (for a shutdown) request of the live notification
    /// with this id: what a button press on it is about. Looked up here, never
    /// read from the press.
    pub fn live_by_id(&self, id: &str) -> Option<(NotifyKind, String, Option<String>)> {
        self.live
            .values()
            .find(|l| l.notification.id == id)
            .map(|l| (l.notification.kind, l.notification.block_id.clone(), l.request_id.clone()))
    }

    /// Number of live (pending or shown) notifications — for tests/diagnostics.
    #[cfg(test)]
    pub fn live_len(&self) -> usize {
        self.live.len()
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
