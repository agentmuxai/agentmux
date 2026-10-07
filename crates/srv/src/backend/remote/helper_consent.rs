// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Asking before AgentMux's helper is installed on a host
//! (SPEC_REMOTES_PANE_2026_10_05.md §4.9).
//!
//! The helper is software put on someone's machine, so nothing is uploaded
//! until the user says so: in the approval window (crates/cef `ssh_approval`),
//! which agents can neither reach nor answer. `conn:helper` for the host, else
//! the global `conn:helper`, says `ask` (the default), `always` or `never`.
//! An install an agent's pane sets off is always asked about, whatever the
//! setting: letting an agent use a host is not letting it install software
//! there.
//!
//! The server installs the window and the settings ([`install`]) once srv is
//! up; until then (and in tests) there is no one to ask, so nothing installs.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

/// What the settings say for a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Ask,
    Always,
    Never,
}

impl Policy {
    /// `always` and `never` as stored; anything else (unset included) is ask.
    pub fn parse(value: &str) -> Policy {
        match value.trim().to_ascii_lowercase().as_str() {
            "always" => Policy::Always,
            "never" => Policy::Never,
            _ => Policy::Ask,
        }
    }

    /// The host's own setting, else the global one.
    pub fn resolve(host: &str, global: &str) -> Policy {
        if host.trim().is_empty() {
            Policy::parse(global)
        } else {
            Policy::parse(host)
        }
    }
}

/// What to do before installing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Install,
    Refuse,
    Ask,
}

/// `Always` installs and `Never` refuses without asking, for the user's own
/// panes only; an agent's always asks.
pub fn step(policy: Policy, by_agent: bool) -> Step {
    match (policy, by_agent) {
        (_, true) | (Policy::Ask, false) => Step::Ask,
        (Policy::Always, false) => Step::Install,
        (Policy::Never, false) => Step::Refuse,
    }
}

/// The user's answer in the approval window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Answer {
    /// False when the window was closed, timed out or could not open.
    pub answered: bool,
    /// Install.
    pub approve: bool,
    /// "Don't ask again for this host".
    pub remember: bool,
}

/// What an answer does: install or not, and what to store for the host.
pub fn outcome(answer: Answer) -> (bool, Option<&'static str>) {
    let install = answer.answered && answer.approve;
    let remember = (answer.answered && answer.remember).then_some(if install { "always" } else { "never" });
    (install, remember)
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What [`allow_install`] needs from the server.
pub struct Deps {
    /// Ask the question in the approval window over the window showing the
    /// pane (`block_id`; empty: the main window).
    pub ask: Box<dyn Fn(String, serde_json::Value) -> BoxFuture<Result<Answer, String>> + Send + Sync>,
    /// `conn:helper` for the host, and the global one.
    pub settings: Box<dyn Fn(&str) -> (String, String) + Send + Sync>,
    /// Store `conn:helper` for the host.
    pub remember: Box<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>,
}

static DEPS: OnceLock<Deps> = OnceLock::new();

/// Called once, when srv is up.
pub fn install(deps: Deps) {
    let _ = DEPS.set(deps);
}

/// For tests that install the helper: every question is answered Install.
#[cfg(test)]
pub fn install_answering_yes() {
    install(Deps {
        ask: Box::new(|_, _| Box::pin(async { Ok(Answer { answered: true, approve: true, remember: false }) })),
        settings: Box::new(|_| (String::new(), String::new())),
        remember: Box::new(|_, _| Ok(())),
    });
}

/// The most agent panes remembered; past it the oldest go (a pane that old
/// has long since installed the helper, or not).
const MAX_AGENT_PANES: usize = 512;

/// Panes an agent opened on a host, and the agent: an install from one of
/// them is the agent's doing. Kept in memory, not in the pane's meta, which
/// any caller holding srv's key could clear. `opening` counts the agent
/// `pane.open`s in flight per host: until one returns its pane isn't noted
/// yet, so any install on that host asks.
#[derive(Default)]
struct AgentPanes {
    by_block: HashMap<String, String>,
    order: std::collections::VecDeque<String>,
    opening: HashMap<String, u32>,
}

fn agent_panes() -> &'static Mutex<AgentPanes> {
    static PANES: OnceLock<Mutex<AgentPanes>> = OnceLock::new();
    PANES.get_or_init(|| Mutex::new(AgentPanes::default()))
}

fn lock() -> std::sync::MutexGuard<'static, AgentPanes> {
    agent_panes().lock().unwrap_or_else(|e| e.into_inner())
}

/// Hosts the user answered "Not now" for since srv started.
fn declined() -> std::sync::MutexGuard<'static, HashSet<String>> {
    static DECLINED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DECLINED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Whether a durable pane on `connection` runs as a plain SSH terminal
