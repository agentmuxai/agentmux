// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files sent with a jekt (`docs/specs/SPEC_JEKT_FILE_TRANSFER_2026_10_05.md`):
//! whether a request's files belong to it (§3.3), what the agent gets for
//! them (§6), and how they travel to another instance (§4).
//!
//! On the sender's instance the files are already in its attachment store:
//! the sender's MCP uploaded them before sending (§3.1). Another instance on
//! the same machine or the LAN gets them in the same request as the jekt,
//! after it, at `POST /agentmux/reactive/inject-files`; it checks the jekt
//! before reading a single file byte.

use agentmux_common::jekt_files;

use crate::backend::attachments::{self, prompt};
use crate::backend::reactive::sanitize::quote_in_jekt;
use crate::backend::reactive::types::{FilesDelivery, InjectionRequest, WanInstanceStatus};

/// Why a request's files can't be accepted at all, before anything is
/// delivered: they're malformed, the msgid doesn't bind them, or they
/// would have to come from another instance. The text is for the sender.
pub(crate) fn check(req: &InjectionRequest) -> Result<(), String> {
    if req.files.is_none() {
        return Ok(());
    }
    check_manifest(req)?;
    // `/inject` names files by hash, so only this instance's own senders may
    // use it. Another instance's files come with them (`inject-files`).
    if req.delivery_tier.as_deref() != Some("host") {
        return Err("files refused: files from another AgentMux come with the request, not by name".to_string());
    }
    Ok(())
}

/// A request's manifest is well formed and its msgid binds it.
fn check_manifest(req: &InjectionRequest) -> Result<(), String> {
    let files = req.files.as_deref().unwrap_or_default();
    jekt_files::validate(files).map_err(|e| format!("files refused: {e}"))?;
    let msgid = req.request_id.as_deref().unwrap_or("");
    if !jekt_files::msgid_binds(msgid, files) {
        return Err("files refused: they don't match the message id they were sent under".to_string());
    }
    Ok(())
}

/// Why trust rules out delivering a jekt's files (§6.4), or `None`.
fn withheld_reason(req: &InjectionRequest) -> Option<&'static str> {
    let failed = [req.sig_verified, req.reagent_verified, req.lan_verified, req.channel_verified, req.wan_verified]
        .contains(&Some(false))
        || req.wan_instance.as_ref().is_some_and(|i| i.status == WanInstanceStatus::Revoked);
    if failed {
        return Some("the sender's signature failed verification");
    }
    if req.delivery_tier.as_deref() == Some("lan") && req.lan_verified != Some(true) {
        return Some("files over the LAN need a verified sender");
    }
    None
}

/// Put what the agent gets for this jekt's files in `req.files_delivery`:
/// the list, a line saying they were withheld, or (for a msgid that binds
/// files when none came) a line saying they didn't arrive. Leaves it `None`
/// for a text-only jekt.
pub(crate) async fn prepare(req: &mut InjectionRequest) {
    let msgid = req.request_id.as_deref().unwrap_or("");
    let Some(files) = req.files.clone() else {
        if jekt_files::bound_digest(msgid).is_some() {
            req.files_delivery = Some(FilesDelivery {
                delivered: 0,
                text: "This message was sent with files that didn't arrive.".to_string(),
            });
        }
        return;
    };
    let count = files.len();
    let (noun, verb) = if count == 1 { ("file", "was") } else { ("files", "were") };
    if let Some(reason) = withheld_reason(req) {
        req.files_delivery = Some(FilesDelivery {
            delivered: 0,
            text: format!("{count} {noun} sent with this message {verb} withheld: {reason}."),
        });
        return;
    }
    let Some(svc) = attachments::get() else {
        req.files_delivery = Some(FilesDelivery {
            delivered: 0,
            text: format!("{count} {noun} sent with this message couldn't be delivered: the attachment store isn't available."),
        });
        return;
    };
    // Re-derive anything stored under an older fingerprint, and check each
    // file's size against the manifest: the hash names the file, the size
    // is part of what was signed.
    let mut present = Vec::with_capacity(count);
    for f in &files {
        let ok = svc.ensure_derived(&f.sha256).await.is_some_and(|meta| meta.bytes == f.size);
        present.push(ok);
    }
    // Every name the sender controls is quoted before it is used, the stored
    // copy's name included, so neither a name nor the path it gives the copy
    // can close the jekt block or open a list.
    let sender = quote_in_jekt(req.source_agent.as_deref().unwrap_or("Another agent"));
    let delivery = tokio::task::spawn_blocking(move || {
        let mut found = Vec::new();
        let mut missing = Vec::new();
        for (i, (f, ok)) in files.iter().zip(present).enumerate() {
            let name = quote_in_jekt(&f.name);
            match ok.then(|| svc.send_target(&f.sha256, &name)).flatten() {
                Some(t) => found.push(prompt::Resolved {
                    number: i + 1,
                    name,
                    path: t.path,
                    mime: t.mime,
                    page_count: t.page_count,
                    note: t.note,
                }),
                None => missing.push(name),
            }
        }
        FilesDelivery {
            delivered: found.len(),
            text: prompt::jekt_list_block(&sender, &found, &missing, &[]),
        }
    })
    .await
    .unwrap_or_else(|_| FilesDelivery {
        delivered: 0,
        text: format!("{count} {noun} sent with this message couldn't be delivered."),
    });
    req.files_delivery = Some(delivery);
}

