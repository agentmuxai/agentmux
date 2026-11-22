// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What a kind of collection says about merging (SPEC_USER_DATA_SYNC_LAYER §3.3): the layer calls its adapter
//! with the record's current header and the incoming one, and the adapter answers. The layer never decides what a
//! conflict means.

use super::record::Header;

/// An adapter's answer for an incoming record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// Take the incoming record as the record's current state.
    Applied,
    /// Keep what is here.
    KeptLocal,
    /// Neither wins: both rows stay in the log for the person (a Vault's secrets), and the record's current state
    /// is the side the adapter names, so every machine shows the same one as current.
    Conflict { current: Side },
}

/// Which side of a conflict is the record's current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Local,
    Incoming,
}

/// One kind of collection's merge rule.
pub trait Adapter {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge;
}

/// The newest version wins, for a record whose whole content is one value.
pub struct NewestWins;

impl Adapter for NewestWins {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge {
        match local {
            Some(l) if l.version >= incoming.version => Merge::KeptLocal,
            _ => Merge::Applied,
        }
    }
}
