// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The concurrent LAN peer fan-out (`query_peers_concurrently`) and how a
//! peer's pubkey answer is read.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Spawn a minimal HTTP peer that waits `delay` before answering `status`.
/// Returns its `127.0.0.1:port`. Same raw-TCP shape as the mock servers in
/// `muxbus::pkce`'s tests — no extra dev-dependency needed.
async fn mock_peer(delay: std::time::Duration, status: &'static str, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                tokio::time::sleep(delay).await;
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("127.0.0.1:{}", addr.port())
}

fn peer_at(addr: &str) -> LanInstance {
    let (host, port) = addr.rsplit_once(':').unwrap();
    LanInstance {
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
        agent_seats: Default::default(),
        first_seen: 0,
        last_seen: 0,
        last_polled_ok: 0,
        other_ttl_secs: 4500,
    }
}

/// The regression this fan-out exists to prevent: a slow peer listed BEFORE
/// the peer that actually has the agent must not delay the answer.
///
/// Sequentially this took `LAN_PEER_QUERY_TIMEOUT_SECS` (2s) to time the
/// slow peer out before even trying the second one. Concurrently the fast
/// peer answers immediately. The 1500ms bound sits well clear of both the
/// ~0ms concurrent path and the ≥2000ms sequential one, so it discriminates
/// without being timing-fragile.
#[tokio::test]
async fn a_slow_peer_does_not_delay_a_fast_one() {
    let slow = mock_peer(
        std::time::Duration::from_secs(LAN_PEER_QUERY_TIMEOUT_SECS + 5),
        "200 OK",
        "{}",
    )
    .await;
    let fast = mock_peer(std::time::Duration::from_millis(0), "200 OK", "{}").await;
    // Slow first — the ordering that defeated the sequential loop.
    let peers = vec![peer_at(&slow), peer_at(&fast)];
    let http = reqwest::Client::new();

    let started = std::time::Instant::now();
    let mut inflight = query_peers_concurrently(&peers, "agent-x", &http);
    let mut winner = None;
    {
        use futures_util::StreamExt as _;
        while let Some((peer_url, _key, result)) = inflight.next().await {
            if matches!(result, Ok(ref r) if r.status().is_success()) {
                winner = Some(peer_url);
                break;
            }
        }
    }
    let elapsed = started.elapsed();

    assert_eq!(
        winner.as_deref(),
        Some(format!("http://{fast}").as_str()),
        "the fast peer should win even though the slow peer was queried first"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "fan-out should not wait on the slow peer; took {elapsed:?}"
    );
}

/// Peers with no address or no scoped lan_key are not queryable and must be
/// skipped — same eligibility rule the sequential loops applied.
#[tokio::test]
async fn ineligible_peers_are_skipped() {
    let mut no_addr = peer_at("127.0.0.1:1");
    no_addr.address = String::new();
    let mut no_key = peer_at("127.0.0.1:2");
    no_key.auth_key = String::new();
    let peers = vec![no_addr, no_key];
    let http = reqwest::Client::new();
    let inflight = query_peers_concurrently(&peers, "agent-x", &http);
    assert_eq!(inflight.len(), 0, "neither peer is eligible to be queried");
}

/// A peer's answer for a slug its agent was renamed from names the current
/// one; an answer for the name itself, or with no `current_name`, doesn't.
#[test]
fn a_renamed_answer_names_the_current_slug() {
    let body = |v: serde_json::Value| v;
    assert_eq!(
        renamed_to("loap-loap-2", &body(serde_json::json!({ "current_name": "loap-2", "lan_public_key": "k" }))),
        Some("loap-2".to_string())
    );
    assert_eq!(renamed_to("loap-2", &body(serde_json::json!({ "current_name": "Loap-2" }))), None);
    assert_eq!(renamed_to("loap-2", &body(serde_json::json!({ "lan_public_key": "k" }))), None);
    assert_eq!(renamed_to("loap-2", &body(serde_json::json!({ "current_name": "  " }))), None);
}

/// A peer that answers `/agentmux/reactive/agent?id=<id>` from `routes`
/// (404 for any other id) and counts every request it gets.
async fn routed_peer(routes: Vec<(&'static str, &'static str)>) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = request.lines().next().unwrap_or("");
                let found = routes.iter().find(|(id, _)| line.contains(&format!("id={id} ")));
                let (status, body) = match found {
                    Some((_, body)) => ("200 OK", *body),
                    None => ("404 Not Found", "{}"),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });
    (format!("127.0.0.1:{}", addr.port()), hits)
}

