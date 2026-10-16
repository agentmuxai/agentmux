// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files sent with a jekt over the cloud relay
//! (`docs/specs/SPEC_JEKT_FILE_TRANSFER_2026_10_05.md` §4.4).
//!
//! Sending: each file is encrypted (AES-256-GCM, a fresh key per file and
//! send), uploaded to a URL the relay issues (`POST /reactive/blob`), and
//! named in a cloud manifest with its key and blob id. The key is sealed to
//! the install that runs the target when its lease offers a key
//! (`jekt_seal`, spec §7), and travels as plain base64 to an older one. The
//! jekt's msgid is rebound to that manifest, so srv's WAN signature covers
//! keys and blob ids too. Receiving: after the claim, each blob is downloaded
//! through a URL the relay issues to the target alone
//! (`GET /reactive/blob/:id`), decrypted and checked against its hash, then
//! stored like any received file.

use std::time::Duration;

use agentmux_common::jekt_files::{self, CloudFile, JektFile};
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

use super::jekt_seal::{self, Recipient};
use crate::backend::attachments;
use crate::backend::reactive::types::{FilesDelivery, InjectionRequest};

/// An upload or download may take this long; the relay's URLs live 300 s.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(240);
/// Bytes a blob adds to its file: the nonce in front, the GCM tag behind.
const OVERHEAD: usize = NONCE_LEN + 16;
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Encrypt `plain` under a fresh key: the blob (nonce, ciphertext, tag) and
/// the key.
fn seal(mut plain: Vec<u8>) -> Option<(Vec<u8>, [u8; 32])> {
    let rng = SystemRandom::new();
    let mut key = [0u8; 32];
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill(&mut key).ok()?;
    rng.fill(&mut nonce).ok()?;
    let sealing = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).ok()?);
    sealing
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut plain)
        .ok()?;
    let mut blob = Vec::with_capacity(NONCE_LEN + plain.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&plain);
    Some((blob, key))
}

/// Decrypt a blob `seal` made; `None` if the key, nonce or tag don't match.
fn open(blob: &[u8], key: &[u8; 32]) -> Option<Vec<u8>> {
    if blob.len() < OVERHEAD {
        return None;
    }
    let (nonce, sealed) = blob.split_at(NONCE_LEN);
    let opening = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut buf = sealed.to_vec();
    let plain = opening
        .open_in_place(Nonce::try_assume_unique_for_key(nonce).ok()?, Aad::empty(), &mut buf)
        .ok()?;
    Some(plain.to_vec())
}

/// Send `req` and its files through the relay: `Ok(Some(answer))` once the
/// relay queued it, `Ok(None)` when this instance has no relay or the relay
/// didn't take it (the caller holds or refuses), `Err` with why the files
/// can't go that way at all.
pub(crate) async fn relay(state: &super::AppState, req: &InjectionRequest, caller_uid: &str) -> Result<Option<serde_json::Value>, String> {
    let files = req.files.as_deref().unwrap_or_default();
    let Some(source) = req.source_agent.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if req.delivery_tier.as_deref() == Some("wan") {
        return Ok(None);
    }
    jekt_files::check_tier_limits(files, "wan").map_err(|e| format!("files refused: {e}"))?;
    let Some(credential) = crate::muxbus::relay::relay_token(source, &state.id_store, &state.http_client).await else {
        return Ok(None);
    };
    let base = crate::muxbus::relay::rest_base_url();
    // Only to a target running now on an install that says it can fetch
    // them: the relay doesn't bind a jekt to the install that will claim it,
    // so a target with no live holder could be claimed later by an older
    // install, which would deliver the text without its files (spec §3.4).
    let holder = crate::muxbus::wan_lease::holder(&base, &req.target_agent, source, &credential.token, &state.http_client).await;
    let refused = |why: &str| format!("files refused: {} {why}; nothing was sent", req.target_agent);
    let held_by = takes_cloud_files(&holder).map_err(refused)?;
    let seal_to = match &held_by.seal {
        Some(offer) => Some(recipient(state, &base, &req.target_agent, &held_by.channel, offer).await.map_err(|why| refused(&why))?),
        None => None,
    };
    let cloud_req = upload(&base, &state.http_client, &credential.token, source, req, seal_to.as_ref()).await?;
    Ok(super::reactive::try_cloud_relay(state, &cloud_req, caller_uid).await)
}

