// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A site grant ("Always on this site" for an entry) as a sync record (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! section 6.6). Grants to an agent (`signin:<agent>`, a command's first-use grant) name an agent of one machine and
//! never sync: this module has no way to build a record from one.
//!
//! A grant holds no value, and two machines that make the same grant make the same record, so a grant has no keep-both:
//! the later version, a grant or its revoking, is the one every machine follows.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::sync_merge::Decision;
use super::sync_seal::Version;

/// A site grant as a sync record carries it, sealed: which entry and which site stay out of the clear header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantBody {
    /// The record the entry is written under (its link id), the same on every machine; not the id it has on one.
    pub entry_id: String,
    pub origin: String,
    pub granted_at: i64,
    /// When the entry was made. A grant only applies to the entry the person was asked about: a re-made entry of the same
    /// name has the same id and a new `created_at`, and does not inherit the site (the same check the local store makes).
    pub entry_created_at: i64,
}

/// Whether `id` has the shape of a grant's record id (`record_id`): `g-` and 32 lowercase hex characters.
pub fn is_record_id(id: &str) -> bool {
    id.strip_prefix("g-").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// A grant's record id: `g-` and 32 hex characters of a hash over the entry and the site, so the id says neither and is
/// the same on every machine for the same grant. Revoking is a tombstone of this id.
pub fn record_id(entry_id: &str, origin: &str) -> String {
    let mut h = Sha256::new();
    for part in [entry_id, origin] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let digest = h.finalize();
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("g-{hex}")
}

impl GrantBody {
    pub fn new(entry_id: &str, origin: &str, granted_at: i64, entry_created_at: i64) -> Self {
        Self { entry_id: entry_id.to_string(), origin: origin.to_string(), granted_at, entry_created_at }
    }

    pub fn record_id(&self) -> String {
        record_id(&self.entry_id, &self.origin)
    }

    /// Whether a machine whose copy of the entry was made at `local_created_at` (`None` if it has no such entry yet) may
    /// apply this grant. With no entry yet, the grant waits until the entry arrives.
    pub fn applies_to(&self, local_created_at: Option<i64>) -> bool {
        local_created_at == Some(self.entry_created_at)
    }
}

/// What to do with a grant or a revoke that arrives, given the current version this machine holds of that record: the
/// later version wins, and a version this machine has had changes nothing.
pub fn merge(current: Option<&Version>, had: &[Version], incoming: &Version) -> Decision {
    if had.contains(incoming) {
        return Decision::KeptLocal;
    }
    match current {
        Some(c) if incoming <= c => Decision::KeptLocal,
        _ => Decision::Applied,
    }
}

#[cfg(test)]
#[path = "sync_grant_tests.rs"]
mod tests;
