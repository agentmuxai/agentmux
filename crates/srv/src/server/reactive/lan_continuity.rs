// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LAN key continuity (`docs/specs/SPEC_LAN_KEY_CONTINUITY_2026_10_29.md`): a
//! pinned name whose agent now shows a different key moves to it with no
//! person in the loop, but only when the install that hosts the agent vouches
//! for the key with a certificate, and this machine already trusts that
//! install. Anything else stays refused.
//!
//! The sending half is [`issue_cert`]: the lookup answer for an agent carries
//! `lan_cert`, a [`WanKeyRecord`] (channel `lan`) signed by the install's own
//! key. The receiving half is [`move_if_vouched`], called when the name pin
//! and the key just shown disagree, and by the agent route of `pins.adopt`.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use agentmux_common::jekt_sign::WanKeyRecord;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::json;

use super::AppState;

/// The channel a LAN key's certificate names.
pub(crate) const CERT_CHANNEL: &str = "lan";
/// How far from now a certificate's time may be: it is signed when the answer
/// is given, so an old copy is a replay.
pub(crate) const CERT_MAX_AGE_SECS: i64 = 600;
/// An unvouched key is not looked into again for this long, so a peer that
/// keeps showing one cannot make this machine ask the directory every time.
const RECHECK_AFTER_SECS: i64 = 60;
/// At most this many unvouched keys are remembered at once; past it a key is simply looked into again.
const MAX_UNVOUCHED: usize = 1024;

/// What made the issuing install trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Path {
    /// This machine had already pinned the install for some agent.
    PinnedInstance,
    /// The signed-in account's install list shows it live and not revoked.
    AccountDirectory,
}

impl Path {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Path::PinnedInstance => "pinned-instance",
            Path::AccountDirectory => "account-directory",
        }
    }
}

/// The certificate for `agent`'s LAN key (`public_key_b64`), signed by this
/// install now. `None` when this install has no WAN identity or the key is not
/// a key.
pub(crate) fn issue_cert(state: &AppState, agent: &str, public_key_b64: &str) -> Option<WanKeyRecord> {
    let wan = state.mstore.wan_identity().or_else(crate::backend::storage::wan_identity::global)?;
    let instance = wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()).ok()?;
    let key: [u8; 32] = BASE64.decode(public_key_b64).ok()?.try_into().ok()?;
    WanKeyRecord::certify(&instance.private_key, agent, CERT_CHANNEL, &key, &instance.host_hint, agentmux_common::time::now_secs())
}

/// The pin and the key just shown disagree: move the pin if the hosting install vouches for the key,
/// else record why it stays refused. `true` means carry on verifying under the (moved) pin.
pub(super) async fn on_mismatch(state: &AppState, claimed: &str, observed_key: &[u8]) -> bool {
    let cert = state.lan_discovery.cached_lan_cert(claimed);
    match move_if_vouched(state, claimed, observed_key, cert.as_ref(), "automatic", false).await {
        Ok(_) => true,
        Err(why) => {
            super::lan_refusals::pin_mismatch_with(claimed, &BASE64.encode(observed_key), cert, Some(why));
            false
        }
    }
}

/// Put this install's certificate for `public_key_b64` into a lookup answer as `lan_cert`.
pub(super) fn add_cert(state: &AppState, agent: &str, public_key_b64: &str, answer: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(cert) = issue_cert(state, agent, public_key_b64) {
        answer.insert("lan_cert".to_string(), json!(cert));
    }
}

/// The certificate's own checks, none of which need the network: it is a
/// chain-valid certificate by some install, for this agent and channel `lan`
/// and exactly the key shown, issued close to now and after the pin it would
/// replace. Whether the install is trusted is [`instance_trust`]'s.
pub(crate) fn check_cert(cert: &WanKeyRecord, agent: &str, observed_key: &[u8], pin_since: i64, now: i64) -> Result<(), &'static str> {
    let Some(certified) = cert.check_chain() else { return Err("the certificate does not verify") };
    if cert.agent_id != agent.to_lowercase() {
        return Err("the certificate names another agent");
    }
    if cert.channel != CERT_CHANNEL {
        return Err("the certificate is not for a LAN key");
    }
    if certified.as_slice() != observed_key {
        return Err("the certificate is for another key");
    }
    if (now - cert.issued_at).abs() > CERT_MAX_AGE_SECS {
        return Err("the certificate is not fresh");
    }
    if cert.issued_at <= pin_since {
        return Err("the certificate is older than the pin it would replace");
    }
    Ok(())
}