/// The key to seal `target`'s files to, from its lease's offer, checked
/// against the agent's record in the account's key directory. `Err` says
/// why not, after the target's name: a holder that offers a key gets sealed
/// keys or nothing, never plain ones.
async fn recipient(
    state: &super::AppState,
    base: &str,
    target: &str,
    channel: &str,
    offer: &jekt_seal::SealOffer,
) -> Result<Recipient, String> {
    const NO_IDENTITY: &str = "can't be sent sealed files: this AgentMux has no WAN identity";
    const SIGNED_OUT: &str = "can't be sent sealed files: this AgentMux isn't signed in to the cloud";
    let wan = crate::backend::storage::wan_identity::global().ok_or(NO_IDENTITY)?;
    let own = wan
        .instance_ensure(&crate::backend::reactive::registry::local_host_label())
        .map_err(|_| NO_IDENTITY)?;
    // The directory answers the account's own token, not an agent's.
    let scheduler = crate::broker::get_global().ok_or(SIGNED_OUT)?;
    let token = crate::muxbus::cloud_subscriber::load_valid_token(&state.id_store, &scheduler)
        .await
        .ok_or(SIGNED_OUT)?;
    let dir = crate::muxbus::wan_verify::Directory {
        base_url: base,
        http: &state.http_client,
        token: &token,
        budget: &crate::muxbus::wan_verify::GLOBAL_BUDGET,
    };
    let now = agentmux_common::time::now_secs();
    let record =
        crate::muxbus::wan_verify::recipient_record(&wan, &own.instance_id, &dir, target, &offer.instance, channel, &offer.key_fp, now)
            .await?;
    jekt_seal::check_offer(offer, &record, target).map_err(str::to_string)
}

/// Whether the target's lease holder can fetch files from the relay: only a
/// live holder that says so. `Err` says why not, after the target's name.
fn takes_cloud_files(holder: &crate::muxbus::wan_lease::Holder) -> Result<&crate::muxbus::wan_lease::HeldBy, &'static str> {
    use crate::muxbus::wan_lease::Holder;
    match holder {
        Holder::Other(h) if h.features.iter().any(|f| f == jekt_files::CLOUD_FEATURE) => Ok(h),
        Holder::Other(_) => Err("runs on an AgentMux too old to receive files through the cloud relay"),
        Holder::Free | Holder::Yours => Err("isn't running on another computer right now, and files through the cloud relay go only to a running agent"),
        Holder::Unknown | Holder::Unsupported | Holder::Unreachable => Err("can't be confirmed to receive files: the cloud relay didn't say who runs it"),
    }
}

