// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A registered block's names (`Handler::block_names`).

/// A block's names, typed (identity M2, spec §4.4.2).
///
/// `display` is the name the block was most recently registered under —
/// the Register-tail and the HTTP presence path supply it every time — and
/// is **replaced** on re-registration, so a retired name does not linger and
/// make a later agent ambiguous. `stable` is the `AGENTMUX_AGENT_ID` parked
/// at spawn (what the alias map used to hold) and is **kept** across display
/// changes: it is the value GitHub PR-body tags embed precisely because it
/// does not change on rename (`INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameBindings {
    pub display: String,
    pub stable: Option<String>,
}

impl NameBindings {
    /// Every lowercased key this block is bound under.
    pub(super) fn keys(&self) -> Vec<String> {
        let mut keys = vec![self.display.to_lowercase()];
        if let Some(s) = &self.stable {
            let k = s.to_lowercase();
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys
    }
}