// ---- Moving the files to another instance (spec §4) ----

/// The longest a forward with files may take: the bytes go with it.
pub(crate) const TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
/// The inject request in front of the files is small (a 10 KB message and
/// its signatures); anything longer is refused before it is read.
const MAX_REQUEST_JSON: u32 = 64 * 1024;
/// Transfers a receiver takes at once; another waits for the sender's retry.
static TRANSFERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
/// How long a LAN files jekt's msgid is remembered, so a captured request
/// can't be delivered twice: longer than any signature's freshness window.
const SEEN_FOR: std::time::Duration = std::time::Duration::from_secs(15 * 60);
static SEEN_LAN: std::sync::Mutex<Vec<(String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

/// Whether this LAN files jekt's msgid was delivered here before.
fn seen_before(msgid: &str) -> bool {
    let mut seen = SEEN_LAN.lock().unwrap_or_else(|e| e.into_inner());
    seen.retain(|(_, at)| at.elapsed() < SEEN_FOR);
    seen.iter().any(|(id, _)| id == msgid)
}

/// Remember a LAN files jekt's msgid once it is delivered, and only then:
/// a delivery that failed may be retried.
fn remember_delivered(msgid: &str) {
    let mut seen = SEEN_LAN.lock().unwrap_or_else(|e| e.into_inner());
    seen.push((msgid.to_string(), std::time::Instant::now()));
}

/// The body of a forward with files: the request's JSON length (u32,
/// big-endian), the JSON, then each file's bytes in manifest order, read
/// from this instance's store.
pub(crate) async fn transfer_body(req: &InjectionRequest) -> Result<reqwest::Body, String> {
    let svc = attachments::get().ok_or("files refused: the attachment store isn't available")?;
    Ok(reqwest::Body::wrap_stream(transfer_stream(&svc, req).await?))
}

type ByteStream = futures_util::stream::BoxStream<'static, std::io::Result<axum::body::Bytes>>;

async fn transfer_stream(svc: &attachments::Service, req: &InjectionRequest) -> Result<ByteStream, String> {
    use futures_util::StreamExt;
    let json = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    // The receiver reads at most this much before the files; a message too
    // long for it would be refused there on every retry.
    if json.len() > MAX_REQUEST_JSON as usize {
        return Err(format!(
            "files refused: the message is too long to send with files ({} KB with its signatures; at most {} KB)",
            json.len() / 1024,
            MAX_REQUEST_JSON / 1024
        ));
    }
    let mut head = (json.len() as u32).to_be_bytes().to_vec();
    head.extend_from_slice(&json);
    let mut parts = vec![futures_util::stream::once(async move { Ok::<_, std::io::Error>(axum::body::Bytes::from(head)) }).boxed()];
    for f in req.files.as_deref().unwrap_or_default() {
        let file = match svc.store().find_blob(&f.sha256) {
            Some((path, _)) => tokio::fs::File::open(path).await.ok(),
            None => None,
        };
        let Some(file) = file else {
            return Err(format!("files refused: {} is no longer in this AgentMux's store", f.name));
        };
        if file.metadata().await.map(|m| m.len()).ok() != Some(f.size) {
            return Err(format!("files refused: {} changed size in this AgentMux's store", f.name));
        }
        parts.push(tokio_util::io::ReaderStream::new(file).boxed());
    }
    Ok(futures_util::stream::iter(parts).flatten().boxed())
}

/// Whether the LAN peer at `url` takes files for `target`: its agent answer
/// lists the feature (spec §3.4).
pub(crate) async fn peer_accepts_files(client: &reqwest::Client, url: &str, auth_key: &str, target: &str) -> bool {
    let mut get = client.get(format!("{url}/agentmux/reactive/agent")).query(&[("id", target)]);
    if !auth_key.is_empty() {
        get = get.header(agentmux_common::AUTH_KEY_HEADER, auth_key);
    }
    let Ok(resp) = get.send().await else { return false };
    let Ok(body) = resp.json::<serde_json::Value>().await else { return false };
    body.get("jekt_features")
        .and_then(|f| f.as_array())
        .is_some_and(|f| f.iter().any(|v| v.as_str() == Some(jekt_files::FEATURE)))
}

/// An answer that ends the sender's search: the files were refused for a
/// reason no other candidate would change.
pub(crate) fn refused(req: &InjectionRequest, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "files_refused": true,
        "request_id": req.request_id.clone().unwrap_or_default(),
        "error": error.into(),
        "timestamp": agentmux_common::time::now_ms_u64(),
    })
}