/// Whether this machine trusts the install `instance_id`: pinned here for some
/// agent already, else live in the account's directory. An install this machine
/// knows to be revoked is trusted by neither way, and a pin does not outlive a
/// revocation the directory holds.
pub(crate) async fn instance_trust(state: &AppState, instance_id: &str) -> Result<Path, String> {
    use crate::muxbus::wan_verify::{instance_is_live_install, instance_is_revoked, Directory, GLOBAL_BUDGET};
    let wan = state.mstore.wan_identity().or_else(crate::backend::storage::wan_identity::global);
    if let Some(wan) = &wan {
        let known = wan.known_instance_get(instance_id).map_err(|e| format!("the known installs could not be read: {e}"))?;
        if known.is_some_and(|k| k.revoked_at.is_some()) {
            return Err("the install is revoked".to_string());
        }
    }
    let pinned = match &wan {
        Some(wan) => wan
            .agent_instance_pins_list()
            .map_err(|e| format!("the WAN pins could not be read: {e}"))?
            .iter()
            .any(|p| p.instance_id == instance_id),
        None => false,
    };
    let token = match crate::broker::get_global() {
        Some(scheduler) => crate::muxbus::cloud_subscriber::load_valid_token(&state.id_store, &scheduler).await,
        None => None,
    };
    let base_url = crate::muxbus::relay::rest_base_url();
    let dir = token.as_ref().map(|token| Directory { base_url: &base_url, http: &state.http_client, token, budget: &GLOBAL_BUDGET });
    if pinned {
        if let Some(dir) = &dir {
            if instance_is_revoked(dir, instance_id).await == Some(true) {
                if let Some(wan) = &wan {
                    remember_revoked(wan, instance_id, agentmux_common::time::now_secs());
                }
                return Err("the install is revoked".to_string());
            }
        }
        return Ok(Path::PinnedInstance);
    }
    let Some(dir) = dir else {
        return Err("the install is not pinned here and this machine is not signed in to ask the account's directory".to_string());
    };
    match instance_is_live_install(&dir, instance_id).await {
        Some(true) => Ok(Path::AccountDirectory),
        Some(false) => Err("the install is not pinned here and the account's directory does not list it as a live install".to_string()),
        None => Err("the install is not pinned here and the account's directory could not be read".to_string()),
    }
}

/// Keep a revocation learned from the directory, so it still stands when the directory cannot be
/// asked. The install may have no known-install row yet (a pin the user adopted never went through
/// WAN verification), and recording a check updates a row only, so make the row first.
fn remember_revoked(wan: &crate::backend::storage::wan_identity::WanIdentityStore, instance_id: &str, now: i64) {
    let _ = wan.known_instance_observe(instance_id, "?", now);
    let _ = wan.known_instance_record_revocation_check(instance_id, true, now);
}

static UNVOUCHED: LazyLock<Mutex<HashMap<(String, String), (i64, String)>>> = LazyLock::new(Default::default);

fn unvouched() -> std::sync::MutexGuard<'static, HashMap<(String, String), (i64, String)>> {
    UNVOUCHED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Move `agent`'s name pin to `observed_key` if `cert` vouches for it. The UID
