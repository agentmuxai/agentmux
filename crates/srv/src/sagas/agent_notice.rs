// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent doing something the person may want to stop, told to the person in
//! the agent's own pane (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md):
//! starting an agent, using the person's SSH keys on a host, using a Google
//! connector, adopting memory. It replaces the approval window that opened over
//! the pane and refused when nobody clicked.
//!
//! Nothing waits for a click. [`announce`] decides by the requester's state:
//! - **Waived** (the person's own turn asked for it, or the "Agents may start
//!   agents" setting is on), or the person said **Always** to this agent
//!   before: it goes ahead at once, and the pane gets a one-line notice.
//! - **No window connected** (headless): nobody could stop it, so it goes ahead
//!   at once, and is logged.
//! - Otherwise a **card** with a 30 s countdown: **Stop it**, **Start now**, **Always
//!   allow this agent**. If nobody acts it goes ahead at the deadline. The
//!   requester closed or deleted (no pane left, or its agent stopped) ends it
//!   as [`Outcome::Gone`]. A parked agent is still running, so it does not.
//!
//! The person's choice comes only through `agent.notice.decide`, accepted only
//! from the proven AgentMux window (`operator_only`): an agent can't stop,
//! hurry or grant on the person's behalf, which is the one guard kept. The card
//! is drawn from srv's own record, with the agent's own words only as the quoted
//! brief.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::agent_grants::{self, Topic};
pub use crate::backend::prompt_events::{GrantsEvent, NoticeView, EVENT_AGENT_GRANTS, EVENT_AGENT_NOTICE};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// The person's Stop it, Start now or Always for a countdown.
pub const COMMAND_AGENT_NOTICE_DECIDE: &str = "agent.notice.decide";
/// The person's switch of one agent's "Always" (the pane menu and Settings).
pub const COMMAND_AGENT_NOTICE_SET_GRANT: &str = "agent.notice.setgrant";
/// The agents with an "Always" for a topic.
pub const COMMAND_AGENT_NOTICE_GRANTS: &str = "agent.notice.grants";
/// The one place the length of the wait is set.
pub const COUNTDOWN: Duration = Duration::from_secs(30);
/// How often a waiting countdown looks for its requester.
const WATCH_EVERY: Duration = Duration::from_millis(500);
/// A settled countdown stays answerable (as "too late") this long.
const KEEP_FINISHED_MS: i64 = 60_000;
/// How many events are kept for a window that opens mid-countdown.
const REPLAY_EVENTS: usize = 32;

/// What the notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Starting a worker, creating an agent, or starting workers.
    Spawn,
    /// Running a command on an SSH host with the person's keys.
    SshAccess,
    /// A Google connector action the agent's definition asks to confirm.
    Google,
    /// Adopting memory from the agent's earlier accounts.
    Memory,
}

impl Kind {
    fn wire(self) -> &'static str {
        match self {
            Kind::Spawn => "spawn",
            Kind::SshAccess => "ssh",
            Kind::Google => "google",
            Kind::Memory => "memory",
        }
    }

    fn topic(self) -> Option<Topic> {
        match self {
            Kind::Spawn => Some(Topic::Spawn),
            Kind::Google => Some(Topic::Google),
            Kind::Memory => Some(Topic::Memory),
            Kind::SshAccess => None,
        }
    }
}

/// Why no card was needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiver {
    /// The person's own turn asked for it.
    UserTurn,
    /// The person turned on "Agents may start agents".
    Setting,
}

/// What the person is told, and what a grant is keyed by. Built by srv from its own record.
#[derive(Debug, Clone)]
pub struct Notice {
    pub kind: Kind,
    /// The requester's pane.
    pub block_id: String,
    /// The requester's definition id: what "Always" belongs to. Empty: no "Always".
    pub agent_id: String,
    /// The requester's name.
    pub agent: String,
    /// The SSH host, for [`Kind::SshAccess`].
    pub connection: String,
    /// "Posa is starting Korp-2": one line.
    pub headline: String,
    /// The facts: model, route, who pays.
    pub body: String,
    /// The agent's own words (its brief), shown quoted and clipped; empty for none.
    pub quote: String,
    /// The Always button's words; empty offers none.
    pub always_label: String,
    pub waived: Option<Waiver>,
}