/// Encrypt and upload each file, sealing its key to `seal_to` when there is
/// one; the request to relay, its files named by the cloud manifest and its
/// msgid bound to it.
async fn upload(
    base: &str,
    http: &reqwest::Client,
    token: &str,
    source: &str,
    req: &InjectionRequest,
    seal_to: Option<&Recipient>,
) -> Result<InjectionRequest, String> {
    let svc = attachments::get().ok_or("files refused: the attachment store isn't available")?;
    let mut cloud = Vec::new();
    for f in req.files.as_deref().unwrap_or_default() {
        let path = svc.store().find_blob(&f.sha256).map(|(p, _)| p);
        let plain = match path {
            Some(p) => tokio::fs::read(p).await.ok(),
            None => None,
        }
        .filter(|b| b.len() as u64 == f.size)
        .ok_or_else(|| format!("files refused: {} is no longer in this AgentMux's store", f.name))?;
        let (blob, key) = tokio::task::spawn_blocking(move || seal(plain))
            .await
            .ok()
            .flatten()
            .ok_or("files refused: couldn't encrypt a file")?;
        let issued: serde_json::Value = http
            .post(format!("{}/reactive/blob", base.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {token}"))
            .header("X-Agent-ID", source)
            .json(&serde_json::json!({ "target_agent": req.target_agent, "size": blob.len() }))
            .send()
            .await
            .map_err(|e| format!("files refused: the cloud relay is unreachable: {e}"))?
            .error_for_status()
            .map_err(|e| format!("files refused: the cloud relay refused a file: {e}"))?
            .json()
            .await
            .map_err(|e| format!("files refused: the cloud relay's answer is unreadable: {e}"))?;
        let (Some(blob_id), Some(url)) = (issued["blob_id"].as_str(), issued["upload_url"].as_str()) else {
            return Err("files refused: the cloud relay can't take files".to_string());
        };
        http.put(url)
            .timeout(TRANSFER_TIMEOUT)
            .body(blob)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("files refused: uploading {} failed: {e}", f.name))?;
        let key = match seal_to {
            Some(to) => jekt_seal::wrap(&key, to, &f.sha256, blob_id).ok_or("files refused: couldn't seal a file's key")?,
            None => B64.encode(key),
        };
        cloud.push(CloudFile { file: f.clone(), key, blob_id: blob_id.to_string() });
    }
    let mut out = req.clone();
    out.request_id = Some(jekt_files::bind_cloud_msgid(req.request_id.as_deref().unwrap_or_default(), &cloud));
    out.cloud_files = Some(cloud);
    Ok(out)
}

/// On the receiving side, after the claim: check a cloud manifest, then
/// fetch, decrypt and store each file, setting `req.files` for
/// `jekt_files::prepare` to list. `Err` when the relay couldn't be reached,
/// so the caller releases the claim for a retry; a file that is gone,
/// tampered with or undecryptable is left out and listed as missing.
pub(crate) async fn fetch(
    base: &str,
    http: &reqwest::Client,
    token: &str,
    agent_id: &str,
    req: &mut InjectionRequest,
    files: Vec<CloudFile>,
    wan_msg_id: Option<&str>,
) -> Result<(), String> {
    let note = |req: &mut InjectionRequest, text: &str| {
        req.files_delivery = Some(FilesDelivery { delivered: 0, text: text.to_string(), inline: Vec::new(), inline_charge: None });
    };
    let plain: Vec<JektFile> = files.iter().map(|f| f.file.clone()).collect();
    if jekt_files::validate_cloud(&files).is_err() || jekt_files::check_tier_limits(&plain, "wan").is_err() {
        note(req, "The files sent with this message were malformed or over the cloud's limits, so none were delivered.");
        return Ok(());
    }
    // A verified sender's signature covers the manifest through its msgid.
    if req.wan_verified == Some(true) && !wan_msg_id.is_some_and(|m| jekt_files::cloud_msgid_binds(m, &files)) {
        note(req, "The files sent with this message don't match its signature, so none were delivered.");
        return Ok(());
    }
    req.files = Some(plain);
    if super::jekt_files::withheld_reason(req).is_some() {
        return Ok(()); // `prepare` says they were withheld
    }
    let Some(svc) = attachments::get() else {
        return Ok(());
    };
    let base = base.trim_end_matches('/');
    for f in &files {
        if svc.store().find_blob(&f.file.sha256).is_some() {
            continue; // already here: a retry, or the same file sent before
        }
        let key = match file_key(f, agent_id) {
            Ok(key) => key,
            Err(why) => {
                tracing::warn!(file = %f.file.name, why, "jekt cloud files: key didn't open");
                continue;
            }
        };
        let issued = http
            .get(format!("{base}/reactive/blob/{}", f.blob_id))
            .header("Authorization", format!("Bearer {token}"))
            .header("X-Agent-ID", agent_id)
            .header("X-Agent-Instance", crate::muxbus::wan_lease::instance_id())
            .send()
            .await
            .map_err(|e| format!("blob URL request failed: {e}"))?;
        let status = issued.status();
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
            tracing::warn!(file = %f.file.name, %status, "jekt cloud files: blob not available");
            continue;
        }
        if !status.is_success() {
            return Err(format!("blob URL request answered {status}"));
        }
        let url = issued.json::<serde_json::Value>().await.ok().and_then(|v| v["url"].as_str().map(str::to_string));
        let Some(url) = url else { return Err("blob URL answer unreadable".to_string()) };
        let mut resp = http
            .get(&url)
            .timeout(TRANSFER_TIMEOUT)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("blob download failed: {e}"))?;
        // Read no more than the file and its encryption can be.
        let max = f.file.size as usize + OVERHEAD;
        let mut blob = Vec::with_capacity(max.min(1 << 20));
        let mut oversized = false;
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("blob download cut short: {e}"))? {
            if blob.len() + chunk.len() > max {
                oversized = true;
                break;
            }
            blob.extend_from_slice(&chunk);
        }
        if oversized {
            tracing::warn!(file = %f.file.name, "jekt cloud files: blob larger than its file");
            continue;
        }
        let plain = tokio::task::spawn_blocking(move || open(&blob, &key)).await.ok().flatten();
        let intact = plain.filter(|p| p.len() as u64 == f.file.size && {
            use sha2::Digest;
            hex::encode(sha2::Sha256::digest(p)) == f.file.sha256
        });
        let Some(plain) = intact else {
            tracing::warn!(file = %f.file.name, "jekt cloud files: blob didn't decrypt to its file");
            continue;
        };
        let tmp = svc.store().new_incoming_path();
        if tokio::fs::write(&tmp, &plain).await.is_err()
            || svc.commit_upload(tmp.clone(), f.file.sha256.clone(), f.file.name.clone()).await.is_err()
        {
            let _ = tokio::fs::remove_file(&tmp).await;
            tracing::warn!(file = %f.file.name, "jekt cloud files: received file not stored");
        }
    }
    Ok(())
}

