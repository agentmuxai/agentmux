// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The note each agent gets when it resumes after AgentMux was upgraded
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §5.2,
//! phase 4b): `[AgentMux] AgentMux was upgraded from 0.59.26 to 0.59.27 …`,
//! once per agent, with the first message it gets after it resumes: the note
//! asks nothing of it, so it starts no turn (`reactive::held_notes`). The
//! agent that asked for the upgrade (`App` `upgrade`) gets it too, which is
//! how it learns it is done.
//!
//! srv records the version it runs in the channel folder ([`RECORD_FILE`],
//! like `cli_notice`'s CLI versions): an installed build's data folder is
//! per version, so the new version's own would read as a first start. A
//! start on a newer version than the one recorded is an upgrade; a first
//! start, the same version or an older one (a downgrade, or the older build
//! started again) is not. The record keeps the upgrade, and the panes already
//! told, for [`PENDING_FOR`]: a crash or another restart soon after still
//! tells the agents not yet told, and never one twice.
//!
//! Every agent that registers with the reactive handler, whatever runs it, is
//! a candidate ([`on_registered`]): the note goes to one whose pane has a
//! session (`agent:sessionid`) as it registers, i.e. a resumed one, not a new
//! pane. That is read from its block then ([`has_session`]), not from a list
//! made at start: the quit that precedes an upgrade deletes the panes, and the
//! next start restores them into new blocks once srv is up.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The last AgentMux version this channel ran, in its folder.
pub const RECORD_FILE: &str = "agentmux-last-version.json";
/// How long after an upgrade resumed agents are still told.
pub const PENDING_FOR: Duration = Duration::from_secs(60 * 60);
/// Where the note waits for the agent's next turn.
const NOTE_KEY: &str = "upgrade";
/// How long after registering an agent is told, so its spawn has settled.
const SETTLE: Duration = Duration::from_secs(1);
/// A restored pane's controller can come up after the pane registers, so a
/// note that finds none is tried again, this often, up to [`RETRIES`] times.
pub(crate) const RETRY_GAP: Duration = Duration::from_secs(2);
pub(crate) const RETRIES: u32 = 15;
/// The agents whose turn was running when AgentMux last quit, in the channel
/// folder ([`record_interrupted`]).
pub const INTERRUPTED_FILE: &str = "agentmux-interrupted-turns.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Record {
    version: String,
    at: i64,
    /// Set while the upgrade to `version` is still being announced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upgraded_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upgraded_at: Option<i64>,
    /// Panes already told about it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notified: Vec<String>,
}

/// The agents (by definition id, `agentId` in their pane's meta) whose turn
/// was in flight when the last quit began, and when. A pane's block id is new
/// after a restore, the agent's id is not.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Interrupted {
    at: i64,
    agents: Vec<String>,
}

/// This start is (still announcing) an upgrade from `from` to `to`, made at
/// `at_ms`; `notified` are the panes told already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgraded {
    pub from: String,
    pub to: String,
    pub at_ms: i64,
    pub notified: Vec<String>,
}

