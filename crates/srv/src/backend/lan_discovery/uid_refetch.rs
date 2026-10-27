// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fresh ask of the LAN peers for the UID key beside a name key whose cached
//! answer is gone or held none (identity M4d-6, `lan_v2::claim_uid`).

use super::*;

impl LanDiscoveryController {
    /// One token from the fan-out limiter ([`Self::find_agent_lan_pubkey`]).
    pub(super) fn pubkey_lookup_allowed(&self) -> bool {
        self.pubkey_lookup_limiter.lock().map(|mut limiter| limiter.check()).unwrap_or(true)
    }

    /// The UID key `agent_id`'s peer gives beside `name_key`, asked for afresh
    /// because the cached answer is gone or held none. `None` when the peers
    /// give none, give another name key, or the limiter refuses the fan-out:
    /// the caller reads that as "not shown", never as "nothing to check".
    pub(crate) async fn refetch_lan_uid_key(&self, agent_id: &str, name_key: &[u8], http: &reqwest::Client) -> Option<(String, Vec<u8>)> {
        self.refetch_lan_uid_key_from(&self.get_instances(), agent_id, name_key, http).await
    }

    pub(super) async fn refetch_lan_uid_key_from(
        &self,
        peers: &[LanInstance],
        agent_id: &str,
        name_key: &[u8],
        http: &reqwest::Client,
    ) -> Option<(String, Vec<u8>)> {
        if !self.pubkey_lookup_allowed() {
            return None;
        }
        match self.pubkey_from_peers(peers, agent_id, http).await {
            LanPubkeyLookup::Found(key) if key == name_key => self.cached_lan_uid_key(agent_id),
            _ => None,
        }
    }
}