/// `POST /agentmux/reactive/inject-files`: a jekt from another instance on
/// this machine (full key) or a LAN peer (`lan_key`), with its files after it
/// (spec §4). The jekt is checked as `/agentmux/reactive/inject` checks one,
/// before any file byte is read; then each file is streamed into the store,
/// hashed as it goes, and the jekt is delivered with the ones that arrived
/// intact. It is never forwarded on.
pub(super) async fn handle_reactive_inject_files(
    axum::extract::State(state): axum::extract::State<super::AppState>,
    axum::extract::Extension(auth_via): axum::extract::Extension<super::ReactiveAuthVia>,
    caller: Option<axum::extract::Extension<super::caller::Caller>>,
    body: axum::body::Body,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use futures_util::StreamExt;
    let stream = body.into_data_stream().map(|r| r.map_err(std::io::Error::other));
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let req = match read_request(&mut reader).await {
        Ok(req) => req,
        Err(e) => {
            let body = serde_json::json!({ "success": false, "error": e });
            return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
    };
    super::actor::check_actor(&state, caller.as_deref(), super::actor::ActorSite::Inject, req.source_agent.as_deref());
    let caller_uid = super::caller::attributed_uid(caller.as_deref());
    let Ok(_slot) = TRANSFERS.try_acquire() else {
        // Not a verdict on this candidate: the sender moves on.
        let body = serde_json::json!({ "success": false, "error": "busy receiving other files" });
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, axum::Json(body)).into_response();
    };
    axum::Json(receive(&state, auth_via, &caller_uid, req, &mut reader).await).into_response()
}

async fn read_request<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<InjectionRequest, String> {
    use tokio::io::AsyncReadExt;
    let len = reader.read_u32().await.map_err(|e| format!("no request: {e}"))?;
    if len == 0 || len > MAX_REQUEST_JSON {
        return Err(format!("request of {len} bytes refused"));
    }
    let mut json = vec![0u8; len as usize];
    reader.read_exact(&mut json).await.map_err(|e| format!("request cut short: {e}"))?;
    serde_json::from_slice(&json).map_err(|e| format!("request unreadable: {e}"))
}

