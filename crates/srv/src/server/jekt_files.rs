// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files sent with a jekt, on the receiving side: whether a request's files
//! belong to it, and what the agent gets for them
//! (`docs/specs/SPEC_JEKT_FILE_TRANSFER_2026_10_05.md` §3.3, §6).
//!
//! The files themselves are already in this instance's attachment store:
//! the sender's MCP uploaded them before sending (§3.1). Files from another
//! instance arrive with their own request (§4, a later phase); until then a
//! request whose files would have to come from elsewhere is refused.

use agentmux_common::jekt_files;

use crate::backend::attachments::{self, prompt};
use crate::backend::reactive::types::{FilesDelivery, InjectionRequest, WanInstanceStatus};

/// Why a request's files can't be accepted at all, before anything is
/// delivered: they're malformed, the msgid doesn't bind them, or they
/// would have to come from another instance. The text is for the sender.
pub(crate) fn check(req: &InjectionRequest) -> Result<(), String> {
    let Some(files) = req.files.as_deref() else {
        return Ok(());
    };
    jekt_files::validate(files).map_err(|e| format!("files refused: {e}"))?;
    let msgid = req.request_id.as_deref().unwrap_or("");
    if !jekt_files::msgid_binds(msgid, files) {
        return Err("files refused: they don't match the message id they were sent under".to_string());
    }
    // Only the instance that holds them can deliver them. A forward from
    // another instance, a LAN peer or the relay names files this store may
    // not have, or may have from someone else.
    if req.delivery_tier.as_deref() != Some("host") {
        return Err(
            "files refused: files can only be sent to an agent on the same AgentMux for now".to_string(),
        );
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
    let sender = req.source_agent.clone().unwrap_or_else(|| "Another agent".to_string());
    let delivery = tokio::task::spawn_blocking(move || {
        let mut found = Vec::new();
        let mut missing = Vec::new();
        for (i, (f, ok)) in files.iter().zip(present).enumerate() {
            match ok.then(|| svc.send_target(&f.sha256, &f.name)).flatten() {
                Some(t) => found.push(prompt::Resolved {
                    number: i + 1,
                    name: f.name.clone(),
                    path: t.path,
                    mime: t.mime,
                    page_count: t.page_count,
                    note: t.note,
                }),
                None => missing.push(f.name.clone()),
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
    fn files_from_another_instance_are_refused_for_now() {
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
}
