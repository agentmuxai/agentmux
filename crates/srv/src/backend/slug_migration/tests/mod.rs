// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The fork slug migration's tests (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md`
//! §6): the planner, each step, crash and resume, a full fixture home, and
//! the preconditions.

pub mod crash_tests;
pub mod fixture;
pub mod home_tests;
pub mod journal_tests;
pub mod permission_tests;
pub mod plan_tests;
pub mod precondition_tests;
pub mod steps_tests;

use super::run::{self, Options};

/// Apply with no crash.
fn apply(home: &super::Home) -> Result<crate::backend::rpc_types::ForkSlugReport, String> {
    run::apply(home, &Options::default())
}