fn read(dir: &Path) -> Option<Record> {
    std::fs::read(dir.join(RECORD_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Replace the record (`std::fs::rename` replaces an existing file on
/// Windows too: `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`).
fn write(dir: &Path, record: &Record) {
    let path = dir.join(RECORD_FILE);
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(record)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "upgrade notice: could not save the record");
    }
}

/// Record that `version` started at `now_ms`, and return the upgrade being
/// announced, if any: this start's, or one made less than [`PENDING_FOR`]
/// ago on this same version. A record that can't be read or written is
/// logged and treated as none: at worst a note is missed.
pub fn observe_start(dir: &Path, version: &str, now_ms: i64) -> Option<Upgraded> {
    use crate::backend::update_check::version_core;
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = read(dir);
    let mut record = Record { version: version.to_string(), at: now_ms, ..Default::default() };
    match &previous {
        Some(p) if version_core(&p.version).is_some() && version_core(version) > version_core(&p.version) => {
            record.upgraded_from = Some(p.version.clone());
            record.upgraded_at = Some(now_ms);
        }
        Some(p) if p.version == version && p.upgraded_from.is_some() => {
            let still = p.upgraded_at.is_some_and(|t| now_ms - t < PENDING_FOR.as_millis() as i64);
            if still {
                record.upgraded_from = p.upgraded_from.clone();
                record.upgraded_at = p.upgraded_at;
                record.notified = p.notified.clone();
            }
        }
        _ => {}
    }
    write(dir, &record);
    Some(Upgraded {
        from: record.upgraded_from?,
        to: version.to_string(),
        at_ms: record.upgraded_at?,
        notified: record.notified,
    })
}

/// At quit, before the agents are closed: keep which of them have a turn in
/// flight (`agents`, their ids), for the next start to tell them to carry on.
/// Replaces the last list; none removes it. Unlike a note held for a turn, an
/// interrupted turn is never continued by the CLI by itself on `--resume`.
pub fn record_interrupted(dir: &Path, agents: &[String], now_ms: i64) {
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(INTERRUPTED_FILE);
    if agents.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    write_interrupted(dir, &Interrupted { at: now_ms, agents: agents.to_vec() });
}

fn write_interrupted(dir: &Path, record: &Interrupted) {
    let path = dir.join(INTERRUPTED_FILE);
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(record)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "upgrade notice: could not save the interrupted turns");
    }
}

/// The list [`record_interrupted`] left, if it is less than [`PENDING_FOR`]
/// old; empty otherwise (a quit long ago is not news).
fn read_interrupted(dir: &Path, now_ms: i64) -> Interrupted {
    read_interrupted_any_age(dir).filter(|r| now_ms - r.at < PENDING_FOR.as_millis() as i64).unwrap_or_default()
}