/// Why it goes ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Go {
    Waived(Waiver),
    /// The person said Always earlier.
    Granted,
    /// No window is connected to stop it in.
    Headless,
    /// Nobody acted before the deadline.
    Deadline,
    /// The person pressed Start now.
    Now,
    /// The person pressed Always.
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Go(Go),
    /// The person pressed Stop it.
    Stopped,
    /// The requester is gone: nothing is left to start it for.
    Gone,
}

impl Outcome {
    pub fn goes(self) -> bool {
        matches!(self, Outcome::Go(_))
    }

    fn wire(self) -> &'static str {
        match self {
            Outcome::Go(Go::Now) => "now",
            Outcome::Go(Go::Always) => "always",
            Outcome::Go(_) => "deadline",
            Outcome::Stopped => "stopped",
            Outcome::Gone => "gone",
        }
    }
}

/// Data for `agent.notice.decide`: the card's `id` (from `agent:notice`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentNoticeDecideData {
    pub request_id: String,
    #[ts(type = r#""veto" | "now" | "always""#)]
    pub choice: String,
}

/// How the person's choice came out: `too_late` when the countdown had ended.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentNoticeDecideResult {
    #[ts(type = r#""stopped" | "started" | "always" | "too_late""#)]
    pub outcome: String,
}

/// Data for `agent.notice.setgrant`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentNoticeSetGrantData {
    #[ts(type = r#""spawn" | "google" | "memory""#)]
    pub topic: String,
    /// The agent's definition id.
    pub agent_id: String,
    pub allowed: bool,
}

/// Data for `agent.notice.grants`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentNoticeGrantsData {
    #[ts(type = r#""spawn" | "google" | "memory""#)]
    pub topic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentNoticeGrantsResult {
    pub agents: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Veto,
    Now,
    Always,
}

struct Entry {
    view: NoticeView,
    notice: Notice,
    choice: Option<Choice>,
    wake: Arc<tokio::sync::Notify>,
    finished_at_ms: Option<i64>,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn entries() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

/// The countdowns still running.
pub fn pending() -> Vec<NoticeView> {
    entries().values().filter(|e| e.view.status == "pending").map(|e| e.view.clone()).collect()
}

fn view_of(id: &str, n: &Notice, status: &'static str, opened_ms: i64, deadline_ms: i64) -> NoticeView {
    NoticeView {
        id: id.to_string(),
        kind: n.kind.wire(),
        block_id: n.block_id.clone(),
        agent: n.agent.clone(),
        headline: n.headline.clone(),
        body: n.body.clone(),
        quote: n.quote.clone(),
        always_label: if n.agent_id.is_empty() { String::new() } else { n.always_label.clone() },
        deadline_ms,
        opened_ms,
        status,
        outcome: String::new(),
    }
}

/// Whether the person already said Always to this requester.
fn granted(n: &Notice) -> bool {
    if n.agent_id.is_empty() {
        return false;
    }
    match n.kind.topic() {
        Some(topic) => agent_grants::allowed(topic, &n.agent_id),
        None => crate::backend::remote::agent_access::always_allowed(&n.agent_id, &n.connection),
    }
}

/// Tell the person, then say whether the thing goes ahead. `alive` says whether the requester is
/// still there (its pane exists and its agent runs), asked while the countdown runs.
pub async fn announce(state: &AppState, notice: Notice, alive: impl Fn() -> bool) -> Outcome {
    announce_for(state, notice, alive, COUNTDOWN).await
}

/// [`announce`] with a countdown of `window`.
pub async fn announce_for(state: &AppState, notice: Notice, alive: impl Fn() -> bool, window: Duration) -> Outcome {
    if let Some(why) = notice.waived {
        line(state, &notice);
        return Outcome::Go(Go::Waived(why));
    }
    if granted(&notice) {
        // Only a start is worth a line each time: an SSH command or a connector call is frequent, and
        // its host row (Remotes) or its agent's settings already show the grant.
        if notice.kind == Kind::Spawn {
            line(state, &notice);
        }
        return Outcome::Go(Go::Granted);
    }
    if state.host_ipc.lock().await.is_none() {
        tracing::info!(kind = notice.kind.wire(), agent = %notice.agent, "agent notice: no window is connected to stop it in, so it goes ahead");
        return Outcome::Go(Go::Headless);
    }
    countdown(state, notice, alive, window).await
}

/// The one-line notice, for a start nobody needs to answer.
fn line(state: &AppState, n: &Notice) {
    let now = now_ms();
    let view = view_of(&uuid::Uuid::new_v4().to_string(), n, "line", now, 0);
    state.broker.publish_event::<crate::backend::mps_events::AgentNotice>(&view, vec![], 0);
}

async fn countdown(state: &AppState, notice: Notice, alive: impl Fn() -> bool, window: Duration) -> Outcome {
    let id = uuid::Uuid::new_v4().to_string();
    let opened = now_ms();
    let deadline = tokio::time::Instant::now() + window;
    let view = view_of(&id, &notice, "pending", opened, opened + window.as_millis() as i64);
    let wake = Arc::new(tokio::sync::Notify::new());
    {
        let mut map = entries();
        map.retain(|_, e| e.finished_at_ms.is_none_or(|t| opened - t < KEEP_FINISHED_MS));
        map.insert(id.clone(), Entry { view: view.clone(), notice: notice.clone(), choice: None, wake: wake.clone(), finished_at_ms: None });
    }
    state.broker.publish_event::<crate::backend::mps_events::AgentNotice>(&view, vec![], REPLAY_EVENTS);
    // The pane's waiting tone, tab badge and notification, while it counts.
    let _asking = crate::backend::user_attention::Asking::start(&state.broker, &state.mstore, &notice.block_id, "consent", &notice.headline);
    tracing::info!(kind = notice.kind.wire(), agent = %notice.agent, request_id = %id, "agent notice: the person has {} s to stop it", window.as_secs());

    let mut ending = Ending { broker: state.broker.clone(), id: id.clone(), view, kind: notice.kind.wire(), agent: notice.agent.clone(), done: false };
    let fallback = loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break Outcome::Go(Go::Deadline);
        }
        tokio::select! {
            _ = tokio::time::sleep(left.min(WATCH_EVERY)) => {}
            _ = wake.notified() => {}
        }
        if choice_of(&id).is_some() {
            break Outcome::Go(Go::Deadline);
        }
        if !alive() {
            break Outcome::Gone;
        }
    };
    ending.finish(fallback)
}

