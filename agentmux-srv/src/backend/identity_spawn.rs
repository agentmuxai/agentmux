// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which spawned agent processes carry an identity token — identity M4a
//! (`docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.6).
//!
//! Two measurements, because request-side counters cannot gate anything: an
//! unattributed request from the UI and one from a tokenless agent look the
//! same (§6.5.1).
//!
//! - **`spawn.no_token.<path>`** counts process spawns, per controller, of
//!   an agent that has a row (a UID) but got no `AGENTMUX_AGENT_TOKEN` — the
//!   gap M4b closes, gated on these reaching zero. "Has a row" is decided by
//!   the store, not by the environment: the gap paths (`agent.send`, App
//!   Server) are exactly the ones whose environment carries no
//!   `AGENTMUX_AGENT_UID` although their block has a row (adversarial review
//!   of #3571). **`spawn.no_row.<path>`**
//!   counts spawns with no row at all (quick-launch panes, template-based
//!   continuations before M4b binds them), kept apart so the gate can be met.
//!   Recorded where a process is actually started, never where an
//!   environment is merely built: a turn delivered to a running process
//!   rebuilds the environment without spawning anything (review on #3571).
//! - **`live.tokenless_or_unknown`** counts live registered agents (with a
//!   row, so they *should* carry a token) whose process is not known to carry
//!   one — including a process spawned before this srv started, which never
//!   passes a spawn site again. M4d and M5 wait for this to drain to zero,
//!   not for a counter that merely stops moving.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::reactive::AgentRegistration;
use crate::backend::storage::store::Store;

static CARRIED_BY_BLOCK: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

fn carried_by_block() -> &'static Mutex<HashMap<String, bool>> {
    CARRIED_BY_BLOCK.get_or_init(Default::default)
}

/// Record that this srv spawned a process on `block_id`, and whether it
/// carried a token. `no_token_site` names the launch path's counter; `None`
/// for a path that is not an agent launch by itself (a terminal pane, where
/// an agent CLI is only one possibility — the live gauge covers that case).
pub(crate) fn record_spawn(
    block_id: &str,
    carried_token: bool,
    no_token_site: Option<&'static str>,
) {
    if block_id.is_empty() {
        return;
    }
    carried_by_block()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(block_id.to_string(), carried_token);
    if !carried_token {
        if let Some(site) = no_token_site {
            crate::backend::agent_resolve::record_uid_fallback(site);
        }
    }
}

/// Drop `block_id`'s record when its processes are released
/// (`blockcontroller::release_block_processes`), so the map is bounded by
/// open blocks, not by every block ever opened.
pub(crate) fn forget_block(block_id: &str) {
    carried_by_block()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(block_id);
}

/// The controller a process was spawned by — the `<path>` in the counters.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SpawnPath {
    Persistent,
    Subprocess,
    Acp,
    AppServer,
    Container,
}

/// This channel's object store, for deciding whether a spawned block has a
/// row. Attached once at boot (`bootstrap.rs`), beside the token index; the
/// subprocess controller has no store handle of its own.
static ROW_STORE: OnceLock<Arc<Store>> = OnceLock::new();

pub(crate) fn attach_row_store(store: Arc<Store>) {
    let _ = ROW_STORE.set(store);
}

/// Whether `block_id` shows an agent that has a row — the same block → row
/// resolution `build_persistent_spawn_env` uses, which also finds the row a
/// template-based continuation folded into. A synchronous store read; spawn
/// paths call it only when the environment carries no UID.
pub(crate) fn block_has_row(block_id: &str) -> bool {
    ROW_STORE
        .get()
        .is_some_and(|store| store_block_has_row(store, block_id))
}

/// [`block_has_row`] against a given store. A store fault answers "has a
/// row" (counted): a gate must never read clear because a read failed.
pub(crate) fn store_block_has_row(store: &Store, block_id: &str) -> bool {
    match store.instance_get_active_for_block(block_id) {
        Ok(Some(instance)) => !instance.id.trim().is_empty(),
        Ok(None) => false,
        Err(e) => {
            crate::backend::agent_resolve::record_uid_fallback("identity.row_lookup_error");
            tracing::warn!(block_id, error = %e, "identity: block row lookup failed — counted as row-backed");
            true
        }
    }
}

