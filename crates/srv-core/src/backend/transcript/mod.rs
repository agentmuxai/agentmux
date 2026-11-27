// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent transcript writing.

pub mod append;
pub mod attribution;
pub mod compact;
pub mod counting;
pub mod damage;
pub mod framer;
pub mod images;
pub mod origin_ledger;
pub mod origin_resign;
pub mod origin_sig;
pub mod source;
pub mod tail;
pub mod turns;
pub mod withhold;
pub mod withhold_sources;
#[cfg(test)]
mod withhold_sources_tests;
