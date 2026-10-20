// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The gate in front of `Desktop`: an agent clicking and typing in a native
//! app (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.4–5.6, §5.9).
//!
//! srv decides and remembers; the MCP tool acts. Before any action the tool
//! asks [`authorize`]-side code (`server/app_api/desktop.rs`), which uses this
//! module for:
//! - **grants**: the user allows an agent to drive one app, for a time or for
//!   the agent's session. They live in memory only, and are tied to the pane
//!   (`block_id`) that asked, so a new session asks again;
//! - **the blocklist**: apps that are never driven (password managers, web
//!   browsers, AgentMux itself, trading and crypto apps), editable in settings;
//! - **limits**: at most two input actions a second and 200 per grant;
//! - **Stop**: the user's brake. It revokes an agent's grants, and the agent's
//!   next action is told so once.
//!
//! The user's answer to a prompt comes only through the host
//! (`POST /api/v1/host/desktop_request`), which agents can't impersonate, so a
//! prompt-injected agent can't approve its own request.
//!
//! [`Gate`] is plain data with the clock passed in, so every rule below is
//! tested without a UI.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// `desktop:enabled`: the tool is off unless this is true.
pub const SETTING_ENABLED: &str = "desktop:enabled";
/// `desktop:blocklist`: app names added to the default blocklist.
pub const SETTING_BLOCKLIST: &str = "desktop:blocklist";
/// `desktop:allowlist`: app names removed from the default blocklist.
pub const SETTING_ALLOWLIST: &str = "desktop:allowlist";
/// `desktop:grantminutes`: how long "Allow for a while" lasts.
pub const SETTING_GRANT_MINUTES: &str = "desktop:grantminutes";
pub const DEFAULT_GRANT_MINUTES: u64 = 30;

/// Input actions allowed per grant before the user is asked again.
pub const MAX_ACTIONS_PER_GRANT: u32 = 200;
/// Two input actions a second.
pub const MIN_ACTION_GAP_MS: u64 = 500;
/// How long a prompt waits for the user.
pub const ANSWER_WAIT: Duration = Duration::from_secs(10 * 60);
/// A grant shows as "Driving" this long after its last action.
pub const DRIVING_SHOWN_MS: u64 = 10_000;

/// An agent waiting for the user's yes or no on one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DesktopRequest {
    pub id: String,
    /// The agent that asked, by name.
    pub agent: String,
    /// The app, as its executable's name without the extension.
    pub app: String,
    /// The window's title when it asked.
    pub title: String,
    #[ts(type = "number")]
    pub asked_ms: u64,
    /// The asking agent's pane.
    pub block_id: String,
    /// How long "Allow for a while" lasts, for the prompt's button.
    #[ts(type = "number")]
    pub grant_minutes: u64,
}

/// An app an agent may drive right now, for the pane's "Driving" badge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DesktopGrantView {
    pub agent: String,
    pub app: String,
    pub block_id: String,
    /// When a timed grant ends; `None` for "until this session ends".
    #[ts(type = "number | null")]
    pub until_ms: Option<u64>,
    /// Whether an action ran in the last [`DRIVING_SHOWN_MS`].
    pub active: bool,
}

/// `app:desktop`: prompts waiting for the user, and the grants in force.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DesktopEvent {
    pub requests: Vec<DesktopRequest>,
    pub grants: Vec<DesktopGrantView>,
}

/// What the user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// Allow for `desktop:grantminutes`.
    Timed,
    /// Allow until this agent's session ends.
    Session,
    Deny,
}

/// The user's answer, relayed by the host (`POST /api/v1/host/desktop_request`).
#[derive(Debug, Clone, Deserialize)]
pub struct DesktopAnswer {
    pub id: String,
    pub choice: Choice,
}

#[derive(Debug)]
struct Grant {
    id: String,
    agent: String,
    app: String,
    block_id: String,
    until_ms: Option<u64>,
    actions: u32,
    last_action_ms: u64,
}

struct Pending {
    request: DesktopRequest,
    waiters: Vec<oneshot::Sender<Choice>>,
}

/// What [`Gate::check`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Allow { grant_id: String },
    /// The user pressed Stop since the agent's last action.
    Stopped,
    /// The user said no to this app earlier in this session.
    Denied,
    /// Too soon after the last action; try again in `wait_ms`.
    TooFast { wait_ms: u64 },
    /// No grant: the user has to be asked.
    Ask,
}