/// Ends a countdown however it ends: its caller can be dropped while it waits (the agent's request
/// closed with the agent), and the card must still go.
struct Ending {
    broker: Arc<crate::backend::mps::Broker>,
    id: String,
    view: NoticeView,
    kind: &'static str,
    agent: String,
    done: bool,
}

impl Ending {
    fn finish(&mut self, fallback: Outcome) -> Outcome {
        let outcome = settle(&self.id, fallback);
        self.done = true;
        let mut done = self.view.clone();
        done.status = "cleared";
        done.outcome = outcome.wire().to_string();
        self.broker.publish_event::<crate::backend::mps_events::AgentNotice>(&done, vec![], REPLAY_EVENTS);
        tracing::info!(kind = self.kind, agent = %self.agent, request_id = %self.id, outcome = outcome.wire(), "agent notice: ended");
        outcome
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        if !self.done {
            self.finish(Outcome::Gone);
        }
    }
}

fn choice_of(id: &str) -> Option<Choice> {
    entries().get(id).and_then(|e| e.choice)
}

/// End the countdown `id`: the person's choice if they made one (it wins over `fallback`, however
/// close to the deadline), else `fallback`. After this the countdown answers "too late".
fn settle(id: &str, fallback: Outcome) -> Outcome {
    let mut map = entries();
    let Some(e) = map.get_mut(id) else { return fallback };
    let outcome = match e.choice {
        Some(Choice::Veto) => Outcome::Stopped,
        Some(Choice::Now) => Outcome::Go(Go::Now),
        Some(Choice::Always) => Outcome::Go(Go::Always),
        None => fallback,
    };
    e.view.status = "cleared";
    e.view.outcome = outcome.wire().to_string();
    e.finished_at_ms = Some(now_ms());
    outcome
}

