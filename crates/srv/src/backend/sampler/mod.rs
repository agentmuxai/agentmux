// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sampling the machine on demand: what to measure, for whom, and how often.
//! docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.5.

pub mod panes;
pub mod schedule;

pub use schedule::{Scheduler, SourceId, SourceSpec};

#[cfg(test)]
mod loop_tests;