#[derive(Default)]
pub struct Gate {
    pending: Vec<Pending>,
    grants: Vec<Grant>,
    denied: HashSet<(String, String)>,
    stopped: HashSet<String>,
}

impl Gate {
    fn purge(&mut self, now: u64) {
        self.grants.retain(|g| g.until_ms.map_or(true, |u| u > now));
    }

    /// Whether `agent` (in pane `block_id`) may act on `app` now, counting
    /// this as an action when it may.
    pub fn check(&mut self, agent: &str, block_id: &str, app: &str, now: u64) -> Check {
        self.purge(now);
        if self.stopped.remove(agent) {
            return Check::Stopped;
        }
        if self.denied.contains(&(agent.to_string(), app.to_string())) {
            return Check::Denied;
        }
        let Some(pos) = self
            .grants
            .iter()
            .position(|g| g.agent == agent && g.app == app && g.block_id == block_id)
        else {
            return Check::Ask;
        };
        let g = &mut self.grants[pos];
        if g.actions >= MAX_ACTIONS_PER_GRANT {
            self.grants.remove(pos);
            return Check::Ask;
        }
        let since = now.saturating_sub(g.last_action_ms);
        if g.actions > 0 && since < MIN_ACTION_GAP_MS {
            return Check::TooFast { wait_ms: MIN_ACTION_GAP_MS - since };
        }
        g.actions += 1;
        g.last_action_ms = now;
        Check::Allow { grant_id: g.id.clone() }
    }

    /// Queue a prompt for `request`; the answer arrives on the receiver. The
    /// same agent asking again about the same app joins the prompt it has.
    pub fn ask(&mut self, request: DesktopRequest) -> oneshot::Receiver<Choice> {
        let (tx, rx) = oneshot::channel();
        if let Some(p) = self
            .pending
            .iter_mut()
            .find(|p| p.request.agent == request.agent && p.request.app == request.app && p.request.block_id == request.block_id)
        {
            p.waiters.push(tx);
        } else {
            self.pending.push(Pending { request, waiters: vec![tx] });
        }
        rx
    }

    /// The user's answer to prompt `id`.
    pub fn answer(&mut self, id: &str, choice: Choice, now: u64, grant_minutes: u64) -> Result<(), String> {
        let Some(pos) = self.pending.iter().position(|p| p.request.id == id) else {
            return Err("that request was already answered".into());
        };
        let p = self.pending.remove(pos);
        let r = &p.request;
        match choice {
            Choice::Deny => {
                self.denied.insert((r.agent.clone(), r.app.clone()));
            }
            Choice::Timed | Choice::Session => {
                let until_ms = (choice == Choice::Timed).then(|| now + grant_minutes.max(1) * 60_000);
                self.grants.retain(|g| !(g.agent == r.agent && g.app == r.app && g.block_id == r.block_id));
                self.grants.push(Grant {
                    id: uuid::Uuid::new_v4().to_string(),
                    agent: r.agent.clone(),
                    app: r.app.clone(),
                    block_id: r.block_id.clone(),
                    until_ms,
                    actions: 0,
                    last_action_ms: 0,
                });
            }
        }
        for tx in p.waiters {
            let _ = tx.send(choice);
        }
        Ok(())
    }

    /// A prompt nobody answered in time: take it back.
    pub fn abandon(&mut self, id: &str) {
        self.pending.retain(|p| p.request.id != id);
    }

    /// The user's Stop for the agent in pane `block_id`: its grants end and
    /// its next action hears so. Prompts it has waiting are dropped.
    pub fn stop(&mut self, block_id: &str) -> usize {
        let agents: HashSet<String> = self
            .grants
            .iter()
            .filter(|g| g.block_id == block_id)
            .map(|g| g.agent.clone())
            .chain(self.pending.iter().filter(|p| p.request.block_id == block_id).map(|p| p.request.agent.clone()))
            .collect();
        let before = self.grants.len();
        self.grants.retain(|g| g.block_id != block_id);
        self.pending.retain(|p| p.request.block_id != block_id);
        let revoked = before - self.grants.len();
        self.stopped.extend(agents);
        revoked
    }

    /// Whether `agent` in pane `block_id` holds a grant on `app` right now.
    pub fn has_grant(&mut self, agent: &str, block_id: &str, app: &str, now: u64) -> bool {
        self.purge(now);
        self.grants.iter().any(|g| g.agent == agent && g.app == app && g.block_id == block_id)
    }

