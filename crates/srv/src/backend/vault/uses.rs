// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's use of a Vault entry, waiting for the person to allow it
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.4).
//!
//! The agent asks (`Browser` `fill` with `vault`); every window shows the person
//! the entry, the site and the field; their answer reaches srv only through the
//! host's Vault route. An agent can ask; it can never answer.
//!
//! srv keeps each request's deadline
//! (SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §5): a use of an
//! API key, login or note goes ahead once if nobody answers in [`VETO`]; any
//! other entry, and a value the person must type, waits for a yes until the
//! request's own wait runs out, and is then denied. The person's activity in the
//! request holds its countdown ([`Uses::hold`]).

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Default, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct UseRequest {
    pub id: String,
    /// The agent, as its verified identity names it.
    pub agent: String,
    /// The asking agent's own pane, where the person answers it; empty when it
    /// has none.
    pub block_id: String,
    pub entry_id: String,
    pub entry_label: String,
    /// What is being filled: "Card number", "Social Security number".
    pub what: String,
    /// The site the field is on (its frame's origin).
    pub origin: String,
    /// The field, as a person reads it.
    pub field: String,
    /// Whether "Always on this site" is offered: only when the field looks like
    /// what the entry is.
    pub offer_always: bool,
    /// Why the person is being asked: it is a new site, or the field doesn't look
    /// like what the entry is.
    pub reason: String,
    /// The agent's own reason for asking, one line of at most 300 characters; shown as "<agent> says: ...",
    /// never used to decide anything. Empty for a use no agent asked for.
    pub agent_reason: String,
    /// There is no entry yet: the person types the value in the prompt, and may
    /// keep it in the Vault (spec section 6.5).
    pub needs_value: bool,
    /// The kind of entry that would be kept, when `needs_value`.
    pub kind: String,
    /// A command the agent runs with the value in its environment (spec section 7.3);
    /// empty for a web form. `origin` and `field` are then empty too.
    pub command: String,
    /// The command's working directory, when `command` is set.
    pub cwd: String,
    /// What `command` prints replaces the entry's value (`capture`); nothing is
    /// given to the command.
    pub replaces: bool,
    /// An agent's sign-in with a saved `login` (`Browser` `signin`): `field` is
    /// then the masked user name, and "Always" is for this agent on this site.
    pub signin: bool,
    #[ts(type = "number")]
    pub requested_ms: u64,
    /// When silence decides: it goes ahead then if `goes_ahead`, else it is
    /// denied. Set by [`Uses::ask`]; a hold moves it.
    #[ts(type = "number")]
    pub deadline_ms: u64,
    /// The latest a hold can move `deadline_ms` to: the request's own wait.
    #[ts(type = "number")]
    pub expires_ms: u64,
    /// Nobody answering lets it go ahead, once.
    pub goes_ahead: bool,
}

/// How long a use that may go ahead on silence waits for a Deny.
pub const VETO: Duration = Duration::from_secs(30);
/// How long the person's last activity in a request holds its countdown.
pub const HOLD: Duration = Duration::from_secs(15);

/// Whether a use of a `kind` entry goes ahead when nobody answers: an API
/// key, a login or a note does; an ID, a card or a bank account never does,
/// nor does a value only the person can type.
pub fn goes_ahead_on_silence(kind: &str, needs_value: bool) -> bool {
    !needs_value && matches!(kind, "api-key" | "login" | "note")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Once,
    Always,
    Deny,
}

impl Decision {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "once" => Some(Self::Once),
            "always" => Some(Self::Always),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Who decided a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Person,
    /// Nobody answered by its deadline.
    Silence,
    /// Too many of this agent's requests waited, and this was the oldest.
    Crowded,
}

/// What the person answered. `value` is what they typed when the entry didn't
/// exist; it lives only as long as the fill does.
pub struct UseAnswer {
    pub decision: Decision,
    pub value: Option<zeroize::Zeroizing<String>>,
    /// Keep the typed value in the Vault.
    pub save: bool,
    pub by: By,
}

