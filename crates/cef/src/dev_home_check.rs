// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A dev host started by hand (not by the launcher, not a CEF helper process) with no isolated data
//! home would start its own srv on the real AgentMux home: refuse before anything is created.

/// `launcher_is_parent`: the launcher started this process, and it passed the same check itself.
pub(super) fn refuse_hand_started_dev_host(launcher_is_parent: bool) {
    if launcher_is_parent || std::env::args().any(|a| a.starts_with("--type=")) {
        return;
    }
    agentmux_common::dev_home_guard::refuse_or_continue_current_exe();
}
