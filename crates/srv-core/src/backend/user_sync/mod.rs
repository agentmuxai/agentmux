// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! One sync layer for a user's own data (docs/specs/SPEC_USER_DATA_SYNC_LAYER_2026_11_01.md): a change log of
//! signed, versioned records and the rules that make a collection's copies on several machines converge. This is
//! the core (build step S0): the clock, the record, the log and its state, and the adapter a kind of collection
//! implements. Nothing here sends anything; the transports come after it.

pub mod adapter;
pub mod chain_store;
pub mod hlc;
pub mod kanban;
pub mod keys;
pub mod lock;
pub mod log;
pub mod members;
pub mod ops;
pub mod record;
pub mod round;
pub mod seal;

#[cfg(test)]
mod kanban_tests;
#[cfg(test)]
mod keys_tests;
#[cfg(test)]
mod lock_tests;
#[cfg(test)]
mod members_tests;
#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod round_tests;
#[cfg(test)]
mod seal_tests;
#[cfg(test)]
mod tests;