impl UseAnswer {
    pub fn of(decision: Decision) -> Self {
        Self { decision, value: None, save: false, by: By::Person }
    }

    fn by(decision: Decision, by: By) -> Self {
        Self { by, ..Self::of(decision) }
    }
}

/// How a use request ended, as its caller tells the agent and the Activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Once,
    Always,
    /// Nobody answered in [`VETO`], so it went ahead, once.
    WentAhead,
    Denied,
    /// An entry that needs a yes, and nobody gave one.
    Expired,
    Crowded,
    /// The caller stopped waiting before srv decided.
    NoAnswer,
}

impl Ended {
    /// `answer` as it came, or `None` if the caller's own wait ran out.
    pub fn of(answer: Option<&UseAnswer>) -> Self {
        match answer.map(|a| (a.decision, a.by)) {
            None => Self::NoAnswer,
            Some((Decision::Once, By::Silence)) => Self::WentAhead,
            Some((Decision::Once, _)) => Self::Once,
            Some((Decision::Always, _)) => Self::Always,
            Some((Decision::Deny, By::Silence)) => Self::Expired,
            Some((Decision::Deny, By::Crowded)) => Self::Crowded,
            Some((Decision::Deny, By::Person)) => Self::Denied,
        }
    }

    pub fn allowed(self) -> bool {
        matches!(self, Self::Once | Self::Always | Self::WentAhead)
    }

    /// The Activity line's outcome, saying who decided (spec §5.2).
    pub fn outcome(self) -> &'static str {
        match self {
            Self::Once | Self::Always | Self::WentAhead => "done",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Crowded => "dropped",
            Self::NoAnswer => "timed out",
        }
    }

    /// What the agent is told when it didn't go ahead; `nothing` says what
    /// didn't happen ("nothing was typed").
    pub fn refusal(self, nothing: &str) -> String {
        match self {
            Self::Expired => format!("an ID or card entry is used only on a yes, and nobody gave one: {nothing}; ask the person"),
            Self::Crowded => format!("too many of your Vault requests were waiting, and this was the oldest: {nothing}; ask again"),
            Self::NoAnswer => format!("the person didn't answer in time: {nothing}; ask again when they are there"),
            _ => format!("the person said no: {nothing}"),
        }
    }
}

/// The Activity outcome of a use nobody answered that went ahead, on a line of
/// its own before the use's own line: the Vault pane filters these.
pub const WENT_AHEAD_OUTCOME: &str = "approved: no answer in 30 s";
/// What the agent is told when its use went ahead on silence.
pub const WENT_AHEAD: &str = "no one answered in 30 s, so it went ahead";

#[derive(Default)]
pub struct Uses {
    pending: Mutex<Vec<(UseRequest, oneshot::Sender<UseAnswer>)>>,
}

pub fn uses() -> &'static Uses {
    static USES: OnceLock<Uses> = OnceLock::new();
    USES.get_or_init(Uses::default)
}

