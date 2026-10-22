// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Delivery to a former slug (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` Phase 1).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::super::*;
use super::{note_delivery, refuse_former_slug_sender};
use crate::backend::reactive::types::{InjectionRequest, InjectionResponse};

const UID: &str = "a0dfc499-0000-4000-8000-000000000003";

/// A handler whose agent `UID` runs on `block-loap3` as `Loap-3` / `loap-3`,
/// whose former slug is `loap-loap-2-loap-3`, and which records every block
/// it sent to. The resolver counts its calls.
fn renamed_agent() -> (Handler, Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = Handler::new();
    let sink = sent.clone();
    h.set_input_sender(Arc::new(move |block: &str, _data: &[u8]| {
        sink.lock().unwrap().push(block.to_string());
        Ok(())
    }));
    let counter = calls.clone();
    h.set_former_slug_resolver(Arc::new(move |name: &str| {
        counter.fetch_add(1, Ordering::SeqCst);
        (name == "loap-loap-2-loap-3").then(|| UID.to_string())
    }));
    h.register_agent_full("Loap-3", "block-loap3", None, 0, Some("loap-3"), Some(UID), "test.former_slug").unwrap();
    (h, sent, calls)
}

fn inject(h: &mut Handler, target: &str) -> InjectionResponse {
    h.inject_message(InjectionRequest {
        target_agent: target.to_string(),
        message: "hello".to_string(),
        request_id: Some(format!("req-former-{target}")),
        ..Default::default()
    })
}

#[tokio::test]
async fn a_message_to_a_former_slug_reaches_the_agents_uid() {
    let (mut h, sent, _) = renamed_agent();
    // The pane confirms its UID, as the controller does: the check runs
    // against the agent's UID, not the name it was sent to.
    h.set_uid_identity_confirmer(Arc::new(|_| Some(UID.to_string())));
    let resp = inject(&mut h, "Loap-Loap-2-Loap-3");
    assert!(resp.success, "{:?}", resp.error);
    assert_eq!(resp.block_id.as_deref(), Some("block-loap3"));
    assert_eq!(sent.lock().unwrap().first().map(String::as_str), Some("block-loap3"));
    assert_eq!(h.get_agent("loap-loap-2-loap-3").map(|r| r.block_id.clone()).as_deref(), Some("block-loap3"));
    assert!(matches!(h.lookup_by_name("loap-loap-2-loap-3"), LookupOutcome::One(r) if r.uid.as_deref() == Some(UID)));
}

/// A live name or current slug never asks the resolver: it adds nothing to a
/// delivery that resolves as before, and can't take a name from its holder.
#[tokio::test]
async fn live_names_win_and_skip_the_resolver() {
    let (mut h, sent, calls) = renamed_agent();
    h.register_agent_full("loap-loap-2-loap-3", "block-other", None, 0, None, None, "test.former_slug").unwrap();
    assert!(inject(&mut h, "loap-3").success);
    let resp = inject(&mut h, "loap-loap-2-loap-3");
    assert!(resp.success, "{:?}", resp.error);
    assert_eq!(resp.block_id.as_deref(), Some("block-other"), "the live holder of the name gets it");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(sent.lock().unwrap().iter().any(|b| b == "block-other"));
}

/// A former slug whose agent isn't registered here is "not found", so the
/// server's miss path can look for it elsewhere; and a pane whose own UID is
/// another agent's is refused, as for any UID target.
#[tokio::test]
async fn a_former_slug_is_not_found_without_its_agent_and_checked_by_uid() {
    let (mut h, _, _) = renamed_agent();
    let resp = inject(&mut h, "nobody-old");
    assert!(resp.error.as_deref().is_some_and(|e| e.starts_with("agent not found")), "{:?}", resp.error);

    h.set_uid_identity_confirmer(Arc::new(|_| Some("some-other-uid".to_string())));
    let resp = inject(&mut h, "loap-loap-2-loap-3");
    assert!(!resp.success);
    assert!(resp.error.as_deref().unwrap().contains("identity mismatch"), "{:?}", resp.error);
}

#[test]
fn a_delivery_through_a_former_slug_is_logged_once_an_hour_per_name() {
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }
    let out = Capture::default();
    let subscriber = tracing_subscriber::fmt().with_writer(out.clone()).with_max_level(tracing::Level::INFO).finish();
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..3 {
            note_delivery("former-log-a", UID);
        }
        note_delivery("FORMER-LOG-B", UID);
    });
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    assert_eq!(text.matches("former-log-a").count(), 1, "{text}");
    assert_eq!(text.matches("FORMER-LOG-B").count(), 1, "{text}");
}

/// The relay's WebSocket path (`cloud_subscriber::delivery`) hands the
/// handler a WAN request directly, with the WAN verdict it checked. A sender
/// claiming a former slug is unverified there too, even from an approved
/// install: forced sensitive and stopped. The same sender under a live name
/// is untouched.
#[tokio::test]
async fn a_wan_jekt_from_a_former_slug_is_unverified_on_the_relay_path() {
    use crate::backend::reactive::types::{WanInstanceInfo, WanInstanceStatus};
    let (mut h, _, _) = renamed_agent();
    h.register_agent_full("AgentY", "block-y", None, 0, Some("agenty"), Some("uid-y"), "test.former_slug").unwrap();
    let wan = |source: &str| InjectionRequest {
        target_agent: "agenty".into(),
        message: "review is in".into(),
        source_agent: Some(source.into()),
        request_id: Some(format!("req-wan-{source}")),
        delivery_tier: Some("wan".into()),
        wan_verified: Some(true),
        wan_instance: Some(WanInstanceInfo { id: "inst".into(), label: "host~inst".into(), status: WanInstanceStatus::Approved }),
        ..Default::default()
    };

    let resp = h.inject_message(wan("Loap-Loap-2-Loap-3"));
    assert!(resp.success, "{:?}", resp.error);
    assert_eq!(resp.effective_tier.as_deref(), Some("sensitive"));
    assert_eq!(resp.requires_stop, Some(true));

    let resp = h.inject_message(wan("loap-3"));
    assert!(resp.success, "{:?}", resp.error);
    assert_ne!(resp.effective_tier.as_deref(), Some("sensitive"));
    assert_ne!(resp.requires_stop, Some(true));
}

/// The rule itself: a live name or an unknown one passes; a former slug is
/// counted unverified whatever verdict it carried.
#[test]
fn the_rule_refuses_a_former_slug_only() {
    let req = |source: &str| InjectionRequest { source_agent: Some(source.into()), sig_verified: Some(true), ..Default::default() };
    let owner = |n: &str| (n == "old-id").then(|| UID.to_string());
    let mut r = req("Old-Id");
    assert!(refuse_former_slug_sender(&mut r, |_| false, owner));
    assert_eq!(r.sig_verified, Some(false));
    let mut r = req("old-id");
    assert!(!refuse_former_slug_sender(&mut r, |_| true, owner), "a live name is its holder's");
    assert_eq!(r.sig_verified, Some(true));
    let mut r = req("someone");
    assert!(!refuse_former_slug_sender(&mut r, |_| false, owner));
    let mut r = InjectionRequest::default();
    assert!(!refuse_former_slug_sender(&mut r, |_| false, owner));
}