    /// The agent gives its own grant on `app` back.
    pub fn release(&mut self, agent: &str, block_id: &str, app: &str) -> bool {
        let before = self.grants.len();
        self.grants.retain(|g| !(g.agent == agent && g.block_id == block_id && g.app == app));
        before != self.grants.len()
    }

    pub fn event(&mut self, now: u64) -> DesktopEvent {
        self.purge(now);
        DesktopEvent {
            requests: self.pending.iter().map(|p| p.request.clone()).collect(),
            grants: self
                .grants
                .iter()
                .map(|g| DesktopGrantView {
                    agent: g.agent.clone(),
                    app: g.app.clone(),
                    block_id: g.block_id.clone(),
                    until_ms: g.until_ms,
                    active: g.actions > 0 && now.saturating_sub(g.last_action_ms) < DRIVING_SHOWN_MS,
                })
                .collect(),
        }
    }
}

static GATE: Mutex<Option<Gate>> = Mutex::new(None);

/// Run `f` on the process-wide gate.
pub fn with_gate<T>(f: impl FnOnce(&mut Gate) -> T) -> T {
    let mut guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Gate::default))
}

// ---------------------------------------------------------------------------
// The blocklist (spec §5.5)

/// Apps never driven unless the user takes them off the list. Matched against
/// an executable's name (no extension, lower case). A convenience, not
/// coverage: the user's own list sits beside it.
const DEFAULT_BLOCKED: &[(&str, &str)] = &[
    // password managers
    ("keepass", "a password manager"),
    ("keepassxc", "a password manager"),
    ("1password", "a password manager"),
    ("bitwarden", "a password manager"),
    ("lastpass", "a password manager"),
    ("dashlane", "a password manager"),
    ("enpass", "a password manager"),
    ("nordpass", "a password manager"),
    ("roboform", "a password manager"),
    // web browsers: the Browser tool has the guards
    ("chrome", "a web browser; use the Browser tool"),
    ("msedge", "a web browser; use the Browser tool"),
    ("firefox", "a web browser; use the Browser tool"),
    ("brave", "a web browser; use the Browser tool"),
    ("opera", "a web browser; use the Browser tool"),
    ("vivaldi", "a web browser; use the Browser tool"),
    ("chromium", "a web browser; use the Browser tool"),
    ("iexplore", "a web browser; use the Browser tool"),
    ("safari", "a web browser; use the Browser tool"),
    ("arc", "a web browser; use the Browser tool"),
    ("librewolf", "a web browser; use the Browser tool"),
    ("waterfox", "a web browser; use the Browser tool"),
    // the OS asking the user to confirm something
    ("consent", "an operating system prompt"),
    ("credentialuibroker", "an operating system prompt"),
    ("logonui", "an operating system prompt"),
    ("sechealthui", "Windows Security"),
    // trading and crypto
    ("metatrader", "a trading app"),
    ("terminal64", "a trading app"),
    ("tradingview", "a trading app"),
    ("thinkorswim", "a trading app"),
    ("tws", "a trading app"),
    ("ledger live", "a crypto wallet"),
    ("ledgerlive", "a crypto wallet"),
    ("exodus", "a crypto wallet"),
    ("electrum", "a crypto wallet"),
    ("coinbase", "a crypto app"),
    ("binance", "a crypto app"),
];

/// An executable name as the blocklist compares it: file name, no extension,
/// lower case.
pub fn app_key(exe: &str) -> String {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name);
    name.trim().to_lowercase()
}