/// Record a process spawn on `block_id` from the environment it was
/// actually given. Call at the point the process is started.
pub(crate) fn record_process_spawn(block_id: &str, path: SpawnPath, env: &HashMap<String, String>) {
    record_process_spawn_with(block_id, path, env, block_has_row);
}

fn record_process_spawn_with(
    block_id: &str,
    path: SpawnPath,
    env: &HashMap<String, String>,
    has_row: impl FnOnce(&str) -> bool,
) {
    let has = |k: &str| env.get(k).is_some_and(|v| !v.trim().is_empty());
    let carried = has("AGENTMUX_AGENT_TOKEN");
    // A token implies a row (tokens are minted for rows); otherwise a carried
    // UID does, and failing both, the store decides.
    let row = carried || has("AGENTMUX_AGENT_UID") || has_row(block_id);
    let site = match (row, path) {
        (true, SpawnPath::Persistent) => "spawn.no_token.persistent",
        (true, SpawnPath::Subprocess) => "spawn.no_token.subprocess",
        (true, SpawnPath::Acp) => "spawn.no_token.acp",
        (true, SpawnPath::AppServer) => "spawn.no_token.app_server",
        (true, SpawnPath::Container) => "spawn.no_token.container",
        (false, SpawnPath::Persistent) => "spawn.no_row.persistent",
        (false, SpawnPath::Subprocess) => "spawn.no_row.subprocess",
        (false, SpawnPath::Acp) => "spawn.no_row.acp",
        (false, SpawnPath::AppServer) => "spawn.no_row.app_server",
        (false, SpawnPath::Container) => "spawn.no_row.container",
    };
    record_spawn(block_id, carried, Some(site));
}

/// Whether the process this srv last spawned on `block_id` carried a token;
/// `None` if this srv has not spawned one there.
pub(crate) fn carried_token(block_id: &str) -> Option<bool> {
    carried_by_block()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(block_id)
        .copied()
}