/// instead, because the user doesn't want the helper there (§4.9: "Not now,
/// or Never: the pane falls back"): the host is set to Never, or the user
/// answered "Not now" for it since srv started. Set to Always, never. Also
/// a host the helper can't run on (a Windows host), whatever the setting.
pub fn plain_instead(connection: &str) -> bool {
    if unsupported().contains(&host_key(connection)) {
        return true;
    }
    let Some(deps) = DEPS.get() else {
        return false;
    };
    let (host, global) = (deps.settings)(connection);
    plain_for(
        Policy::resolve(&host, &global),
        declined().contains(&host_key(connection)),
    )
}

/// Hosts found since srv started to be ones the helper can't run on.
fn unsupported() -> std::sync::MutexGuard<'static, HashSet<String>> {
    static UNSUPPORTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    UNSUPPORTED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// `connection`'s host can't run the helper (a Windows host, spec §6.4 of
/// the remote-terminals spec): its durable panes run as plain SSH terminals.
pub fn note_unsupported(connection: &str) {
    unsupported().insert(host_key(connection));
}

/// [`plain_instead`] from the host's policy and whether the user answered
/// "Not now" for it since srv started.
fn plain_for(policy: Policy, declined: bool) -> bool {
    match policy {
        Policy::Always => false,
        Policy::Never => true,
        Policy::Ask => declined,
    }
}

/// Remember an answer for [`plain_instead`]: "Not now" until srv restarts,
/// undone by a later Install. A closed window is no answer.
pub(crate) fn note_answer(connection: &str, answer: Answer) {
    if !answer.answered {
        return;
    }
    let mut declined = declined();
    if answer.approve {
        declined.remove(&host_key(connection));
    } else {
        declined.insert(host_key(connection));
    }
}

/// One host, however it is spelled.
fn host_key(connection: &str) -> String {
    super::ConnTarget::parse(connection)
        .map(|t| t.name())
        .unwrap_or_else(|_| connection.trim().to_string())
}

/// `block_id` was opened by `agent` (a verified agent's `pane.open` on a host).
pub fn note_agent_pane(block_id: &str, agent: &str) {
    if block_id.is_empty() {
        return;
    }
    let mut panes = lock();
    if panes.by_block.insert(block_id.to_string(), agent.to_string()).is_none() {
        panes.order.push_back(block_id.to_string());
    }
    while panes.order.len() > MAX_AGENT_PANES {
        if let Some(old) = panes.order.pop_front() {
            panes.by_block.remove(&old);
        }
    }
}

/// An agent's `pane.open` on a host is in flight while this lives.
pub struct AgentOpening(String);

impl Drop for AgentOpening {
    fn drop(&mut self) {
        let mut panes = lock();
        if let Some(n) = panes.opening.get_mut(&self.0) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                panes.opening.remove(&self.0);
            }
        }
    }
}

/// Call before an agent's `pane.open` on `connection`; hold the guard until
/// the pane is noted ([`note_agent_pane`]).
pub fn begin_agent_open(connection: &str) -> AgentOpening {
    let key = host_key(connection);
    *lock().opening.entry(key.clone()).or_default() += 1;
    AgentOpening(key)
}

fn agent_of(block_id: Option<&str>) -> Option<String> {
    let block_id = block_id.filter(|b| !b.is_empty())?;
    lock().by_block.get(block_id).cloned()
}

fn agent_opening(connection: &str) -> bool {
    lock().opening.contains_key(&host_key(connection))
}

/// The question (§4.9).
pub fn question(connection: &str, agent: Option<&str>) -> serde_json::Value {
    let who = match agent {
        Some(a) => format!("{a} is using a pane on {connection} that needs it. "),
        None => String::new(),
    };
    serde_json::json!({
        "kind": "consent",
        "title": format!("Install AgentMux's helper on {connection}?"),
        "message": format!(
            "{who}The helper enables file browsing and terminals that survive disconnects. \
             It is about 500 KB, in ~/.agentmux-remote on that machine; it needs no root, \
             and you can remove it any time from Remotes."
        ),
        "checkbox": format!("Don't ask again for {connection}"),
        "ok_label": "Install",
        "cancel_label": "Not now",
    })
}