/// Why `app` (an executable name) with window `title` must not be driven, or
/// `None`. `added` extends the defaults; `allowed` takes names off them. An
/// entry matches the app's name exactly, or any part of the window title.
pub fn blocked_reason(app: &str, title: &str, added: &[String], allowed: &[String]) -> Option<String> {
    let app = app_key(app);
    let title = title.to_lowercase();
    let norm = |s: &String| app_key(s);
    let allowed: HashSet<String> = allowed.iter().map(norm).collect();
    if app.starts_with("agentmux") {
        return Some("AgentMux itself; use the UI tool".into());
    }
    if let Some((_, why)) = DEFAULT_BLOCKED.iter().find(|(name, _)| *name == app && !allowed.contains(*name)) {
        return Some((*why).to_string());
    }
    if (title.contains("windows security") || title.contains("user account control")) && !allowed.contains("windowssecurity") {
        return Some("an operating system prompt".into());
    }
    for entry in added {
        let e = norm(entry);
        if e.is_empty() {
            continue;
        }
        if e == app || title.contains(&e) {
            return Some(format!("on your blocklist ({entry})"));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The audit log (spec §5.10)

/// Append one line to `desktop-audit.log`, next to the window-capture audit
/// log. Best effort: failing to write it never stops the tool. Typed text is
/// never in an entry, only its length.
pub fn audit(entry: &serde_json::Value) {
    let Ok(root) = agentmux_common::data_paths::agentmux_root() else {
        return;
    };
    let dir = root.join("tmp/capture-window");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("desktop-audit.log")) {
        use std::io::Write;
        let _ = f.write_all(format!("{entry}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, agent: &str, app: &str, block: &str) -> DesktopRequest {
        DesktopRequest {
            id: id.into(),
            agent: agent.into(),
            app: app.into(),
            title: "t".into(),
            asked_ms: 0,
            block_id: block.into(),
            grant_minutes: 30,
        }
    }

    /// Ask, answer and return the check that follows.
    fn grant(g: &mut Gate, choice: Choice, now: u64) {
        let _rx = g.ask(req("r1", "naki", "notepad", "b1"));
        g.answer("r1", choice, now, 30).unwrap();
    }

    #[test]
    fn with_no_grant_the_user_is_asked() {
        let mut g = Gate::default();
        assert_eq!(g.check("naki", "b1", "notepad", 1_000), Check::Ask);
    }

    #[test]
    fn a_timed_grant_allows_then_expires() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Timed, 0);
        assert!(matches!(g.check("naki", "b1", "notepad", 1_000), Check::Allow { .. }));
        // 30 minutes on, it is gone.
        assert_eq!(g.check("naki", "b1", "notepad", 30 * 60_000 + 1), Check::Ask);
    }

    #[test]
    fn a_session_grant_does_not_expire_but_is_tied_to_its_pane() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Session, 0);
        assert!(matches!(g.check("naki", "b1", "notepad", 999_999_999), Check::Allow { .. }));
        // The same agent in a new pane (a new session) is asked again.
        assert_eq!(g.check("naki", "b2", "notepad", 999_999_999 + 1000), Check::Ask);
    }

    #[test]
    fn a_grant_is_for_one_agent_and_one_app() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Timed, 0);
        assert_eq!(g.check("other", "b1", "notepad", 1_000), Check::Ask);
        assert_eq!(g.check("naki", "b1", "calc", 1_000), Check::Ask);
    }

    #[test]
    fn a_denial_is_remembered_so_the_agent_cannot_nag() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Deny, 0);
        assert_eq!(g.check("naki", "b1", "notepad", 1_000), Check::Denied);
        assert_eq!(g.check("naki", "b1", "notepad", 2_000), Check::Denied);
    }

    #[test]
    fn actions_are_at_most_two_a_second() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Timed, 0);
        assert!(matches!(g.check("naki", "b1", "notepad", 10_000), Check::Allow { .. }));
        assert_eq!(g.check("naki", "b1", "notepad", 10_100), Check::TooFast { wait_ms: 400 });
        assert!(matches!(g.check("naki", "b1", "notepad", 10_500), Check::Allow { .. }));
    }

    #[test]
    fn a_grant_ends_after_its_action_limit() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Session, 0);
        let mut t = 10_000;
        for _ in 0..MAX_ACTIONS_PER_GRANT {
            assert!(matches!(g.check("naki", "b1", "notepad", t), Check::Allow { .. }));
            t += MIN_ACTION_GAP_MS;
        }
        assert_eq!(g.check("naki", "b1", "notepad", t), Check::Ask);
    }

    #[test]
    fn stop_revokes_and_the_next_action_hears_it_once() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Session, 0);
        assert_eq!(g.stop("b1"), 1);
        assert_eq!(g.check("naki", "b1", "notepad", 1_000), Check::Stopped);
        // Heard once; after that the agent is simply asked again.
        assert_eq!(g.check("naki", "b1", "notepad", 2_000), Check::Ask);
    }

    #[test]
    fn stop_also_drops_a_prompt_the_agent_was_waiting_on() {
        let mut g = Gate::default();
        let _rx = g.ask(req("r1", "naki", "notepad", "b1"));
        g.stop("b1");
        assert!(g.event(0).requests.is_empty());
        assert!(g.answer("r1", Choice::Timed, 0, 30).is_err());
    }

    #[test]
    fn has_grant_says_whether_the_agent_may_drive_the_app() {
        let mut g = Gate::default();
        assert!(!g.has_grant("naki", "b1", "notepad", 0));
        grant(&mut g, Choice::Timed, 0);
        assert!(g.has_grant("naki", "b1", "notepad", 1_000));
        assert!(!g.has_grant("naki", "b2", "notepad", 1_000));
        assert!(!g.has_grant("naki", "b1", "notepad", 30 * 60_000 + 1));
    }

    #[test]
    fn release_gives_a_grant_back() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Session, 0);
        assert!(g.release("naki", "b1", "notepad"));
        assert_eq!(g.check("naki", "b1", "notepad", 1_000), Check::Ask);
        assert!(!g.release("naki", "b1", "notepad"));
    }

    #[test]
    fn asking_twice_about_one_app_is_one_prompt_and_both_hear_the_answer() {
        let mut g = Gate::default();
        let mut a = g.ask(req("r1", "naki", "notepad", "b1"));
        let mut b = g.ask(req("r2", "naki", "notepad", "b1"));
        assert_eq!(g.event(0).requests.len(), 1);
        g.answer("r1", Choice::Timed, 0, 30).unwrap();
        assert_eq!(a.try_recv().unwrap(), Choice::Timed);
        assert_eq!(b.try_recv().unwrap(), Choice::Timed);
    }

    #[test]
    fn an_answered_prompt_cannot_be_answered_again() {
        let mut g = Gate::default();
        let _rx = g.ask(req("r1", "naki", "notepad", "b1"));
        g.answer("r1", Choice::Deny, 0, 30).unwrap();
        assert!(g.answer("r1", Choice::Timed, 0, 30).is_err());
    }

    #[test]
    fn the_event_shows_a_grant_as_driving_only_just_after_an_action() {
        let mut g = Gate::default();
        grant(&mut g, Choice::Session, 0);
        assert!(!g.event(1_000).grants[0].active);
        g.check("naki", "b1", "notepad", 5_000);
        assert!(g.event(6_000).grants[0].active);
        assert!(!g.event(5_000 + DRIVING_SHOWN_MS).grants[0].active);
    }

    #[test]
    fn app_names_are_compared_without_path_extension_or_case() {
        assert_eq!(app_key(r"C:\Program Files\KeePass\KeePass.exe"), "keepass");
        assert_eq!(app_key("/usr/bin/firefox"), "firefox");
        assert_eq!(app_key("Notepad.EXE"), "notepad");
    }

    #[test]
    fn password_managers_browsers_and_agentmux_are_blocked_by_default() {
        let none: &[String] = &[];
        assert!(blocked_reason("KeePass.exe", "", none, none).unwrap().contains("password"));
        assert!(blocked_reason("chrome.exe", "", none, none).unwrap().contains("Browser tool"));
        assert!(blocked_reason("agentmux-0.59.35.exe", "", none, none).unwrap().contains("UI tool"));
        assert!(blocked_reason("consent.exe", "", none, none).unwrap().contains("operating system"));
        assert!(blocked_reason("notepad.exe", "Untitled - Notepad", none, none).is_none());
    }

    #[test]
    fn an_operating_system_prompt_is_recognised_by_its_title() {
        let none: &[String] = &[];
        assert!(blocked_reason("somehost.exe", "User Account Control", none, none).is_some());
    }

    #[test]
    fn the_user_can_add_to_the_blocklist_by_app_or_by_title() {
        let added = vec!["Photoshop".to_string(), "bank".to_string()];
        let none: &[String] = &[];
        assert!(blocked_reason("photoshop.exe", "x", &added, none).unwrap().contains("blocklist"));
        assert!(blocked_reason("notepad.exe", "My Bank - Notepad", &added, none).is_some());
        assert!(blocked_reason("notepad.exe", "notes", &added, none).is_none());
    }

    #[test]
    fn the_user_can_take_a_default_off_the_list() {
        let none: &[String] = &[];
        let allowed = vec!["chrome".to_string()];
        assert!(blocked_reason("chrome.exe", "", none, &allowed).is_none());
        // AgentMux itself can't be taken off.
        let allowed = vec!["agentmux".to_string()];
        assert!(blocked_reason("agentmux.exe", "", none, &allowed).is_some());
    }
}
