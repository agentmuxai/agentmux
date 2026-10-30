// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The key a LAN peer's agent signs with, as this machine trusts it: asked of the
//! peers, pinned on first use, and moved only when the hosting install vouches
//! (`lan_continuity`). Shared by the LAN jekt check (`verify_lan_signature`) and
//! the remote-operation check (`remote_ops::receive`), so the two can't judge a
//! sender's key differently.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use super::lan_continuity;
use super::AppState;
use crate::backend::lan_discovery::LanPubkeyLookup;

/// What became of looking up `claimed`'s signing key.
pub(crate) enum SignerKey {
    /// The key the peers show and this machine's pin accepts.
    Pinned(Vec<u8>),
    /// No peer has a key on file for that name: nothing to check against.
    Unknown,
    /// A key can't be had or can't be trusted (renamed, rate-limited, pin
    /// unreadable, or a different key nobody vouched for): counts as a failure.
    Refused,
}

/// Ask the LAN peers for `claimed`'s key and check it against this machine's pin.
pub(crate) async fn resolve(state: &AppState, claimed: &str) -> SignerKey {
    let observed_key = match state.lan_discovery.find_agent_lan_pubkey_asking_again(claimed, &state.http_client).await {
        LanPubkeyLookup::Found(key) => key,
        // Genuinely unknown sender: nothing to check against, not a red flag on its own.
        LanPubkeyLookup::NotFound => return SignerKey::Unknown,
        LanPubkeyLookup::Renamed(current) => {
            tracing::warn!(agent_id = %claimed, current = %current, "LAN jekt signed as a name its agent was renamed from, which never signs: counted as failed");
            return SignerKey::Refused;
        }
        // The lookup was skipped (rate-limited), not genuinely absent: a signature WAS
        // presented, so "we didn't check" must not collapse into "nothing to check", or an
        // attacker could exhaust the limiter to slip a forged identity through unverified.
        LanPubkeyLookup::RateLimited => {
            tracing::warn!(agent_id = %claimed, "LAN pubkey lookup was rate-limited while verifying a signature: treating as failed rather than passing it through");
            return SignerKey::Refused;
        }
    };
    // Trust-on-first-use pin (mDNS discovery is unauthenticated): the first key seen for a
    // sender is pinned, and a LATER, DIFFERENT key is a red flag, never a silent update.
    let observed_key_b64 = BASE64.encode(&observed_key);
    let Ok(pinned_key_b64) = state.mstore.lan_peer_pubkey_pin_get_or_set(claimed, &observed_key_b64) else {
        tracing::warn!(agent_id = %claimed, "LAN pubkey pin store unreadable: signature counted as failed");
        return SignerKey::Refused; // fail closed
    };
    // A different key moves the pin only if the hosting install vouches for it.
    if pinned_key_b64 != observed_key_b64 && !lan_continuity::on_mismatch(state, claimed, &observed_key).await {
        return SignerKey::Refused;
    }
    SignerKey::Pinned(observed_key)
}