/// The key `f`'s blob opens with, using `agent_id`'s WAN key on this install
/// for a sealed one. `Err` says why not.
fn file_key(f: &CloudFile, agent_id: &str) -> Result<[u8; 32], &'static str> {
    let wan_key = || {
        let wan = crate::backend::storage::wan_identity::global().ok_or("no WAN identity here")?;
        let key = wan.agent_key_load(agent_id).ok().flatten().ok_or("no WAN key for this agent here")?;
        let public = key.public_key_bytes().ok_or("malformed WAN key")?;
        let seed: [u8; 32] = agentmux_common::jekt_sign::decode_key(&key.private_key)
            .and_then(|k| k.try_into().ok())
            .ok_or("malformed WAN key")?;
        Ok((public, seed))
    };
    open_key(f, wan_key)
}

/// A plain key decoded, or a sealed one opened with the WAN keypair
/// `wan_key` gives (asked for only then).
fn open_key(f: &CloudFile, wan_key: impl FnOnce() -> Result<([u8; 32], [u8; 32]), &'static str>) -> Result<[u8; 32], &'static str> {
    if !jekt_files::is_sealed_key(&f.key) {
        return B64.decode(&f.key).ok().and_then(|k| k.try_into().ok()).ok_or("malformed");
    }
    let (public, seed) = wan_key()?;
    jekt_seal::unwrap(&f.key, &public, &seed, &f.file.sha256, &f.blob_id).map_err(|e| match e {
        // The lease moved between the send and the claim.
        jekt_seal::UnwrapError::OtherKey => "sealed to another install's copy of this agent",
        jekt_seal::UnwrapError::Failed => "sealed key didn't open",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_blob_opens_only_with_its_key_and_untouched() {
        let (blob, key) = seal(b"the file".to_vec()).unwrap();
        assert_eq!(blob.len(), 8 + OVERHEAD);
        assert_eq!(open(&blob, &key).as_deref(), Some(&b"the file"[..]));
        let (_, other) = seal(b"x".to_vec()).unwrap();
        assert_eq!(open(&blob, &other), None, "another key");
        let mut tampered = blob.clone();
        tampered[NONCE_LEN] ^= 1;
        assert_eq!(open(&tampered, &key), None, "the tag catches a changed byte");
        assert_eq!(open(&blob[..4], &key), None);
    }

    #[test]
    fn each_send_uses_a_fresh_key_and_nonce() {
        let (a, ka) = seal(b"same".to_vec()).unwrap();
        let (b, kb) = seal(b"same".to_vec()).unwrap();
        assert_ne!(ka, kb);
        assert_ne!(a, b);
    }

    fn manifest() -> Vec<CloudFile> {
        vec![CloudFile {
            file: JektFile { sha256: "ab".repeat(32), name: "shot.png".into(), size: 3 },
            key: "A".repeat(43) + "=",
            blob_id: "blob-0123abcd-0000-4000-8000-0123456789ab".into(),
        }]
    }

    #[test]
    fn only_a_live_holder_that_says_so_takes_cloud_files() {
        use crate::muxbus::wan_lease::Holder;
        use crate::muxbus::wan_lease::HeldBy;
        let capable = HeldBy { desc: "computer x".into(), features: vec![jekt_files::CLOUD_FEATURE.into()], ..Default::default() };
        assert!(takes_cloud_files(&Holder::Other(capable)).is_ok());
        let old = HeldBy { desc: "computer x".into(), ..Default::default() };
        assert!(takes_cloud_files(&Holder::Other(old)).unwrap_err().contains("too old"));
        // No live capable holder: an older install could claim it later.
        for unknown in [Holder::Free, Holder::Yours, Holder::Unknown, Holder::Unsupported, Holder::Unreachable] {
            assert!(takes_cloud_files(&unknown).is_err(), "{unknown:?}");
        }
    }

    #[test]
    fn a_key_sealed_by_the_sender_opens_on_the_receiver_and_its_blob_with_it() {
        use agentmux_common::jekt_sign::{generate_wan_keypair, WanKeyRecord};
        let (public, seed) = generate_wan_keypair([4; 32]);
        let record = WanKeyRecord::certify(&[9; 32], "jf-mac", "stable", &public, "starpower", 1).unwrap();
        let offer = jekt_seal::offer(&record.instance_id, "jf-mac", &public, &seed).unwrap();
        let to = jekt_seal::check_offer(&offer, &record, "jf-mac").unwrap();

        let (blob, key) = seal(b"the screenshot".to_vec()).unwrap();
        let mut f = manifest().remove(0);
        f.key = jekt_seal::wrap(&key, &to, &f.file.sha256, &f.blob_id).unwrap();
        assert!(jekt_files::validate_cloud(std::slice::from_ref(&f)).is_ok(), "{}", f.key);

        let opened = open_key(&f, || Ok((public, seed))).unwrap();
        assert_eq!(open(&blob, &opened).as_deref(), Some(&b"the screenshot"[..]));
        // Another install's copy of the agent, or none here.
        let other = generate_wan_keypair([5; 32]);
        assert!(open_key(&f, || Ok(other)).unwrap_err().contains("another install"));
        assert_eq!(open_key(&f, || Err("no WAN key for this agent here")), Err("no WAN key for this agent here"));
        // A plain key never asks for the WAN key.
        let plain = CloudFile { key: B64.encode(key), ..f.clone() };
        assert_eq!(open_key(&plain, || panic!("not asked")), Ok(key));
    }

    #[tokio::test]
    async fn a_verified_jekt_whose_msgid_does_not_bind_its_manifest_gets_no_files() {
        let client = reqwest::Client::new();
        let mut req = InjectionRequest { wan_verified: Some(true), delivery_tier: Some("wan".into()), ..Default::default() };
        fetch("http://127.0.0.1:1", &client, "t", "agenty", &mut req, manifest(), Some("m-1")).await.unwrap();
        assert!(req.files.is_none());
        assert!(req.files_delivery.unwrap().text.contains("don't match its signature"));
    }

    #[tokio::test]
    async fn a_manifest_over_the_cloud_limits_is_refused_before_fetching() {
        let client = reqwest::Client::new();
        let mut files = manifest();
        files[0].file.size = 30 * 1024 * 1024;
        let mut req = InjectionRequest { delivery_tier: Some("wan".into()), ..Default::default() };
        // Nothing listens at the base: a fetch would fail with Err.
        fetch("http://127.0.0.1:1", &client, "t", "agenty", &mut req, files, None).await.unwrap();
        assert!(req.files.is_none());
        assert!(req.files_delivery.unwrap().text.contains("over the cloud's limits"));
    }

    #[tokio::test]
    async fn a_failed_signature_withholds_cloud_files_without_fetching() {
        let client = reqwest::Client::new();
        let files = manifest();
        let msgid = jekt_files::bind_cloud_msgid("m-1", &files);
        let mut req = InjectionRequest { wan_verified: Some(false), delivery_tier: Some("wan".into()), ..Default::default() };
        // Nothing listens at the base: a fetch would fail with Err.
        fetch("http://127.0.0.1:1", &client, "t", "agenty", &mut req, files, Some(&msgid)).await.unwrap();
        assert_eq!(req.files.as_ref().map(|f| f.len()), Some(1), "listed for `prepare` to say withheld");
    }
}