fn read_interrupted_any_age(dir: &Path) -> Option<Interrupted> {
    std::fs::read(dir.join(INTERRUPTED_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// `agent` has been told to carry on: take it off the list.
fn record_continued(dir: &Path, agent: &str) {
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut record = read_interrupted_any_age(dir).unwrap_or_default();
    record.agents.retain(|a| a != agent);
    if record.agents.is_empty() {
        let _ = std::fs::remove_file(dir.join(INTERRUPTED_FILE));
    } else {
        write_interrupted(dir, &record);
    }
}

/// Serializes read-modify-write of the record: panes resume together.
static RECORD_LOCK: Mutex<()> = Mutex::new(());

/// Add `block_id` to the panes told, in the record.
fn record_notified(dir: &Path, block_id: &str) {
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(mut record) = read(dir) {
        if !record.notified.iter().any(|b| b == block_id) {
            record.notified.push(block_id.to_string());
            write(dir, &record);
        }
    }
}

/// " at 14:17", the local time of `ms`.
fn at_text(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format(" at %H:%M").to_string())
        .unwrap_or_default()
}

/// The tools line: the resumed conversation still holds any tool description
/// the agent loaded before the upgrade (a deferred tool's schema is a message
/// in it), while the tool itself is the new version's: the list of actions it
/// read can be missing new ones (browser driver roadblocks, R8).
const TOOLS_MAY_HAVE_CHANGED: &str = "Its tools may have changed: a tool description you loaded before the upgrade can be out of date, so load it again before relying on the actions it lists.";

/// The note, with the upgrade's local time.
pub fn note_text(u: &Upgraded) -> String {
    format!(
        "[AgentMux] AgentMux was upgraded from {} to {}{}. Your session resumed; nothing is needed from you. {TOOLS_MAY_HAVE_CHANGED}",
        u.from,
        u.to,
        at_text(u.at_ms)
    )
}

/// What an agent whose turn was cut off by the quit is told: unlike
/// [`note_text`], it asks something of it, so it starts a turn. `upgrade` is
/// the upgrade this start made, if any; `quit_ms` when AgentMux quit.
pub fn continue_text(upgrade: Option<&Upgraded>, quit_ms: i64) -> String {
    match upgrade {
        Some(u) => format!(
            "[AgentMux] AgentMux was upgraded from {} to {}{} while you were working; your last turn was cut off mid-tool. {TOOLS_MAY_HAVE_CHANGED} Check the state of what you were doing and carry on.",
            u.from,
            u.to,
            at_text(u.at_ms)
        ),
        None => format!(
            "[AgentMux] AgentMux restarted{} while you were working; your last turn was cut off mid-tool. Check the state of what you were doing and carry on.",
            at_text(quit_ms)
        ),
    }
}

struct State {
    /// The upgrade being announced, if this start made one (or one soon before).
    upgraded: Option<Upgraded>,
    /// The agents whose turn the last quit cut off, not yet told to carry on,
    /// and when it quit.
    interrupted: Mutex<HashSet<String>>,
    quit_ms: i64,
    dir: PathBuf,
    /// Where a pane's block says whether it has a session ([`has_session`]).
    store: Arc<crate::backend::storage::store::Store>,
    noted: Mutex<HashSet<String>>,
}

static STATE: OnceLock<Option<State>> = OnceLock::new();

/// At srv start (`bootstrap::background`): record this version and remember
/// whether an upgrade is being announced and which agents' turns the last quit
/// cut off.
pub fn init(store: Arc<crate::backend::storage::store::Store>) {
    let state = agentmux_common::DataPaths::from_env().and_then(|paths| {
        let dir = paths.instance_dir.clone();
        let now = agentmux_common::time::now_ms();
        let upgraded = observe_start(&dir, env!("CARGO_PKG_VERSION"), now);
        let cut_off = read_interrupted(&dir, now);
        if upgraded.is_none() && cut_off.agents.is_empty() {
            return None;
        }
        if let Some(upgraded) = &upgraded {
            tracing::info!(from = %upgraded.from, to = %upgraded.to, "upgrade notice: resumed agents will be told of the upgrade");
            // This start made the upgrade: its data must be the previous version's,
            // or nothing that version's quit saved (windows, tabs, panes) comes back.
            if upgraded.at_ms == now {
                let db_dir = paths.data_dir.join("db");
                if let Some(problem) = crate::bootstrap::carry_forward::continuity_problem(&db_dir, &upgraded.from) {
                    tracing::error!(from = %upgraded.from, to = %upgraded.to, "upgrade continuity: {problem}");
                }
            }
        }
        if !cut_off.agents.is_empty() {
            tracing::info!(count = cut_off.agents.len(), "upgrade notice: agents whose turn the last quit cut off will be told to carry on");
        }
        let noted = Mutex::new(upgraded.iter().flat_map(|u| u.notified.iter().cloned()).collect());
        Some(State {
            upgraded,
            interrupted: Mutex::new(cut_off.agents.into_iter().collect()),
            quit_ms: cut_off.at,
            dir,
            store,
            noted,
        })
    });
    let _ = STATE.set(state);
}

/// The agent definition a pane runs (`agentId` in its meta); empty for a pane
/// that is no agent's.
fn agent_id_of(store: &crate::backend::storage::store::Store, block_id: &str) -> String {
    store
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .map(|b| crate::backend::obj::meta_get_string(&b.meta, "agentId", "").trim().to_string())
        .unwrap_or_default()
}

/// Whether the pane `block_id` has a session (`agent:sessionid`) now: it is a
/// resumed one. Asked as the pane registers, not when srv starts: a graceful
/// quit, an upgrade's, deletes the panes, and the next start restores them
/// into blocks with new ids (carrying their session) after srv is up, so a list
/// read at start would never hold them. A pane opened after the upgrade gets
/// its session only with its first turn, after it registered.
fn has_session(store: &crate::backend::storage::store::Store, block_id: &str) -> bool {
    store
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .is_some_and(|b| !crate::backend::obj::meta_get_string(&b.meta, "agent:sessionid", "").trim().is_empty())
}

/// Why a note wasn't delivered: the pane is gone, or it may be ready later.
pub(crate) enum Undelivered {
    Gone(String),
    Retry(String),
}

/// `deliver` until it succeeds, at most `attempts` times, `gap` apart; the
/// last error when none does. A pane that is gone isn't waited for.
pub(crate) async fn deliver_with_retry(
    mut deliver: impl FnMut() -> Result<(), Undelivered>,
    attempts: u32,
    gap: Duration,
) -> Result<(), String> {
    let mut last = String::from("no attempt made");
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(gap).await;
        }
        match deliver() {
            Ok(()) => return Ok(()),
            Err(Undelivered::Gone(e)) => return Err(e),
            Err(Undelivered::Retry(e)) => last = e,
        }
    }
    Err(last)
}

