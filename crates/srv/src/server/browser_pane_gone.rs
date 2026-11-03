// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the browser state forgets about a pane when its block is deleted
//! (`backend::wcore::set_block_gone_hook`).

/// Installs the hook; `browser_host_sync::spawn` calls it once at startup.
pub(crate) fn install() {
    crate::backend::wcore::set_block_gone_hook(pane_gone);
}

fn pane_gone(block_id: &str) {
    // An agent's ownership of a browser pane ends with the pane
    // (SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §3).
    super::browser_owner::forget(block_id);
    // So does anything it was waiting on the user for: an agent mid
    // hand-off or approval gets Cancelled now, not after the timeout.
    super::browser_attention::cancel_for(block_id);
    // And its share of a site limit (its chain's other panes keep theirs).
    super::browser_allowlist::drop_pane(block_id);
    // Its action log and pause go with it (Take over keeps the log).
    super::browser_actions::drop_pane(block_id);
    // The host drops an Incognito jar once no tab uses it: tell it now.
    super::browser_owner::changed().notify_one();
}
