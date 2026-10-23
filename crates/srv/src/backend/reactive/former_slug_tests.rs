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
    assert_eq!(h.get_delivery_target("loap-loap-2-loap-3").map(|r| r.block_id.clone()).as_deref(), Some("block-loap3"));
    assert!(matches!(h.lookup_delivery_target("loap-loap-2-loap-3"), LookupOutcome::One(r) if r.uid.as_deref() == Some(UID)));
}

/// The lookups that authenticate and act as an agent never answer a former
/// slug: a retired name can't be resolved to the pane it used to name.
#[test]
fn generic_lookups_never_answer_a_former_slug() {
    let (h, _, calls) = renamed_agent();
    assert!(h.get_agent("loap-loap-2-loap-3").is_none());
    assert!(matches!(h.lookup_by_name("loap-loap-2-loap-3"), LookupOutcome::NotFound));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the store is never asked");
    assert_eq!(h.get_agent("loap-3").map(|r| r.block_id.clone()).as_deref(), Some("block-loap3"));
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

// ---- Former display names (`former_names`, `display_rename`) ----

const ZED: &str = "a0dfc499-0000-4000-8000-0000000000ed";

/// A store whose agent `ZED` (slug `zed`) was renamed from "Zed Prime" to
/// "Zork", and a handler resolving former names through it, with the agent's
/// pane registered under its new name.
fn renamed_display() -> (std::sync::Arc<crate::backend::storage::store::Store>, Handler, Arc<Mutex<Vec<String>>>) {
    let store = std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    let mut def = crate::backend::storage::agents::test_agent_def(ZED, "Zed Prime", "claude", "host", 1, "");
    def.slug = "zed".into();
    store.agent_def_insert(&mut def).unwrap();
    def.name = "Zork".into();
    store.agent_def_update(&mut def).unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut h = Handler::new();
    let sink = sent.clone();
    h.set_input_sender(Arc::new(move |block: &str, _data: &[u8]| {
        sink.lock().unwrap().push(block.to_string());
        Ok(())
    }));
    let resolver = store.clone();
    h.set_former_slug_resolver(Arc::new(move |n: &str| resolver.former_name_owner(n).ok().flatten().map(|o| o.uid)));
    h.register_agent_full("Zork", "block-zed", None, 0, Some("zed"), Some(ZED), "test.former_name").unwrap();
    (store, h, sent)
}

#[tokio::test]
async fn a_message_to_a_former_display_name_reaches_the_agent() {
    let (_store, mut h, sent) = renamed_display();
    let resp = inject(&mut h, "Zed-Prime");
    assert!(resp.success, "{:?}", resp.error);
    assert_eq!(resp.block_id.as_deref(), Some("block-zed"));
    assert_eq!(sent.lock().unwrap().first().map(String::as_str), Some("block-zed"));
    assert!(matches!(h.lookup_delivery_target("zed-prime"), LookupOutcome::One(r) if r.uid.as_deref() == Some(ZED)));
    assert!(h.get_agent("zed-prime").is_none(), "a generic lookup never answers it");
    assert!(matches!(h.lookup_by_name("Zed-Prime"), LookupOutcome::NotFound));
}

/// Another agent listing the same former name, or holding it now, leaves it
/// unresolved: not found here, so the miss path may look elsewhere.
#[tokio::test]
async fn an_ambiguous_former_display_name_is_not_found() {
    let (store, mut h, _) = renamed_display();
    let mut other = crate::backend::storage::agents::test_agent_def("uid-other", "Zed Prime", "claude", "host", 2, "");
    other.slug = "other".into();
    store.agent_def_insert(&mut other).unwrap();
    let resp = inject(&mut h, "Zed-Prime");
    assert!(resp.error.as_deref().is_some_and(|e| e.starts_with("agent not found")), "held now: {:?}", resp.error);
    other.name = "Other".into();
    store.agent_def_update(&mut other).unwrap();
    let resp = inject(&mut h, "zed-prime");
    assert!(resp.error.as_deref().is_some_and(|e| e.starts_with("agent not found")), "listed twice: {:?}", resp.error);
}

/// A sender claiming a former display name is unverified, whatever verdict
/// it carried; the same agent's slug and current display name are not.
#[tokio::test]
async fn a_sender_claiming_a_former_display_name_is_unverified() {
    let (_store, mut h, _) = renamed_display();
    h.register_agent_full("AgentY", "block-y", None, 0, Some("agenty"), Some("uid-y"), "test.former_name").unwrap();
    let from = |source: &str| InjectionRequest {
        target_agent: "agenty".into(),
        message: "hello".into(),
        source_agent: Some(source.into()),
        request_id: Some(format!("req-from-{source}")),
        sig_verified: Some(true),
        ..Default::default()
    };
    let resp = h.inject_message(from("Zed-Prime"));
    assert!(resp.success, "{:?}", resp.error);
    assert_eq!(resp.effective_tier.as_deref(), Some("sensitive"));
    assert_eq!(resp.requires_stop, Some(true));
    for current in ["zed", "Zork"] {
        let resp = h.inject_message(from(current));
        assert!(resp.success, "{:?}", resp.error);
        assert_ne!(resp.requires_stop, Some(true), "{current}");
    }
}

/// A rename moves the pane's display binding at once: the old name is no
/// longer held (it resolves as a former name only), the stable name, UID, tab
/// and nonce stay.
#[test]
fn a_rename_rebinds_the_pane_and_retires_the_old_name() {
    let mut h = Handler::new();
    h.register_agent_full("Zed-Prime", "block-zed", Some("tab-1"), 7, Some("zed"), Some(ZED), "test.former_name").unwrap();
    assert_eq!(h.rebind_renamed(ZED, "Other Title", "Zork"), None, "a pane under another name keeps it");
    let r = h.rebind_renamed(ZED, "Zed Prime", "Zork").unwrap();
    assert_eq!((r.display.as_str(), r.retired.as_deref()), ("Zork", Some("Zed-Prime")));
    assert_eq!(r.names, ["Zork", "zed"]);
    assert!(!h.has_live_name("zed-prime"));
    let reg = h.get_agent("zork").unwrap();
    assert_eq!((reg.block_id.as_str(), reg.tab_id.as_deref(), reg.registration_nonce), ("block-zed", Some("tab-1"), 7));
    assert_eq!(h.get_agent("zed").map(|r| r.block_id.clone()).as_deref(), Some("block-zed"));
    assert_eq!(h.rebind_renamed(ZED, "Zork", "zork"), None, "nothing changes");
    assert_eq!(h.rebind_renamed("uid-none", "Zork", "Zap"), None);
}

/// A pane whose display binding is still its stable name (before its first
/// turn) is rebound, and that name stays held.
#[test]
fn a_rename_never_retires_a_name_the_pane_still_holds() {
    let mut h = Handler::new();
    h.register_agent_full("zed", "block-zed", None, 0, Some("zed"), Some(ZED), "test.former_name").unwrap();
    let r = h.rebind_renamed(ZED, "zed", "Zork").unwrap();
    assert_eq!(r.retired, None);
    assert!(h.has_live_name("zed") && h.has_live_name("zork"));
}

// ── #1459: a current display name's route form ("Naki-2" for "Naki #2") ──

const NAKI: &str = "e4790000-0000-4000-8000-000000000001";

/// A store holding `Naki #2` (slug `naki-naki-2`), and a handler where it runs
/// registered by its slug only, as a pane is before any typed turn, with both
/// resolvers installed the way srv installs them.
fn display_route_agent() -> (Arc<crate::backend::storage::store::Store>, Handler, Arc<Mutex<Vec<String>>>) {
    let store = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    let mut def = crate::backend::storage::agents::test_agent_def(NAKI, "Naki #2", "claude", "host", 1, "");
    def.slug = "naki-naki-2".into();
    store.agent_def_insert(&mut def).unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut h = Handler::new();
    let sink = sent.clone();
    h.set_input_sender(Arc::new(move |block: &str, _data: &[u8]| {
        sink.lock().unwrap().push(block.to_string());
        Ok(())
    }));
    let former = store.clone();
    h.set_former_slug_resolver(Arc::new(move |n: &str| former.former_name_owner(n).ok().flatten().map(|o| o.uid)));
    let current = store.clone();
    h.set_display_route_resolver(Arc::new(move |n: &str| current.display_route_owner(n).ok().flatten().map(|o| o.uid)));
    h.register_agent_full("naki-naki-2", "block-naki", None, 0, Some("naki-naki-2"), Some(NAKI), "test.display_route").unwrap();
    (store, h, sent)
}

#[tokio::test]
async fn a_message_to_a_current_display_names_route_reaches_the_agent() {
    let (_store, mut h, sent) = display_route_agent();
    let resp = inject(&mut h, "Naki-2");
    assert!(resp.success, "{:?}", resp.error);
    assert_eq!(resp.block_id.as_deref(), Some("block-naki"));
    assert_eq!(sent.lock().unwrap().first().map(String::as_str), Some("block-naki"));
    assert!(matches!(h.lookup_delivery_target("naki-2"), LookupOutcome::One(r) if r.uid.as_deref() == Some(NAKI)));
    // Delivery only: the lookups that authenticate and act never answer it.
    assert!(h.get_agent("naki-2").is_none());
    assert!(matches!(h.lookup_by_name("Naki-2"), LookupOutcome::NotFound));
}

/// Two display names that route the same way: no guess, not found here.
#[tokio::test]
async fn an_ambiguous_display_route_is_not_found() {
    let (store, mut h, sent) = display_route_agent();
    let mut other = crate::backend::storage::agents::test_agent_def("uid-naki-b", "Naki 2", "claude", "host", 2, "");
    other.slug = "naki-b".into();
    store.agent_def_insert(&mut other).unwrap();
    let resp = inject(&mut h, "Naki-2");
    assert!(resp.error.as_deref().is_some_and(|e| e.starts_with("agent not found")), "{:?}", resp.error);
    assert!(sent.lock().unwrap().is_empty());
}

/// A name that is one agent's current display route and another's former
/// display name goes to its current holder.
#[tokio::test]
async fn the_current_holder_of_a_name_wins_over_its_former_owner() {
    let (store, mut h, sent) = display_route_agent();
    let mut old = crate::backend::storage::agents::test_agent_def("uid-was-naki", "Naki #2", "claude", "host", 3, "");
    old.slug = "was-naki".into();
    store.agent_def_insert(&mut old).unwrap();
    old.name = "Renamed".into();
    store.agent_def_update(&mut old).unwrap();
    h.register_agent_full("was-naki", "block-was", None, 0, Some("was-naki"), Some("uid-was-naki"), "test.display_route").unwrap();
    let resp = inject(&mut h, "Naki-2");
    assert_eq!(resp.block_id.as_deref(), Some("block-naki"), "{:?}", resp.error);
    let sent = sent.lock().unwrap();
    assert!(!sent.is_empty() && sent.iter().all(|b| b == "block-naki"), "{sent:?}");
}

/// A display route never names a sender: the sender rule asks the former
/// slug resolver alone, so a claim of "Naki-2" is not refused as a former
/// name (it is verified, or not, by its signature as before).
#[test]
fn a_display_route_is_never_a_former_sender() {
    let (_store, h, _) = display_route_agent();
    let mut req = InjectionRequest { source_agent: Some("Naki-2".into()), ..Default::default() };
    assert!(!h.refuse_former_slug_sender(&mut req));
    assert_eq!(req.sig_verified, None);
}