/// An agent registered in `block_id` (any controller). If an upgrade is being
/// announced and the pane is a resumed one not yet told, tell it, once; if its
/// turn was cut off by the quit, tell it to carry on instead, which starts a
/// turn. Called under the handler lock: no I/O here (the pane's block is read,
/// and the note sent, from a task of its own).
pub fn on_registered(block_id: &str) {
    let Some(Some(state)) = STATE.get() else { return };
    let now = agentmux_common::time::now_ms();
    let open = |at_ms: i64| now - at_ms < PENDING_FOR.as_millis() as i64;
    let upgrade_open = state.upgraded.as_ref().is_some_and(|u| open(u.at_ms));
    let cut_off_open = open(state.quit_ms) && !state.interrupted.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
    if !upgrade_open && !cut_off_open {
        return;
    }
    if !state.noted.lock().unwrap_or_else(|e| e.into_inner()).insert(block_id.to_string()) {
        return;
    }
    let block_id = block_id.to_string();
    tokio::spawn(async move {
        let Some(Some(state)) = STATE.get() else { return };
        // A new pane: no session yet, and no note; one that registers again
        // later, with a session by then, is told then.
        if !has_session(&state.store, &block_id) {
            state.noted.lock().unwrap_or_else(|e| e.into_inner()).remove(&block_id);
            return;
        }
        let agent = agent_id_of(&state.store, &block_id);
        let cut_off = cut_off_open && !agent.is_empty() && state.interrupted.lock().unwrap_or_else(|e| e.into_inner()).contains(&agent);
        let upgrade = state.upgraded.as_ref().filter(|_| upgrade_open);
        let text = if cut_off {
            continue_text(upgrade, state.quit_ms)
        } else if let Some(u) = upgrade {
            note_text(u)
        } else {
            state.noted.lock().unwrap_or_else(|e| e.into_inner()).remove(&block_id);
            return;
        };
        tokio::time::sleep(SETTLE).await;
        let handler = crate::backend::reactive::get_global_handler();
        let delivered = deliver_with_retry(
            || {
                let target = handler
                    .list_agents()
                    .into_iter()
                    .find(|r| r.block_id == block_id)
                    .map(|r| r.uid.unwrap_or(r.agent_id))
                    .ok_or_else(|| Undelivered::Gone("not registered any more".to_string()))?;
                // A turn cut off is continued now; a resumed idle agent is
                // told with its next message.
                let sent = if cut_off { handler.deliver_system_note(&target, &text) } else { handler.hold_system_note(&target, NOTE_KEY, &text) };
                sent.map_err(Undelivered::Retry)
            },
            RETRIES,
            RETRY_GAP,
        )
        .await;
        match delivered {
            Ok(()) => {
                tracing::info!(block_id = %block_id, cut_off, "upgrade notice: delivered");
                if state.upgraded.is_some() {
                    record_notified(&state.dir, &block_id);
                }
                if cut_off {
                    state.interrupted.lock().unwrap_or_else(|e| e.into_inner()).remove(&agent);
                    record_continued(&state.dir, &agent);
                }
            }
            Err(e) => {
                tracing::info!(block_id = %block_id, error = %e, "upgrade notice: not delivered; the next registration tries again");
                state.noted.lock().unwrap_or_else(|e| e.into_inner()).remove(&block_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(from: &str, to: &str, at: i64, notified: &[&str]) -> Option<Upgraded> {
        Some(Upgraded { from: from.into(), to: to.into(), at_ms: at, notified: notified.iter().map(|s| s.to_string()).collect() })
    }

    #[test]
    fn only_a_start_on_a_newer_release_is_an_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        // First start: recorded, nothing to say.
        assert_eq!(observe_start(dir.path(), "0.59.26", 1), None);
        // The same version again.
        assert_eq!(observe_start(dir.path(), "0.59.26", 2), None);
        // Upgraded.
        assert_eq!(observe_start(dir.path(), "0.59.27", 3), up("0.59.26", "0.59.27", 3, &[]));
        // The older build started again isn't one, and the newer one after it
        // is, from the older.
        assert_eq!(observe_start(dir.path(), "0.59.26", 4), None);
        assert_eq!(observe_start(dir.path(), "0.59.100", 5).map(|u| u.from), Some("0.59.26".into()));
    }

    #[test]
    fn an_upgrade_is_announced_across_restarts_for_an_hour_and_to_each_pane_once() {
        let dir = tempfile::tempdir().unwrap();
        observe_start(dir.path(), "0.59.26", 0);
        let hour = PENDING_FOR.as_millis() as i64;
        assert_eq!(observe_start(dir.path(), "0.59.27", 1_000), up("0.59.26", "0.59.27", 1_000, &[]));
        record_notified(dir.path(), "block-a");
        record_notified(dir.path(), "block-a");
        // A crash, then a restart soon after: still announcing, block-a told.
        assert_eq!(observe_start(dir.path(), "0.59.27", 2_000), up("0.59.26", "0.59.27", 1_000, &["block-a"]));
        // An hour after the upgrade, it's just this version starting.
        assert_eq!(observe_start(dir.path(), "0.59.27", 1_000 + hour), None);
        assert_eq!(observe_start(dir.path(), "0.59.27", 2_000 + hour), None);
    }

    #[test]
    fn an_unreadable_record_is_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(RECORD_FILE), b"not json").unwrap();
        assert_eq!(observe_start(dir.path(), "0.59.27", 1), None);
        assert_eq!(observe_start(dir.path(), "0.59.28", 2).map(|u| u.from), Some("0.59.27".into()));
    }

    #[tokio::test]
    async fn a_pane_restored_after_the_start_is_a_resumed_one_when_its_block_has_a_session() {
        use crate::backend::obj::Block;
        let state = crate::server::tests::test_state();
        let block = |oid: &str, session: Option<&str>| {
            let mut b = Block { oid: oid.into(), ..Default::default() };
            if let Some(id) = session {
                b.meta.insert("agent:sessionid".into(), serde_json::json!(id));
            }
            state.mstore.insert(&mut b).unwrap();
        };
        // Created after "start", as a restore does with a quit's saved panes.
        block("restored", Some("28ec4fdb-58a0-4571-9c14-c1b8b877ab60"));
        block("new-pane", None);
        block("blank-session", Some("  "));
        assert!(has_session(&state.mstore, "restored"));
        assert!(!has_session(&state.mstore, "new-pane"), "a new pane gets its session with its first turn");
        assert!(!has_session(&state.mstore, "blank-session"));
        assert!(!has_session(&state.mstore, "no-such-block"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_note_finding_no_controller_yet_is_tried_again_until_it_goes_through() {
        let mut calls = 0;
        let r = deliver_with_retry(
            || {
                calls += 1;
                if calls < 3 { Err(Undelivered::Retry("no controller for block b".into())) } else { Ok(()) }
            },
            15,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!((r, calls), (Ok(()), 3));
        // Never ready: the last error, after every attempt.
        let mut calls = 0;
        let r = deliver_with_retry(
            || {
                calls += 1;
                Err(Undelivered::Retry(format!("no controller (try {calls})")))
            },
            4,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!((r, calls), (Err("no controller (try 4)".to_string()), 4));
        // A pane that is gone isn't waited for.
        let mut calls = 0;
        let r = deliver_with_retry(
            || {
                calls += 1;
                Err(Undelivered::Gone("not registered any more".into()))
            },
            15,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!((r, calls), (Err("not registered any more".to_string()), 1));
    }

    #[test]
    fn the_note_is_a_system_note() {
        let text = note_text(&up("0.59.26", "0.59.27", 1_792_000_000_000, &[]).unwrap());
        assert!(text.starts_with("[AgentMux] AgentMux was upgraded from 0.59.26 to 0.59.27 at "), "{text}");
        assert!(text.contains("Your session resumed; nothing is needed from you. Its tools may have changed: "), "{text}");
        assert!(text.ends_with("load it again before relying on the actions it lists."), "{text}");
    }

    #[test]
    fn the_agents_a_quit_cut_off_are_kept_for_an_hour_and_each_leaves_the_list_once_told() {
        let dir = tempfile::tempdir().unwrap();
        let hour = PENDING_FOR.as_millis() as i64;
        // Nothing recorded: nothing to say.
        assert_eq!(read_interrupted(dir.path(), 5), Interrupted::default());
        record_interrupted(dir.path(), &["agent-a".into(), "agent-b".into()], 1_000);
        assert_eq!(read_interrupted(dir.path(), 2_000).agents, vec!["agent-a".to_string(), "agent-b".to_string()]);
        assert_eq!(read_interrupted(dir.path(), 2_000).at, 1_000);
        // Told: it leaves the list, the other stays, and the file goes with the last one.
        record_continued(dir.path(), "agent-a");
        assert_eq!(read_interrupted(dir.path(), 2_000).agents, vec!["agent-b".to_string()]);
        record_continued(dir.path(), "agent-b");
        assert!(!dir.path().join(INTERRUPTED_FILE).exists());
        // An hour later a list is stale.
        record_interrupted(dir.path(), &["agent-a".into()], 1_000);
        assert_eq!(read_interrupted(dir.path(), 1_000 + hour), Interrupted::default());
        // A quit with no turn running replaces the list with none.
        record_interrupted(dir.path(), &[], 3_000);
        assert!(!dir.path().join(INTERRUPTED_FILE).exists());
    }

    #[test]
    fn an_unreadable_interrupted_list_is_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(INTERRUPTED_FILE), b"not json").unwrap();
        assert_eq!(read_interrupted(dir.path(), 1), Interrupted::default());
    }

    #[test]
    fn a_cut_off_turn_is_told_to_carry_on_and_says_why() {
        let upgraded = up("0.60.3", "0.60.4", 1_792_000_000_000, &[]).unwrap();
        let after_upgrade = continue_text(Some(&upgraded), 1_792_000_000_000);
        assert!(after_upgrade.starts_with("[AgentMux] AgentMux was upgraded from 0.60.3 to 0.60.4 at "), "{after_upgrade}");
        assert!(after_upgrade.contains(" while you were working; your last turn was cut off mid-tool. "), "{after_upgrade}");
        assert!(after_upgrade.contains("load it again before relying on the actions it lists."), "{after_upgrade}");
        assert!(after_upgrade.ends_with("Check the state of what you were doing and carry on."), "{after_upgrade}");
        assert!(!after_upgrade.contains("nothing is needed from you"), "{after_upgrade}");
        let after_restart = continue_text(None, 1_792_000_000_000);
        assert!(after_restart.starts_with("[AgentMux] AgentMux restarted at "), "{after_restart}");
        assert!(after_restart.ends_with("Check the state of what you were doing and carry on."), "{after_restart}");
        assert!(!after_restart.contains("upgraded"), "{after_restart}");
        // Read by the agent as written: one space between words.
        for text in [&after_upgrade, &after_restart, &note_text(&upgraded)] {
            assert!(!text.contains("  "), "{text}");
        }
    }

    #[tokio::test]
    async fn the_agent_a_pane_runs_is_read_from_its_block() {
        use crate::backend::obj::Block;
        let state = crate::server::tests::test_state();
        let mut agent = Block { oid: "with-agent".into(), ..Default::default() };
        agent.meta.insert("agentId".into(), serde_json::json!(" def-1 "));
        state.mstore.insert(&mut agent).unwrap();
        let mut shell = Block { oid: "shell".into(), ..Default::default() };
        state.mstore.insert(&mut shell).unwrap();
        assert_eq!(agent_id_of(&state.mstore, "with-agent"), "def-1");
        assert_eq!(agent_id_of(&state.mstore, "shell"), "");
        assert_eq!(agent_id_of(&state.mstore, "no-such-block"), "");
    }
}
