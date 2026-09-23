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
//! - **`spawn.no_token.<path>`** counts spawns, per launch path, whose
//!   process got no `AGENTMUX_AGENT_TOKEN`. M4b closes those paths.
//! - **`live.tokenless_or_unknown`** counts live registered agents (with a
//!   row, so they *should* carry a token) whose process is not known to carry
//!   one — including a process spawned before this srv started, which never
//!   passes a spawn site again. M4d and M5 wait for this to drain to zero,
//!   not for a counter that merely stops moving.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::backend::reactive::AgentRegistration;

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
/// `(tokenless_or_unknown, unidentified)`. An agent with a UID whose process
/// is not known to carry a token counts toward the first; an agent with no
/// row (a quick-launch pane — outside the identity system, §6.5.6) toward
/// the second.
pub(crate) fn live_gauges(registrations: &[AgentRegistration]) -> (usize, usize) {
    let mut tokenless_or_unknown = 0;
    let mut unidentified = 0;
    for reg in registrations {
        if reg.uid.is_none() {
            unidentified += 1;
        } else if carried_token(&reg.block_id) != Some(true) {
            tokenless_or_unknown += 1;
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
        assert_eq!(live_gauges(&regs), (2, 1));
        assert_eq!(carried_token(with), Some(true));
        assert_eq!(carried_token(unknown), None);
    }
}