/// The person's choice for countdown `id`. `Always` first remembers the grant (and says so if it
/// could not), then starts the thing.
pub fn decide(state: &AppState, id: &str, choice: &str) -> Result<&'static str, String> {
    let choice = match choice {
        "veto" => Choice::Veto,
        "now" => Choice::Now,
        "always" => Choice::Always,
        other => return Err(format!("INVALID_ARG: choice is 'veto', 'now' or 'always', not '{}'", other.chars().take(20).collect::<String>())),
    };
    // One lock from the check to the grant and the choice: a countdown that ends in between
    // (`settle` takes the same lock) leaves no grant behind for a person who was told "too late".
    let mut map = entries();
    let Some(e) = map.get_mut(id).filter(|e| e.view.status == "pending" && e.choice.is_none()) else {
        return Ok("too_late");
    };
    if choice == Choice::Always {
        remember(state, &e.notice)?;
    }
    e.choice = Some(choice);
    e.wake.notify_one();
    Ok(match choice {
        Choice::Veto => "stopped",
        Choice::Now => "started",
        Choice::Always => "always",
    })
}

/// Write the "Always" the card offered.
fn remember(state: &AppState, n: &Notice) -> Result<(), String> {
    if n.agent_id.is_empty() || n.always_label.is_empty() {
        return Err("INVALID_ARG: this notice offers no Always".to_string());
    }
    match n.kind.topic() {
        Some(topic) => {
            agent_grants::allow(topic, &n.agent_id).map_err(|e| format!("could not remember Always: {e}"))?;
            publish_grants(state, topic);
        }
        None => {
            crate::server::app_api::connections::remember_always_in(&state.broker, &crate::backend::base::get_mux_config_dir(), &n.agent_id, &n.connection)
                .map_err(|e| format!("could not remember Always: {e}"))?;
        }
    }
    Ok(())
}

fn topic_of(name: &str) -> Result<Topic, String> {
    match name {
        "spawn" => Ok(Topic::Spawn),
        "google" => Ok(Topic::Google),
        "memory" => Ok(Topic::Memory),
        other => Err(format!("INVALID_ARG: topic is 'spawn', 'google' or 'memory', not '{}'", other.chars().take(20).collect::<String>())),
    }
}

fn topic_wire(t: Topic) -> &'static str {
    match t {
        Topic::Spawn => "spawn",
        Topic::Google => "google",
        Topic::Memory => "memory",
    }
}

fn publish_grants(state: &AppState, topic: Topic) {
    let event = GrantsEvent { topic: topic_wire(topic), agents: agent_grants::all(topic) };
    state.broker.publish_event::<crate::backend::mps_events::AgentGrants>(&event, vec![], 1);
}

/// `agent.notice.decide`, `agent.notice.setgrant` and `agent.notice.grants`. The first two are
/// accepted only from the proven AgentMux window (`operator_only`), never from an agent: an agent
/// can't stop, hurry or grant on the person's behalf.
pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_NOTICE_DECIDE, move |cmd: CommandAgentNoticeDecideData, ctx| {
        let state = s.clone();
        async move {
            ctx.operator_only("only the person can stop, hurry or auto-approve what an agent does")?;
            let outcome = decide(&state, &cmd.request_id, &cmd.choice)?;
            Ok(AgentNoticeDecideResult { outcome: outcome.to_string() })
        }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_NOTICE_SET_GRANT, move |cmd: CommandAgentNoticeSetGrantData, ctx| {
        let state = s.clone();
        async move {
            ctx.operator_only("only the person can let an agent act without a notice")?;
            let topic = topic_of(&cmd.topic)?;
            if cmd.agent_id.trim().is_empty() {
                return Err("INVALID_ARG: agent_id is required".to_string());
            }
            if cmd.allowed {
                agent_grants::allow(topic, &cmd.agent_id).map_err(|e| e.to_string())?;
            } else {
                agent_grants::revoke(topic, &cmd.agent_id).map_err(|e| e.to_string())?;
            }
            publish_grants(&state, topic);
            Ok(AgentNoticeGrantsResult { agents: agent_grants::all(topic) })
        }
    });
    engine.register_typed(COMMAND_AGENT_NOTICE_GRANTS, move |cmd: CommandAgentNoticeGrantsData, _ctx| async move {
        Ok(AgentNoticeGrantsResult { agents: agent_grants::all(topic_of(&cmd.topic)?) })
    });
}

#[cfg(test)]
#[path = "agent_notice_tests.rs"]
mod tests;