fn controller() -> LanDiscoveryController {
    LanDiscoveryController::new(
        "self".into(),
        "test".into(),
        "0".into(),
        0,
        Arc::new(crate::backend::eventbus::EventBus::new()),
        "k".into(),
    )
}

fn cached(c: &LanDiscoveryController, name: &str) -> bool {
    c.pubkey_cache.read().unwrap().contains_key(name)
}

/// SECURITY: a peer answering a lookup for `x` with `current_name: victim`
/// and its own key plants nothing, under either name. The asked-for name is
/// reported renamed (it never verifies); a later lookup for `victim` still
/// goes to the network.
#[tokio::test]
async fn a_peer_supplied_current_name_plants_no_key() {
    let (addr, hits) = routed_peer(vec![
        ("anyone", r#"{"current_name":"victim","lan_public_key":"YXR0YWNrZXI=","uid":"u-evil","uid_public_key":"YXR0YWNrZXI="}"#),
    ])
    .await;
    let c = controller();
    let http = reqwest::Client::new();
    let peers = vec![peer_at(&addr)];
    assert!(matches!(c.pubkey_from_peers(&peers, "anyone", &http).await, LanPubkeyLookup::Renamed(n) if n == "victim"));
    assert!(!cached(&c, "victim") && !cached(&c, "anyone"), "nothing cached from a renamed answer");
    assert!(c.cached_lan_uid_key("victim").is_none());

    let before = hits.load(std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(c.pubkey_from_peers(&peers, "victim", &http).await, LanPubkeyLookup::NotFound));
    assert!(hits.load(std::sync::atomic::Ordering::SeqCst) > before, "the lookup for victim asked the network");
}

/// A `current_name` that isn't a valid agent id is ignored: the answer is
/// read as the asked-for name's own.
#[tokio::test]
async fn an_invalid_current_name_is_ignored() {
    let (addr, _) = routed_peer(vec![("agentx", r#"{"current_name":"not an id!","lan_public_key":"a2V5"}"#)]).await;
    let c = controller();
    let found = c.pubkey_from_peers(&[peer_at(&addr)], "agentx", &reqwest::Client::new()).await;
    assert!(matches!(found, LanPubkeyLookup::Found(k) if k == b"key"));
    assert!(cached(&c, "agentx") && !cached(&c, "not an id!"));
}

/// A real rename: the former slug is reported renamed, and the current name
/// resolves through a lookup of its own, cached under that name.
#[tokio::test]
async fn a_renamed_agent_resolves_by_a_lookup_of_its_current_name() {
    let (addr, _) = routed_peer(vec![
        ("loap-loap-2", r#"{"current_name":"loap-2","lan_public_key":"a2V5"}"#),
        ("loap-2", r#"{"lan_public_key":"a2V5"}"#),
    ])
    .await;
    let c = controller();
    let http = reqwest::Client::new();
    let peers = vec![peer_at(&addr)];
    assert!(matches!(c.pubkey_from_peers(&peers, "loap-loap-2", &http).await, LanPubkeyLookup::Renamed(n) if n == "loap-2"));
    assert!(!cached(&c, "loap-2"));
    assert!(matches!(c.pubkey_from_peers(&peers, "loap-2", &http).await, LanPubkeyLookup::Found(k) if k == b"key"));
    assert!(cached(&c, "loap-2") && !cached(&c, "loap-loap-2"));
}

/// A UID key the cache no longer holds is asked for afresh, and only an
/// answer with the same name key counts.
#[tokio::test]
async fn a_refetch_returns_the_uid_key_beside_the_same_name_key() {
    let (addr, _) = routed_peer(vec![("agentx", r#"{"lan_public_key":"a2V5","uid":"u-1","uid_public_key":"dWtleQ=="}"#)]).await;
    let c = controller();
    let http = reqwest::Client::new();
    let peers = vec![peer_at(&addr)];
    assert!(c.cached_lan_uid_key("agentx").is_none(), "nothing cached to start with");
    let got = c.refetch_lan_uid_key_from(&peers, "agentx", b"key", &http).await;
    assert_eq!(got, Some(("u-1".to_string(), b"ukey".to_vec())));
    // Another name key than the one that verified: not the same agent's answer.
    assert!(c.refetch_lan_uid_key_from(&peers, "agentx", b"other", &http).await.is_none());
}

#[tokio::test]
async fn a_refetch_with_no_uid_key_in_the_answer_or_no_peer_gives_none() {
    let (addr, _) = routed_peer(vec![("agentx", r#"{"lan_public_key":"a2V5"}"#)]).await;
    let c = controller();
    let http = reqwest::Client::new();
    assert!(c.refetch_lan_uid_key_from(&[peer_at(&addr)], "agentx", b"key", &http).await.is_none());
    assert!(c.refetch_lan_uid_key_from(&[], "agentx", b"key", &http).await.is_none());
}

/// A refetch that shows nothing must not replace the cached answer that
/// verified the message: not with a miss, not with another peer's key.
#[tokio::test]
async fn a_refetch_that_shows_nothing_leaves_the_cached_answer_as_it_was() {
    let c = controller();
    let http = reqwest::Client::new();
    c.seed_lan_pubkey_for_test("agentx", b"key".to_vec(), None);
    assert!(c.refetch_lan_uid_key_from(&[], "agentx", b"key", &http).await.is_none());
    assert!(matches!(c.find_agent_lan_pubkey("agentx", &http).await, LanPubkeyLookup::Found(k) if k == b"key"), "no miss cached over the key");

    let (addr, _) = routed_peer(vec![("agentx", r#"{"lan_public_key":"b3RoZXI=","uid":"u-1","uid_public_key":"dWtleQ=="}"#)]).await;
    assert!(c.refetch_lan_uid_key_from(&[peer_at(&addr)], "agentx", b"key", &http).await.is_none());
    assert!(matches!(c.find_agent_lan_pubkey("agentx", &http).await, LanPubkeyLookup::Found(k) if k == b"key"), "another peer's key not cached over it");

    // Nothing cached before: nothing cached after.
    assert!(c.refetch_lan_uid_key_from(&[], "agenty", b"key", &http).await.is_none());
    assert!(!cached(&c, "agenty"));
}

/// Peers that could not be reached say nothing about the key: no miss is
/// cached for them, so the next lookup asks again. A peer that answered "not
/// here" is an answer, and is cached.
#[tokio::test]
async fn a_miss_is_cached_only_when_a_peer_answered() {
    let http = reqwest::Client::new();
    let c = controller();
    assert!(matches!(c.pubkey_from_peers(&[], "agentx", &http).await, LanPubkeyLookup::NotFound));
    assert!(!cached(&c, "agentx"), "no peers: nothing learned");
    let dead = peer_at("127.0.0.1:1");
    assert!(matches!(c.pubkey_from_peers(&[dead], "agentx", &http).await, LanPubkeyLookup::NotFound));
    assert!(!cached(&c, "agentx"), "a peer that could not be reached: nothing learned");
    let (addr, _) = routed_peer(vec![]).await;
    assert!(matches!(c.pubkey_from_peers(&[peer_at(&addr)], "agentx", &http).await, LanPubkeyLookup::NotFound));
    assert!(cached(&c, "agentx"), "a peer said it has no such agent");
}

/// A miss served from the cache is asked again once; a miss the lookup just
/// made is not repeated, and a limiter refusal reads as a miss.
#[tokio::test]
async fn an_old_cached_miss_is_asked_again_and_a_fresh_one_is_not() {
    let (addr, hits) = routed_peer(vec![("agentx", r#"{"lan_public_key":"a2V5"}"#)]).await;
    let http = reqwest::Client::new();
    let peers = vec![peer_at(&addr)];
    let c = controller();
    let asked = || hits.load(std::sync::atomic::Ordering::SeqCst);

    assert!(matches!(c.find_agent_lan_pubkey_again_from(&peers, "agentx", &http).await, LanPubkeyLookup::NotFound), "no cached miss");
    c.seed_lan_pubkey_miss_for_test("agentx", 1);
    assert!(matches!(c.find_agent_lan_pubkey_again_from(&peers, "agentx", &http).await, LanPubkeyLookup::NotFound), "a miss made a second ago");
    assert_eq!(asked(), 0, "neither asked the network");

    c.seed_lan_pubkey_miss_for_test("agentx", 30);
    assert!(matches!(c.find_agent_lan_pubkey_again_from(&peers, "agentx", &http).await, LanPubkeyLookup::Found(k) if k == b"key"));
    assert_eq!(asked(), 1);
    assert!(matches!(c.find_agent_lan_pubkey("agentx", &http).await, LanPubkeyLookup::Found(_)), "and the key is cached");
}

/// An old miss that the peers still confirm stays a miss.
#[tokio::test]
async fn an_old_miss_that_is_still_a_miss_stays_one() {
    let (addr, hits) = routed_peer(vec![]).await;
    let http = reqwest::Client::new();
    let c = controller();
    c.seed_lan_pubkey_miss_for_test("agentx", 30);
    assert!(matches!(c.find_agent_lan_pubkey_again_from(&[peer_at(&addr)], "agentx", &http).await, LanPubkeyLookup::NotFound));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1, "asked once");
}