async fn receive<R: tokio::io::AsyncRead + Unpin>(
    state: &super::AppState,
    auth_via: super::ReactiveAuthVia,
    caller_uid: &str,
    mut req: InjectionRequest,
    reader: &mut R,
) -> serde_json::Value {
    use super::reactive::*;
    // The same steps as `deliver()`, up to the local delivery.
    req.delivery_tier = Some(resolve_delivery_tier(auth_via, req.delivery_tier.as_deref()));
    req.audit_source_uid = caller_uid.to_string();
    super::actor::source_from_token(&state.mstore, caller_uid, &mut req);
    verify_jekt_signature(state, &mut req);
    verify_reagent_signature(&mut req, agentmux_common::time::now_secs());
    verify_lan_signature(state, &mut req).await;
    verify_cross_channel_signature(state, &mut req);
    req.route_unsigned_target();
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);

    if req.files.is_none() {
        return refused(&req, "files refused: the request names no files");
    }
    if let Err(e) = check_manifest(&req) {
        return refused(&req, e);
    }
    let tier = req.delivery_tier.clone().unwrap_or_default();
    if tier != "channel" && tier != "lan" {
        return refused(&req, format!("files refused: not taken from the {tier} tier here"));
    }
    let files = req.files.clone().unwrap_or_default();
    if let Err(e) = jekt_files::check_tier_limits(&files, &tier) {
        return refused(&req, format!("files refused: {e}"));
    }
    // Not here: the sender looks further, as for any forward.
    let target = req.target_agent.clone();
    if !state.reactive_handler.has_live_name(&target) && !state.reactive_handler.has_uid_registration(&target) {
        return serde_json::json!({ "success": false, "error": format!("agent not found: {target}") });
    }
    let withheld = withheld_reason(&req).is_some();
    if tier == "lan" && !withheld && seen_before(req.request_id.as_deref().unwrap_or("")) {
        return refused(&req, "files refused: this message was already delivered");
    }

    if withheld {
        // Read past the files unstored, so the sender gets this answer
        // rather than a reset connection.
        let total: u64 = files.iter().map(|f| f.size).sum();
        let mut rest = tokio::io::AsyncReadExt::take(reader, total);
        let _ = tokio::io::copy(&mut rest, &mut tokio::io::sink()).await;
    } else if let Some(svc) = attachments::get() {
        store_files(&svc, &files, reader).await;
    }
    prepare(&mut req).await;
    let delivered = req.files_delivery.as_ref().map_or(0, |d| d.delivered);
    let msgid = req.request_id.clone().unwrap_or_default();
    let resp = state.reactive_handler.inject_message(req);
    if tier == "lan" && resp.success {
        remember_delivered(&msgid);
    }
    let mut value = serde_json::to_value(&resp).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        obj.insert("files_delivered".into(), delivered.into());
    }
    value
}

/// Stream each file into the store, keeping the ones whose bytes match
/// their hash and size. A file that doesn't is dropped, and the list says it
/// didn't arrive; a stream that ends early leaves the rest missing.
async fn store_files<R: tokio::io::AsyncRead + Unpin>(
    svc: &std::sync::Arc<attachments::Service>,
    files: &[jekt_files::JektFile],
    reader: &mut R,
) {
    use sha2::Digest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for f in files {
        let tmp = svc.store().new_incoming_path();
        let copied = async {
            let mut out = tokio::fs::File::create(&tmp).await?;
            let mut hasher = sha2::Sha256::new();
            let mut left = f.size;
            let mut buf = vec![0u8; 64 * 1024];
            while left > 0 {
                let want = buf.len().min(left as usize);
                let n = reader.read(&mut buf[..want]).await?;
                if n == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "cut short"));
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n]).await?;
                left -= n as u64;
            }
            out.flush().await?;
            Ok::<_, std::io::Error>(hex::encode(hasher.finalize()))
        }
        .await;
        match copied {
            Ok(sha) if sha == f.sha256 => {
                if let Err(e) = svc.commit_upload(tmp.clone(), sha, f.name.clone()).await {
                    tracing::warn!(file = %f.name, error = %e.message, "jekt files: received file not stored");
                    let _ = tokio::fs::remove_file(&tmp).await;
                }
            }
            Ok(_) => {
                tracing::warn!(file = %f.name, "jekt files: received bytes don't match their hash");
                let _ = tokio::fs::remove_file(&tmp).await;
            }
            Err(e) => {
                tracing::warn!(file = %f.name, error = %e, "jekt files: transfer cut short");
                let _ = tokio::fs::remove_file(&tmp).await;
                return;
            }
        }
    }
}