/// The two live gauges over `registrations`:
/// `(tokenless_or_unknown, unidentified)`. An agent with a row whose process
/// is not known to carry a token counts toward the first; an agent with no
/// row (a quick-launch pane — outside the identity system, §6.5.6) toward
/// the second. A registration without a UID is not taken as row-less on its
/// word: a block driven only through `agent.send` registers without one
/// although it has a row, so `has_row` asks the store. `is_live` filters out
/// registrations whose block no longer runs anything here, so a stale
/// registration cannot hold the gauge up.
pub(crate) fn live_gauges(
    registrations: &[AgentRegistration],
    is_live: impl Fn(&str) -> bool,
    has_row: impl Fn(&str) -> bool,
) -> (usize, usize) {
    let mut tokenless_or_unknown = 0;
    let mut unidentified = 0;
    for reg in registrations {
        if !is_live(&reg.block_id) {
            continue;
        }
        if carried_token(&reg.block_id) == Some(true) {
            continue;
        }
        if reg.uid.is_some() || has_row(&reg.block_id) {
            tokenless_or_unknown += 1;
        } else {
            unidentified += 1;
        }
    }
    (tokenless_or_unknown, unidentified)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(block: &str, uid: Option<&str>) -> AgentRegistration {
        let h = crate::backend::reactive::handler::ReactiveHandler::new();
        h.register_agent_full("AgentY", block, None, 0, None, uid, "test.m4a")
            .unwrap();
        h.list_agents().into_iter().next().unwrap()
    }

    /// A row-backed agent counts until this srv has seen it spawned with a
    /// token; one spawned before this srv started stays "unknown" — the case
    /// spawn counters alone would miss. A row-less pane is counted apart.
    #[test]
    fn live_gauges_count_row_backed_agents_not_known_to_carry_a_token() {
        let (with, without, unknown, rowless) = (
            "m4a-blk-with",
            "m4a-blk-without",
            "m4a-blk-unknown",
            "m4a-blk-rowless",
        );
        record_spawn(with, true, Some("test.m4a.no_token"));
        record_spawn(without, false, Some("test.m4a.no_token"));
        let regs = vec![
            reg(with, Some("uid-1")),
            reg(without, Some("uid-2")),
            reg(unknown, Some("uid-3")),
            reg(rowless, None),
        ];
        let no_row = |_: &str| false;
        assert_eq!(live_gauges(&regs, |_| true, no_row), (2, 1));
        // A registration whose block runs nothing here does not count.
        assert_eq!(live_gauges(&regs, |b| b != unknown, no_row), (1, 1));
        // A registration without a UID whose block has a row (an
        // `agent.send`-only block) is tokenless, not unidentified.
        assert_eq!(live_gauges(&regs, |_| true, |b| b == rowless), (3, 0));
        assert_eq!(carried_token(with), Some(true));
        assert_eq!(carried_token(unknown), None);
    }

    /// Classified from the environment the process actually got: a UID
    /// without a token is the gap; no UID at all is a row-less spawn.
    #[test]
    fn a_spawn_is_classified_from_its_actual_environment() {
        let env = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        record_process_spawn(
            "m4a-blk-env-tok",
            SpawnPath::Persistent,
            &env(&[("AGENTMUX_AGENT_UID", "u"), ("AGENTMUX_AGENT_TOKEN", "t")]),
        );
        assert_eq!(carried_token("m4a-blk-env-tok"), Some(true));
        let count = |site: &str| {
            crate::backend::agent_resolve::uid_fallback_counts()
                .into_iter()
                .find(|(s, _)| *s == site)
                .map_or(0, |(_, n)| n)
        };
        let (gap, rowless) = (count("spawn.no_token.acp"), count("spawn.no_row.acp"));
        record_process_spawn(
            "m4a-blk-env-gap",
            SpawnPath::Acp,
            &env(&[("AGENTMUX_AGENT_UID", "u")]),
        );
        record_process_spawn_with("m4a-blk-env-rowless", SpawnPath::Acp, &env(&[]), |_| false);
        assert!(count("spawn.no_token.acp") > gap);
        assert!(count("spawn.no_row.acp") > rowless);
        assert_eq!(carried_token("m4a-blk-env-gap"), Some(false));
    }

    /// The gap paths (`agent.send`, App Server) spawn with no
    /// `AGENTMUX_AGENT_UID` although their block has a row: the store
    /// decides, so they count as the gap, not as row-less (adversarial
    /// review of #3571 — otherwise the M4b gate passes trivially).
    #[test]
    fn a_row_backed_spawn_without_a_carried_uid_is_the_gap() {
        let count = |site: &str| {
            crate::backend::agent_resolve::uid_fallback_counts()
                .into_iter()
                .find(|(s, _)| *s == site)
                .map_or(0, |(_, n)| n)
        };
        let (gap, rowless) = (
            count("spawn.no_token.app_server"),
            count("spawn.no_row.app_server"),
        );
        let mut asked = None;
        record_process_spawn_with(
            "m4a-blk-appserver",
            SpawnPath::AppServer,
            &HashMap::new(),
            |b| {
                asked = Some(b.to_string());
                true
            },
        );
        assert_eq!(asked.as_deref(), Some("m4a-blk-appserver"));
        assert_eq!(count("spawn.no_token.app_server"), gap + 1);
        assert_eq!(count("spawn.no_row.app_server"), rowless);

        // A token alone implies a row; the store is not asked.
        let before = count("spawn.no_row.container");
        record_process_spawn_with(
            "m4a-blk-container",
            SpawnPath::Container,
            &[("AGENTMUX_AGENT_TOKEN".to_string(), "t".to_string())]
                .into_iter()
                .collect(),
            |_| panic!("not asked"),
        );
        assert_eq!(carried_token("m4a-blk-container"), Some(true));
        assert_eq!(count("spawn.no_row.container"), before);
    }

    /// Releasing a block's processes drops its record (ReAgent P2 on #3571:
    /// the map must not grow with every pane ever opened).
    #[test]
    fn releasing_a_block_forgets_its_record() {
        record_spawn("m4a-blk-closed", true, None);
        crate::backend::blockcontroller::release_block_processes("m4a-blk-closed");
        assert_eq!(carried_token("m4a-blk-closed"), None);
    }
}
