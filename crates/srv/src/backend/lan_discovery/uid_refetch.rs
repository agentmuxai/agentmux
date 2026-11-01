// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fresh ask of the LAN peers for the UID key beside a name key whose cached
//! answer is gone or held none (identity M4d-6, `lan_v2::claim_uid`), and for a
//! name key whose cached answer was a miss.

use super::*;

/// A cached miss younger than this was made by the lookup that just ran: asking
/// again would repeat it.
const MISS_RECHECK_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

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
        self.refetch_lan_uid_key_from(&self.get_all_instances(), agent_id, name_key, http).await
    }

    /// [`Self::find_agent_lan_pubkey`], and when it says `NotFound`, one more ask
    /// if that answer came from the cache.
    pub(crate) async fn find_agent_lan_pubkey_asking_again(&self, agent_id: &str, http: &reqwest::Client) -> LanPubkeyLookup {
        match self.find_agent_lan_pubkey(agent_id, http).await {
            LanPubkeyLookup::NotFound => self.find_agent_lan_pubkey_again(agent_id, http).await,
            other => other,
        }
    }

    /// `agent_id`'s name key after [`Self::find_agent_lan_pubkey`] said
    /// `NotFound`, asked for once more when that answer came from the cache: a
    /// miss cached a moment ago (a peer that had not minted the key yet, or had
    /// not announced itself) must not leave a signed jekt unverified for the rest
    /// of its life. A miss the lookup itself just made is not repeated. Anything
    /// but a key reads as `NotFound` again (the limiter's refusal included): the
    /// caller leaves the jekt unverified, as it would have. A key found here goes
    /// through the pin check like any other.
    pub(crate) async fn find_agent_lan_pubkey_again(&self, agent_id: &str, http: &reqwest::Client) -> LanPubkeyLookup {
        self.find_agent_lan_pubkey_again_from(&self.peers_to_ask(), agent_id, http).await
    }

    fn peers_to_ask(&self) -> Vec<LanInstance> {
        #[cfg(test)]
        if let Some(seeded) = Some(self.peers_for_test.lock().unwrap().clone()).filter(|p| !p.is_empty()) {
            return seeded;
        }
        self.get_all_instances()
    }

    pub(super) async fn find_agent_lan_pubkey_again_from(&self, peers: &[LanInstance], agent_id: &str, http: &reqwest::Client) -> LanPubkeyLookup {
        let old_miss = self.pubkey_cache.read().ok().is_some_and(|cache| {
            cache.get(agent_id).is_some_and(|e| {
                // A miss past its TTL was not served from the cache: the lookup that
                // just ran asked the peers itself.
                let left = e.expires.saturating_duration_since(std::time::Instant::now());
                let age = std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS).saturating_sub(left);
                e.public_key.is_none() && !left.is_zero() && age >= MISS_RECHECK_AFTER
            })
        });
        if !old_miss || !self.pubkey_lookup_allowed() {
            return LanPubkeyLookup::NotFound;
        }
        match self.pubkey_from_peers(peers, agent_id, http).await {
            found @ LanPubkeyLookup::Found(_) => found,
            _ => LanPubkeyLookup::NotFound,
        }
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
        // The fan-out caches whatever it finds, a miss included. An ask that
        // does not confirm this name key with a UID key must leave the cache as
        // it was, or a miss would stand in for the key that just verified.
        let before = self.pubkey_cache.read().ok().and_then(|cache| {
            cache.get(agent_id).map(|e| LanPubkeyCacheEntry { public_key: e.public_key.clone(), uid_key: e.uid_key.clone(), cert: e.cert.clone(), expires: e.expires })
        });
        let shown = match self.pubkey_from_peers(peers, agent_id, http).await {
            LanPubkeyLookup::Found(key) if key == name_key => self.cached_lan_uid_key(agent_id),
            _ => None,
        };
        if shown.is_none() {
            if let Ok(mut cache) = self.pubkey_cache.write() {
                match before {
                    Some(entry) => cache.insert(agent_id.to_string(), entry),
                    None => cache.remove(agent_id),
                };
            }
        }
        shown
    }

    /// Give the cached answer for `agent_id` a certificate, as a host's lookup answer carries.
    #[cfg(test)]
    pub fn seed_lan_cert_for_test(&self, agent_id: &str, cert: agentmux_common::jekt_sign::WanKeyRecord) {
        if let Some(entry) = self.pubkey_cache.write().unwrap().get_mut(agent_id) {
            entry.cert = Some(cert);
        }
    }

    /// The certificate the peer that answered [`Self::find_agent_lan_pubkey`]
    /// for `agent_id` gave beside its name key, while that answer is cached.
    pub fn cached_lan_cert(&self, agent_id: &str) -> Option<agentmux_common::jekt_sign::WanKeyRecord> {
        let cache = self.pubkey_cache.read().ok()?;
        cache.get(agent_id).filter(|e| e.expires > std::time::Instant::now())?.cert.clone()
    }
}

#[cfg(test)]
impl LanDiscoveryController {
    /// A peer at `addr` ("host:port"), listed as discovery would list it now.
    pub(crate) fn seed_peer_at_for_test(&self, addr: &str) {
        let (host, port) = addr.rsplit_once(':').unwrap();
        let instance = LanInstance {
            instance_id: format!("peer-{port}"),
            hostname: "test".into(),
            version: "0".into(),
            channel: String::new(),
            os: String::new(),
            address: host.to_string(),
            port: port.parse().unwrap(),
            auth_key: "k".into(),
            agents: vec![],
            agent_status: Default::default(),
            agent_lines: Default::default(),
            agent_providers: Default::default(),
            agent_accounts: Default::default(),
            agent_seats: Default::default(),
            first_seen: 0,
            last_seen: agentmux_common::time::now_secs_u64(),
            last_polled_ok: 0,
            other_ttl_secs: 4500,
        };
        self.peers_for_test.lock().unwrap().push(instance);
    }

    /// A miss for `agent_id` that was cached `age_secs` ago.
    pub(crate) fn seed_lan_pubkey_miss_for_test(&self, agent_id: &str, age_secs: u64) {
        let life = std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS);
        let left = life.saturating_sub(std::time::Duration::from_secs(age_secs));
        self.pubkey_cache.write().unwrap().insert(
            agent_id.to_string(),
            LanPubkeyCacheEntry { public_key: None, uid_key: None, cert: None, expires: std::time::Instant::now() + left },
        );
    }

    /// [`Self::seed_lan_pubkey_for_test`], with the certificate the answer carried.
    #[cfg(test)]
    pub fn seed_lan_pubkey_with_cert_for_test(&self, agent_id: &str, public_key: Vec<u8>, cert: Option<agentmux_common::jekt_sign::WanKeyRecord>) {
        self.pubkey_cache.write().unwrap().insert(
            agent_id.to_string(),
            LanPubkeyCacheEntry {
                public_key: Some(public_key),
                uid_key: None,
                cert,
                expires: std::time::Instant::now() + std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS),
            },
        );
    }
}