/// Whether a same-machine candidate can take this jekt: any can take a text
/// jekt; one with files only if its registry entry advertises them. A
/// candidate passed over for that sets `too_old`.
pub(crate) fn entry_takes(req: &InjectionRequest, entry: &crate::backend::reactive::registry::AgentEntry, too_old: &mut bool) -> bool {
    if req.files.is_none() || entry.jekt_features.iter().any(|f| f == jekt_files::FEATURE) {
        return true;
    }
    *too_old = true;
    false
}

/// Whether the LAN peer found for this jekt can take it. A jekt with files
/// needs the peer to advertise them (else `too_old`), and must be within the
/// LAN's limits, or it is refused outright: no other route is left for it.
pub(crate) async fn lan_peer_takes(
    client: &reqwest::Client,
    req: &InjectionRequest,
    url: &str,
    auth_key: &str,
    too_old: &mut bool,
) -> Result<bool, serde_json::Value> {
    let Some(files) = req.files.as_deref() else {
        return Ok(true);
    };
    if !peer_accepts_files(client, url, auth_key, &req.target_agent).await {
        *too_old = true;
        return Ok(false);
    }
    jekt_files::check_tier_limits(files, "lan").map_err(|e| refused(req, format!("files refused: {e}")))?;
    Ok(true)
}

/// The answer for a jekt with files that no instance on this machine or the
/// LAN took: held for an agent defined here that isn't running, otherwise
/// refused with the reason. Files don't go through the relay yet (spec §8).
pub(crate) async fn no_route(
    state: &super::AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
    candidate_seen: bool,
    too_old: bool,
) -> serde_json::Value {
    use super::reactive::{hold_for_absent_target, HoldReason};
    // A target running in an older build is alive, not absent: holding its
    // files here would only wait out the 24 hours.
    if !candidate_seen && !too_old {
        if let Some(held) = hold_for_absent_target(state, auth_via, req, resp, HoldReason::NotRunning).await {
            return held;
        }
    }
    let target = &req.target_agent;
    refused(
        req,
        if too_old {
            format!("files refused: {target}'s AgentMux is too old to receive files; nothing was sent")
        } else if candidate_seen {
            format!("files refused: {target} couldn't take them just now; nothing was sent, try again shortly")
        } else {
            format!("files refused: {target} isn't on this machine or the LAN, and files can't go through the cloud relay yet; nothing was sent")
        },
    )
}