/// Whether the helper may be installed on `connection` now, for the pane
/// `block_id` (`None`: no pane, which counts as an agent's doing). `Err`
/// says why not, for the pane to show.
pub async fn allow_install(connection: &str, block_id: Option<&str>) -> Result<(), String> {
    let deps = DEPS
        .get()
        .ok_or_else(|| "AgentMux could not ask whether to install its helper".to_string())?;
    let agent = agent_of(block_id);
    let by_agent = agent.is_some() || block_id.is_none_or(str::is_empty) || agent_opening(connection);
    let (host, global) = (deps.settings)(connection);
    match step(Policy::resolve(&host, &global), by_agent) {
        Step::Install => return Ok(()),
        Step::Refuse => {
            return Err(format!(
                "installing AgentMux's helper on {connection} is set to Never (change it in Remotes)"
            ))
        }
        Step::Ask => {}
    }
    let answer = (deps.ask)(
        block_id.unwrap_or_default().to_string(),
        question(connection, agent.as_deref()),
    )
    .await?;
    let (install, remember) = outcome(answer);
    if let Some(value) = remember {
        if let Err(e) = (deps.remember)(connection, value) {
            tracing::warn!(connection = %connection, error = %e, "helper install: could not store the answer");
        }
    }
    tracing::info!(connection = %connection, agent = ?agent, install, remember = ?remember, "helper install: asked");
    note_answer(connection, answer);
    if install {
        Ok(())
    } else if answer.answered {
        Err(format!("you chose not to install AgentMux's helper on {connection}"))
    } else {
        Err(format!("nobody answered whether to install AgentMux's helper on {connection}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosts_setting_wins_over_the_global_one() {
        assert_eq!(Policy::resolve("", ""), Policy::Ask);
        assert_eq!(Policy::resolve("", "Always"), Policy::Always);
        assert_eq!(Policy::resolve("never", "always"), Policy::Never);
        assert_eq!(Policy::resolve("ask", "never"), Policy::Ask);
        assert_eq!(Policy::resolve("nonsense", "always"), Policy::Ask);
    }

    #[test]
    fn an_agents_install_is_always_asked_about() {
        assert_eq!(step(Policy::Always, false), Step::Install);
        assert_eq!(step(Policy::Never, false), Step::Refuse);
        assert_eq!(step(Policy::Ask, false), Step::Ask);
        for p in [Policy::Ask, Policy::Always, Policy::Never] {
            assert_eq!(step(p, true), Step::Ask, "{p:?}");
        }
    }

    #[test]
    fn the_four_answers() {
        let a = |approve, remember| Answer { answered: true, approve, remember };
        assert_eq!(outcome(a(true, false)), (true, None), "Install");
        assert_eq!(outcome(a(true, true)), (true, Some("always")), "Always for this host");
        assert_eq!(outcome(a(false, false)), (false, None), "Not now");
        assert_eq!(outcome(a(false, true)), (false, Some("never")), "Never for this host");
        let closed = Answer { answered: false, approve: false, remember: true };
        assert_eq!(outcome(closed), (false, None), "a closed window stores nothing");
    }

    #[test]
    fn a_host_the_user_doesnt_want_the_helper_on_gets_a_plain_terminal() {
        assert!(plain_for(Policy::Never, false), "Never");
        assert!(plain_for(Policy::Ask, true), "Not now, since srv started");
        assert!(!plain_for(Policy::Ask, false), "not asked yet");
        assert!(!plain_for(Policy::Always, true), "Always wins over an earlier Not now");
    }

    #[test]
    fn not_now_is_remembered_until_an_install_and_a_closed_window_is_no_answer() {
        let a = |answered, approve| Answer { answered, approve, remember: false };
        let host = "me@declined-host";
        note_answer(host, a(false, false));
        assert!(!declined().contains(host), "a closed window");
        note_answer(&format!(" {host} "), a(true, false));
        assert!(declined().contains(host), "Not now, however the host is spelled");
        note_answer(host, a(true, true));
        assert!(!declined().contains(host), "a later Install");
    }

    #[test]
    fn a_host_the_helper_cant_run_on_gets_a_plain_terminal_whatever_the_setting() {
        assert!(!plain_instead("me@windows-host"));
        note_unsupported(" me@windows-host ");
        assert!(plain_instead("me@windows-host"));
    }

    #[test]
    fn the_question_names_the_host_and_the_agent() {
        let q = question("db1", Some("korp"));
        assert_eq!(q["kind"], "consent");
        assert!(q["title"].as_str().unwrap().contains("db1"));
        assert!(q["message"].as_str().unwrap().starts_with("korp is using"));
        assert_eq!(q["ok_label"], "Install");
        assert!(!question("db1", None)["message"].as_str().unwrap().contains("is using"));
    }

    #[test]
    fn a_pane_an_agent_opened_is_the_agents() {
        note_agent_pane("blk-agent-opened", "korp");
        assert_eq!(agent_of(Some("blk-agent-opened")).as_deref(), Some("korp"));
        assert_eq!(agent_of(Some("blk-users-own")), None);
        assert_eq!(agent_of(None), None);
    }

    #[test]
    fn a_host_an_agent_is_opening_a_pane_on_asks() {
        assert!(!agent_opening("me@opening-host"));
        let first = begin_agent_open("me@opening-host");
        let second = begin_agent_open(" me@opening-host ");
        assert!(agent_opening("me@opening-host"), "however it is spelled");
        assert!(!agent_opening("me@other-host"));
        drop(first);
        assert!(agent_opening("me@opening-host"), "one still in flight");
        drop(second);
        assert!(!agent_opening("me@opening-host"));
    }

    #[test]
    fn the_agent_panes_kept_are_bounded() {
        for i in 0..MAX_AGENT_PANES + 10 {
            note_agent_pane(&format!("blk-bound-{i}"), "korp");
        }
        let panes = lock();
        assert!(panes.by_block.len() <= MAX_AGENT_PANES);
        assert_eq!(panes.by_block.len(), panes.order.len());
        assert!(!panes.by_block.contains_key("blk-bound-0"), "the oldest went");
    }
}