/// At most this many of one agent's wait at once; its oldest is denied.
const MAX_PER_AGENT: usize = 16;
/// At most this many wait at once. A new one when they do pushes out its own
/// agent's oldest, never another agent's: with none of its own, it is denied.
const MAX_PENDING: usize = 64;

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Uses {
    /// Asks the person; `wait` is how long the caller waits at most. Sets the
    /// request's deadline, and decides it on silence when that passes.
    pub fn ask(&'static self, req: UseRequest, wait: Duration) -> oneshot::Receiver<UseAnswer> {
        let (rx, id) = self.enqueue(req, wait, agentmux_common::time::now_ms_u64());
        if let (Some(id), Ok(rt)) = (id, tokio::runtime::Handle::try_current()) {
            rt.spawn(async move {
                while let Some(at) = self.deadline(&id) {
                    let now = agentmux_common::time::now_ms_u64();
                    if at > now {
                        tokio::time::sleep(Duration::from_millis(at - now)).await;
                    } else {
                        self.decide_unanswered(&id, now);
                    }
                }
            });
        }
        rx
    }

    /// Adds `req`, made at `now`, within the limits; the id to watch, if it waits.
    fn enqueue(&self, mut req: UseRequest, wait: Duration, now: u64) -> (oneshot::Receiver<UseAnswer>, Option<String>) {
        let (tx, rx) = oneshot::channel();
        req.goes_ahead = goes_ahead_on_silence(&req.kind, req.needs_value);
        req.expires_ms = now.saturating_add(ms(wait));
        req.deadline_ms = if req.goes_ahead { now.saturating_add(ms(VETO)).min(req.expires_ms) } else { req.expires_ms };
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.retain(|(_, tx)| !tx.is_closed());
        let own = pending.iter().filter(|(r, _)| r.agent == req.agent).count();
        if own >= MAX_PER_AGENT || (pending.len() >= MAX_PENDING && own > 0) {
            let i = pending.iter().position(|(r, _)| r.agent == req.agent).unwrap_or(0);
            let (_, oldest) = pending.remove(i);
            let _ = oldest.send(UseAnswer::by(Decision::Deny, By::Crowded));
        } else if pending.len() >= MAX_PENDING {
            let _ = tx.send(UseAnswer::by(Decision::Deny, By::Crowded));
            return (rx, None);
        }
        let id = req.id.clone();
        pending.push((req, tx));
        (rx, Some(id))
    }

    fn deadline(&self, id: &str) -> Option<u64> {
        self.get(id).map(|r| r.deadline_ms)
    }

    /// Decides `id` if its deadline has passed by `now`: once if it may go
    /// ahead, else denied.
    fn decide_unanswered(&self, id: &str, now: u64) {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let Some(i) = pending.iter().position(|(r, _)| r.id == id && r.deadline_ms <= now) else { return };
        let (r, tx) = pending.remove(i);
        let _ = tx.send(UseAnswer::by(if r.goes_ahead { Decision::Once } else { Decision::Deny }, By::Silence));
    }

    /// The person is reading or acting in request `id`: its countdown holds
    /// until [`HOLD`] after now, never past its own wait. False if nothing
    /// waits under `id`.
    pub fn hold(&self, id: &str) -> bool {
        self.hold_at(id, agentmux_common::time::now_ms_u64())
    }

    fn hold_at(&self, id: &str, now: u64) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let Some((r, _)) = pending.iter_mut().find(|(r, _)| r.id == id) else { return false };
        if r.goes_ahead {
            r.deadline_ms = r.deadline_ms.max(now.saturating_add(ms(HOLD))).min(r.expires_ms);
        }
        true
    }

    /// The request, for the host's answer to check against.
    pub fn get(&self, id: &str) -> Option<UseRequest> {
        let pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.iter().find(|(r, _)| r.id == id).map(|(r, _)| r.clone())
    }

    /// The person's answer; false if nothing waits under `id` (answered, or the
    /// agent stopped waiting).
    pub fn answer(&self, id: &str, answer: UseAnswer) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let Some(i) = pending.iter().position(|(r, _)| r.id == id) else { return false };
        let (_, tx) = pending.remove(i);
        tx.send(answer).is_ok()
    }

    /// The requests still waiting; one whose agent stopped waiting goes.
    pub fn list(&self) -> Vec<UseRequest> {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.retain(|(_, tx)| !tx.is_closed());
        pending.iter().map(|(r, _)| r.clone()).collect()
    }

    /// Drops a request the agent gave up on.
    pub fn forget(&self, id: &str) {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).retain(|(r, _)| r.id != id);
    }
}

#[cfg(test)]
#[path = "uses_tests.rs"]
mod tests;