/// pins beside the name pin go with the old key and are set again by the next
/// message that verifies under the new one (`lan_peer_pubkey_pin_replace`).
/// Returns what vouched, or why nothing did. `by` is who asked, for the audit
/// line: `"automatic"` for a message that arrived, an agent's id for the route.
///
/// `force` skips the wait after an unvouched answer (the agent route asks for it
/// on purpose).
pub(crate) async fn move_if_vouched(
    state: &AppState,
    agent: &str,
    observed_key: &[u8],
    cert: Option<&WanKeyRecord>,
    by: &str,
    force: bool,
) -> Result<Path, String> {
    let now = agentmux_common::time::now_secs();
    let observed_b64 = BASE64.encode(observed_key);
    let marker = (agent.to_lowercase(), observed_b64.clone());
    // Asked a moment ago and not vouched: say the same reason again, without asking again.
    if !force {
        if let Some((_, why)) = unvouched().get(&marker).filter(|(at, _)| now - at < RECHECK_AFTER_SECS) {
            return Err(why.clone());
        }
    }
    let result = vouch(state, agent, observed_key, cert, now).await;
    match &result {
        Ok(_) => {
            unvouched().remove(&marker);
        }
        Err(why) => {
            // Only the last minute matters, so a peer that shows a new key every time cannot make this grow.
            let mut seen = unvouched();
            seen.retain(|_, (at, _)| now - *at < RECHECK_AFTER_SECS);
            if seen.len() < MAX_UNVOUCHED {
                seen.insert(marker, (now, why.clone()));
            }
        }
    }
    let (path, cert, (old, old_since)) = result?;
    // Kept from the later of now and the certificate's own time: with the host's clock ahead of
    // ours, an older certificate of the same host (still inside the freshness window) is then
    // older than this pin too, and cannot move it back.
    let since = agentmux_common::time::now_secs().max(cert.issued_at);
    let moved = state
        .mstore
        .lan_peer_pubkey_pin_move(agent, (&old, old_since), &observed_b64, since)
        .map_err(|e| format!("the pin could not be moved: {e}"))?;
    if !moved {
        // The pin changed while the install was being asked about (a newer move, or the user's own
        // change): never write over it from an older check. If it already holds this key, done.
        return match state.mstore.lan_peer_pubkey_pin_since(agent) {
            Ok(Some((k, _))) if k == observed_b64 => {
                super::lan_refusals::clear(agent);
                Ok(path)
            }
            _ => Err("the pin changed while the key was being checked; it was not moved".to_string()),
        };
    }
    super::lan_refusals::clear(agent);
    let entry = json!({
        "action": "continuity",
        "kind": "lan",
        "agent": agent.to_lowercase(),
        "from": fingerprint(&old),
        "to": fingerprint(&observed_b64),
        "by": by,
        "vouched_by": { "instance": cert.instance_id, "host": cert.host_hint, "path": path.as_str(), "issued_at": cert.issued_at },
    });
    audit(&entry);
    tracing::warn!(agent_id = %agent, path = path.as_str(), instance = %cert.instance_id, by, "LAN key moved: the install that hosts the agent vouched for its new key");
    Ok(path)
}

async fn vouch(
    state: &AppState,
    agent: &str,
    observed_key: &[u8],
    cert: Option<&WanKeyRecord>,
    now: i64,
) -> Result<(Path, WanKeyRecord, (String, i64)), String> {
    let cert = cert.ok_or("the peer sent no certificate for the key")?;
    let pinned = state
        .mstore
        .lan_peer_pubkey_pin_since(agent)
        .map_err(|e| format!("the pin could not be read: {e}"))?
        .ok_or("the name is not pinned")?;
    check_cert(cert, agent, observed_key, pinned.1, now).map_err(str::to_string)?;
    let path = instance_trust(state, &cert.instance_id).await?;
    // The pin this check was made against: the move is conditional on it still being the pin.
    Ok((path, cert.clone(), pinned))
}

/// A short fingerprint of a base64 key: enough to tell two keys apart on a
/// screen, never the key. (Not a prefix of the key: it is a hash of it.)
pub(crate) fn fingerprint(key_b64: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(key_b64.as_bytes())[..5])
}

/// One line to `logs/pin-changes.log`, best effort. Fingerprints and labels
/// only: never a key.
pub(crate) fn audit(entry: &serde_json::Value) {
    tracing::warn!(change = %entry, "pin changed");
    // Tests keep the entries they cause instead of writing the real log.
    #[cfg(test)]
    AUDITED.lock().unwrap_or_else(|e| e.into_inner()).push(entry.clone());
    #[cfg(not(test))]
    if let Ok(root) = agentmux_common::data_paths::agentmux_root() {
        write_audit(&root.join("logs"), entry);
    }
}

/// Append one line for `entry` to `pin-changes.log` in `dir`.
pub(crate) fn write_audit(dir: &std::path::Path, entry: &serde_json::Value) {
    let line = json!({ "at": agentmux_common::time::now_secs(), "change": entry });
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("pin-changes.log")) {
        use std::io::Write;
        let _ = f.write_all(format!("{line}
").as_bytes());
    }
}

#[cfg(test)]
pub(crate) static AUDITED: LazyLock<Mutex<Vec<serde_json::Value>>> = LazyLock::new(Default::default);

#[cfg(test)]
mod tests;
