// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether a build keeps its own work store: the Kanban queue and what hangs
//! off it (cards, events, Presence claims, coordination posts, the Jobs run
//! ledger, scheduled jobs).
//!
//! Those tables live in files every build on the host opens: the identity
//! store and the shared store. They are shared on purpose, so the agents of
//! every build the operator runs, `stable` or a `task package` one, work from
//! one queue. A `task dev` build is the exception: it is for testing, and
//! reading the operator's real queue meant a task it created, claimed or
//! completed was the operator's own. So a source-tree build gets a
//! `work-store.db` of its own by default.

/// `1` gives the build its own work store, any other value shares the host's.
pub const ISOLATED_WORK_ENV: &str = "AGENTMUX_ISOLATED_WORK";

/// Why the build does or doesn't have a work store of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedWorkReason {
    /// `AGENTMUX_ISOLATED_WORK=1`.
    ExplicitOptIn,
    /// `AGENTMUX_ISOLATED_WORK` is set to anything else: shared. A typo can't
    /// isolate by accident, the rule `AGENTMUX_ISOLATED_AUTH` follows.
    ExplicitOptOut,
    /// No override; this is a source-tree (`task dev`) build.
    DevBuild,
    /// No override; an installed or `task package` build: shared.
    SharedBuild,
}

impl IsolatedWorkReason {
    pub fn is_isolated(self) -> bool {
        matches!(self, Self::ExplicitOptIn | Self::DevBuild)
    }

    /// Short, log-friendly label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOptIn => "explicit opt-in",
            Self::ExplicitOptOut => "explicit opt-out",
            Self::DevBuild => "dev build — isolated",
            Self::SharedBuild => "default — shared",
        }
    }
}

/// The rule, with its inputs passed in: `opt` is `AGENTMUX_ISOLATED_WORK` (if
/// set) and `dev_build` whether this binary is a source-tree build.
pub fn isolated_work_reason_for(opt: Option<&str>, dev_build: bool) -> IsolatedWorkReason {
    match (opt, dev_build) {
        (Some("1"), _) => IsolatedWorkReason::ExplicitOptIn,
        (Some(_), _) => IsolatedWorkReason::ExplicitOptOut,
        (None, true) => IsolatedWorkReason::DevBuild,
        (None, false) => IsolatedWorkReason::SharedBuild,
    }
}

/// Reads the env, and asks the binary's own path whether it is a dev build
/// ([`crate::runtime_mode::is_dev_self`]), not `AGENTMUX_RUNTIME_MODE`: a
/// packaged build started from a pane inside a dev instance inherits that.
pub fn isolated_work_reason() -> IsolatedWorkReason {
    let opt = std::env::var(ISOLATED_WORK_ENV).ok();
    isolated_work_reason_for(opt.as_deref(), crate::runtime_mode::is_dev_self())
}

/// Whether this build keeps its own work store.
pub fn isolated_work_enabled() -> bool {
    isolated_work_reason().is_isolated()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_keeps_its_own() {
        assert_eq!(isolated_work_reason_for(None, true), IsolatedWorkReason::DevBuild);
        assert!(isolated_work_reason_for(None, true).is_isolated());
    }

    /// `stable` and the `local-*` builds agents run on share one queue.
    #[test]
    fn installed_and_packaged_builds_share_the_hosts() {
        assert!(!isolated_work_reason_for(None, false).is_isolated());
    }

    #[test]
    fn the_override_wins_either_way() {
        assert!(isolated_work_reason_for(Some("1"), false).is_isolated());
        assert!(!isolated_work_reason_for(Some("0"), true).is_isolated());
        assert!(!isolated_work_reason_for(Some(""), true).is_isolated());
        // A typo shares rather than isolates.
        assert!(!isolated_work_reason_for(Some("true"), false).is_isolated());
    }
}