/// `answer` with how many files the agent got, when the jekt had files.
pub(crate) fn with_count(mut answer: serde_json::Value, req: &InjectionRequest) -> serde_json::Value {
    if let (Some(d), Some(obj)) = (req.files_delivery.as_ref(), answer.as_object_mut()) {
        obj.insert("files_delivered".into(), d.delivered.into());
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::jekt_files::JektFile;

    fn files() -> Vec<JektFile> {
        vec![JektFile { sha256: "ab".repeat(32), name: "shot.png".into(), size: 3 }]
    }

    fn req_with(files: Vec<JektFile>, msgid: &str, tier: &str) -> InjectionRequest {
        InjectionRequest {
            target_agent: "agenta".into(),
            message: "look".into(),
            request_id: Some(msgid.into()),
            delivery_tier: Some(tier.into()),
            files: Some(files),
            ..Default::default()
        }
    }

    #[test]
    fn a_bound_manifest_on_this_instance_passes() {
        let f = files();
        assert_eq!(check(&req_with(f.clone(), &jekt_files::bind_msgid("m", &f), "host")), Ok(()));
    }

    #[test]
    fn a_manifest_the_msgid_does_not_bind_is_refused() {
        let f = files();
        let mut other = f.clone();
        other[0].size = 4;
        let err = check(&req_with(other, &jekt_files::bind_msgid("m", &f), "host")).unwrap_err();
        assert!(err.contains("don't match the message id"), "{err}");
        assert!(check(&req_with(f, "m", "host")).is_err(), "an unbound msgid");
    }

    #[test]
    fn files_named_by_hash_only_come_from_this_instance() {
        let f = files();
        let msgid = jekt_files::bind_msgid("m", &f);
        for tier in ["channel", "lan", "wan"] {
            assert!(check(&req_with(f.clone(), &msgid, tier)).is_err(), "{tier}");
        }
    }

    #[test]
    fn a_text_jekt_passes_untouched() {
        let req = InjectionRequest { request_id: Some("m".into()), ..Default::default() };
        assert_eq!(check(&req), Ok(()));
    }

    #[test]
    fn an_active_verification_failure_withholds_the_files() {
        let f = files();
        let mut req = req_with(f.clone(), &jekt_files::bind_msgid("m", &f), "host");
        assert_eq!(withheld_reason(&req), None, "self-declared on this instance");
        req.sig_verified = Some(false);
        assert!(withheld_reason(&req).is_some());
        req.sig_verified = Some(true);
        req.channel_verified = Some(false);
        assert!(withheld_reason(&req).is_some(), "any failed check withholds");
    }

    #[test]
    fn lan_files_need_a_verified_sender() {
        let f = files();
        let mut req = req_with(f.clone(), &jekt_files::bind_msgid("m", &f), "lan");
        assert!(withheld_reason(&req).is_some());
        req.lan_verified = Some(true);
        assert_eq!(withheld_reason(&req), None);
    }

    #[tokio::test]
    async fn withheld_and_missing_files_are_said_so() {
        let f = files();
        let mut req = req_with(f.clone(), &jekt_files::bind_msgid("m", &f), "host");
        req.sig_verified = Some(false);
        prepare(&mut req).await;
        let d = req.files_delivery.unwrap();
        assert_eq!(d.delivered, 0);
        assert!(d.text.contains("1 file sent with this message was withheld"), "{}", d.text);

        let mut bare = InjectionRequest { request_id: Some(jekt_files::bind_msgid("m", &f)), ..Default::default() };
        prepare(&mut bare).await;
        assert!(bare.files_delivery.unwrap().text.contains("didn't arrive"));

        let mut text = InjectionRequest { request_id: Some("m".into()), ..Default::default() };
        prepare(&mut text).await;
        assert_eq!(text.files_delivery, None);
    }

    // ---- Transfer to another instance (spec §4) ----

    use std::sync::Arc;

    fn service(dir: &std::path::Path) -> Arc<attachments::Service> {
        Arc::new(attachments::Service::new(
            attachments::store::Store::new(dir.join("attachments")),
            Arc::new(crate::backend::mps::Broker::new()),
            Arc::new(crate::backend::wconfig::ConfigState::new()),
        ))
    }

    fn sha(bytes: &[u8]) -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(bytes))
    }

    fn manifest_entry(name: &str, bytes: &[u8]) -> JektFile {
        JektFile { sha256: sha(bytes), name: name.into(), size: bytes.len() as u64 }
    }

    /// The framed body as a sender writes it, by hand.
    fn framed(req: &InjectionRequest, files: &[&[u8]]) -> Vec<u8> {
        let json = serde_json::to_vec(req).unwrap();
        let mut out = (json.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&json);
        for f in files {
            out.extend_from_slice(f);
        }
        out
    }

    #[tokio::test]
    async fn files_cross_from_one_store_to_another_intact() {
        let d = tempfile::tempdir().unwrap();
        let (sender, receiver) = (service(&d.path().join("a")), service(&d.path().join("b")));
        let bytes = b"File transfer test note.\n".to_vec();
        let src = d.path().join("notes.txt");
        std::fs::write(&src, &bytes).unwrap();
        let (tmp, id, _) = sender.store().copy_in(&src, |_| true).unwrap();
        sender.commit_upload(tmp, id.clone(), "notes.txt".into()).await.unwrap();

        let files = vec![manifest_entry("notes.txt", &bytes)];
        let req = req_with(files.clone(), &jekt_files::bind_msgid("m", &files), "channel");
        let mut reader = tokio_util::io::StreamReader::new(transfer_stream(&sender, &req).await.unwrap());
        let got = read_request(&mut reader).await.unwrap();
        assert_eq!(got.files, Some(files.clone()));
        assert_eq!(got.request_id, req.request_id, "the msgid that binds them travels too");
        assert!(got.files_delivery.is_none(), "a sender can't supply the list");

        store_files(&receiver, &files, &mut reader).await;
        let (path, _) = receiver.store().find_blob(&id).expect("stored on the far side");
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn a_file_whose_bytes_do_not_match_is_dropped_and_the_next_still_arrives() {
        let d = tempfile::tempdir().unwrap();
        let receiver = service(d.path());
        let files = vec![manifest_entry("a.txt", b"hello"), manifest_entry("b.txt", b"world")];
        let req = req_with(files.clone(), &jekt_files::bind_msgid("m", &files), "lan");
        let body = framed(&req, &[b"jello", b"world"]);
        let mut reader = &body[..];
        read_request(&mut reader).await.unwrap();
        store_files(&receiver, &files, &mut reader).await;
        assert!(receiver.store().find_blob(&sha(b"hello")).is_none());
        assert!(receiver.store().find_blob(&sha(b"jello")).is_none(), "nothing stored under another hash");
        assert!(receiver.store().find_blob(&sha(b"world")).is_some());
        assert!(reader.is_empty(), "exactly the declared bytes were read");
    }

    #[tokio::test]
    async fn a_transfer_cut_short_stores_only_what_arrived_whole() {
        let d = tempfile::tempdir().unwrap();
        let receiver = service(d.path());
        let files = vec![manifest_entry("a.txt", b"hello"), manifest_entry("b.txt", b"world")];
        let req = req_with(files.clone(), &jekt_files::bind_msgid("m", &files), "lan");
        let body = framed(&req, &[b"hello", b"wor"]);
        let mut reader = &body[..];
        read_request(&mut reader).await.unwrap();
        store_files(&receiver, &files, &mut reader).await;
        assert!(receiver.store().find_blob(&sha(b"hello")).is_some());
        assert!(receiver.store().find_blob(&sha(b"world")).is_none());
        let incoming = d.path().join("attachments").join("incoming");
        let left = std::fs::read_dir(&incoming).map(|r| r.count()).unwrap_or(0);
        assert_eq!(left, 0, "no partial file left behind");
    }

    #[tokio::test]
    async fn a_message_too_long_to_carry_files_is_refused_by_the_sender() {
        let d = tempfile::tempdir().unwrap();
        let f = files();
        let mut req = req_with(f.clone(), &jekt_files::bind_msgid("m", &f), "channel");
        req.message = "x".repeat(MAX_REQUEST_JSON as usize);
        let err = transfer_stream(&service(d.path()), &req).await.err().unwrap();
        assert!(err.contains("too long to send with files"), "{err}");
    }

    #[tokio::test]
    async fn an_oversized_or_missing_request_header_is_refused_before_reading_on() {
        let big = (MAX_REQUEST_JSON + 1).to_be_bytes();
        assert!(read_request(&mut &big[..]).await.unwrap_err().contains("refused"));
        assert!(read_request(&mut &[0u8, 0, 0, 0][..]).await.is_err());
        assert!(read_request(&mut &[0u8, 0][..]).await.is_err());
    }

    #[test]
    fn only_a_candidate_that_advertises_files_gets_them() {
        let f = files();
        let with_files = req_with(f.clone(), &jekt_files::bind_msgid("m", &f), "host");
        let text = InjectionRequest { request_id: Some("m".into()), ..Default::default() };
        let mut entry: crate::backend::reactive::registry::AgentEntry = serde_json::from_value(serde_json::json!({
            "agent_id": "agenta", "local_url": "http://127.0.0.1:1", "block_id": "b", "pid": 1, "updated_at": 0
        }))
        .unwrap();
        let mut too_old = false;
        assert!(entry_takes(&text, &entry, &mut too_old) && !too_old, "any entry takes a text jekt");
        assert!(!entry_takes(&with_files, &entry, &mut too_old) && too_old, "an older build's entry");
        entry.jekt_features = vec![jekt_files::FEATURE.into()];
        let mut too_old = false;
        assert!(entry_takes(&with_files, &entry, &mut too_old) && !too_old);
    }

    #[test]
    fn a_lan_files_jekt_is_delivered_once() {
        let id = format!("m-{}", std::process::id());
        assert!(!seen_before(&id));
        assert!(!seen_before(&id), "not delivered yet: a retry is fine");
        remember_delivered(&id);
        assert!(seen_before(&id), "a replay within the window");
        assert!(!seen_before(&format!("{id}-other")));
    }
}
