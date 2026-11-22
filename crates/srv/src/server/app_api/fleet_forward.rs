// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The receiving end of a Swarm broadcast forwarded between channels of one install, and the
//! confirmation that decides how it is delivered.
//!
//! A forward is the person's own broadcast (the user-voice header, `MessageOrigin::broadcast()`)
//! only when another instance on this machine confirms it sent that forward from the operator's
//! window. Any other forward is delivered as an ordinary agent message from `swarm-forward`,
//! labelled self-declared. The sending instance keeps what it is about to forward in memory
//! (`vouch_for_forward`, called only by `fleet_broadcast_impl`, which only the operator's window
//! reaches) and answers a confirmation once, for exactly that forward; a confirmation carries the
//! body's SHA-256, never the words. An answer counts only when the listener that gives it is an
//! AgentMux instance: one held by a process inside an agent's process tree, or one the system
//! cannot identify, does not count and the forward is delivered as an ordinary agent message, with
//! the audit saying which case it was.

use agentmux_common::AUTH_KEY_HEADER;

use crate::backend::message_origin::MessageOrigin;
use crate::backend::reactive::broadcast_turn_message;

use super::fleet::{deliver_broadcast_turn, BroadcastForward, FLEET_BROADCAST_AUDIT_ACTION, NO_REGISTERED_AGENT};
use super::AppState;
use crate::server::agent_handlers::AgentTurnDeps;

/// The broadcasts this instance has sent on to another channel and not yet had confirmed.
static VOUCHED_FORWARDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), VouchedForward>>> = std::sync::OnceLock::new();
const VOUCH_FOR: std::time::Duration = std::time::Duration::from_secs(120);

struct VouchedForward {
    body_sha256: String,
    recipients: usize,
    until: std::time::Instant,
}

/// The SHA-256 of a forwarded body, in hex: what a confirmation carries instead of the words themselves.
pub(crate) fn body_digest(body: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn vouched() -> std::sync::MutexGuard<'static, std::collections::HashMap<(String, String), VouchedForward>> {
    VOUCHED_FORWARDS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Records a forward this instance is about to send (called only for a real operator broadcast).
pub(super) fn vouch_for_forward(req: &BroadcastForward<'_>) {
    let now = std::time::Instant::now();
    let mut table = vouched();
    table.retain(|_, v| v.until > now);
    table.insert(
        (req.msg_id.to_string(), req.block_id.to_string()),
        VouchedForward { body_sha256: body_digest(req.body), recipients: req.recipients, until: now + VOUCH_FOR },
    );
}

/// Whether this instance sent exactly this forward (the body given by its SHA-256): true once, and the entry is spent.
pub(crate) fn confirm_forward(msg_id: &str, block_id: &str, body_sha256: &str, recipients: usize) -> bool {
    let now = std::time::Instant::now();
    let mut table = vouched();
    table.retain(|_, v| v.until > now);
    match table.get(&(msg_id.to_string(), block_id.to_string())) {
        Some(v) if v.body_sha256 == body_sha256 && v.recipients == recipients => {
            table.remove(&(msg_id.to_string(), block_id.to_string()));
            true
        }
        _ => false,
    }
}

/// Who holds the listener that would answer a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Standing {
    /// The listener's processes (all of them), none inside an agent's process tree: an AgentMux instance.
    Instance(Vec<u32>),
    /// A process of this agent's tree listens there: its answer does not count.
    InsideAgent(String),
    /// The lookup could not say (not supported, the table could not be read, the process tracker is not running, or the
    /// address is not an IPv4 loopback address).
    Unknown,
}

/// The standing of the listener at `url`. It reads the process table, so it runs off the async threads.
async fn listener_standing(state: &AppState, url: &str) -> Standing {
    let Some(addr) = loopback_addr(url) else { return Standing::Unknown };
    #[cfg(test)]
    if let Some(standing) = hook::standing_of(addr) {
        return standing;
    }
    let state = state.clone();
    tokio::task::spawn_blocking(move || listener_standing_now(&state, addr)).await.unwrap_or(Standing::Unknown)
}

fn listener_standing_now(state: &AppState, addr: std::net::SocketAddr) -> Standing {
    let Some(pids) = crate::server::peer_process::listeners_at(addr) else { return Standing::Unknown };
    let Some(registry) = crate::backend::process_tracker::registry::global() else { return Standing::Unknown };
    for pid in &pids {
        if let Some(block) = registry.agent_block_holding(&crate::server::confirm_peer::with_ancestors(*pid)) {
            let agent = crate::identity::resolver::block_agent_id(&state.mstore, &block).unwrap_or(block);
            return Standing::InsideAgent(agent);
        }
    }
    Standing::Instance(pids)
}

/// `http://127.0.0.1:<port>` (or `localhost`) as an address; anything else is not looked up.
fn loopback_addr(url: &str) -> Option<std::net::SocketAddr> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let ip = match parsed.host_str()? {
        "localhost" => std::net::Ipv4Addr::LOCALHOST,
        host => host.parse::<std::net::Ipv4Addr>().ok().filter(|ip| ip.is_loopback())?,
    };
    Some(std::net::SocketAddr::new(ip.into(), parsed.port_or_known_default()?))
}

/// What asking the other instances found, and so how the forward is delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForwardProof {
    /// An instance that is not inside an agent's process tree confirmed it sent this forward.
    Confirmed,
    /// No instance confirmed it.
    NoInstanceConfirmed,
    /// An answer came from a process inside this agent's process tree: it does not count.
    AnsweredFromAgent(String),
    /// The listener could not be identified, so its answer was not asked for or does not count.
    ListenerNotIdentified,
}

impl ForwardProof {
    /// How the audit says the forward was delivered.
    fn how(&self) -> String {
        match self {
            ForwardProof::Confirmed => "forwarded from another channel on this machine".to_string(),
            ForwardProof::NoInstanceConfirmed => "forward no other instance confirmed sending: delivered as an agent message, not as the person".to_string(),
            ForwardProof::AnsweredFromAgent(agent) => format!("a listener inside agent:{agent}'s process tree answered, which does not count: delivered as an agent message, not as the person"),
            ForwardProof::ListenerNotIdentified => "the instance that answered could not be identified: delivered as an agent message, not as the person".to_string(),
        }
    }

    /// Which finding is kept when several instances were asked: an answer from inside an agent over an unidentified listener
    /// over nothing.
    fn weight(&self) -> u8 {
        match self {
            ForwardProof::Confirmed => 3,
            ForwardProof::AnsweredFromAgent(_) => 2,
            ForwardProof::ListenerNotIdentified => 1,
            ForwardProof::NoInstanceConfirmed => 0,
        }
    }
}

/// Asks one instance whether it sent this forward. Its answer counts only when the listener that gives it is an AgentMux
/// instance and not a process inside an agent's process tree: that is checked before the question is sent, and again on the
/// answer (the same processes must still hold the listener).
async fn ask_instance(state: &AppState, url: &str, auth_key: &str, block_id: &str, body: &str, msg_id: &str, recipients: usize) -> ForwardProof {
    let before = match listener_standing(state, url).await {
        Standing::Instance(pids) => pids,
        Standing::InsideAgent(agent) => return ForwardProof::AnsweredFromAgent(agent),
        Standing::Unknown => return ForwardProof::ListenerNotIdentified,
    };
    let mut http = state
        .http_client
        .post(format!("{url}/agentmux/agent/broadcast/confirm"))
        .timeout(std::time::Duration::from_secs(3))
        // The digest, not the words: an instance asked here is not necessarily one of ours.
        .json(&serde_json::json!({ "block_id": block_id, "body_sha256": body_digest(body), "msg_id": msg_id, "recipients": recipients }));
    if !auth_key.is_empty() {
        http = http.header(AUTH_KEY_HEADER, auth_key);
    }
    let Ok(resp) = http.send().await else { return ForwardProof::NoInstanceConfirmed };
    let Ok(answer) = resp.json::<serde_json::Value>().await else { return ForwardProof::NoInstanceConfirmed };
    if answer.get("confirmed").and_then(|v| v.as_bool()) != Some(true) {
        return ForwardProof::NoInstanceConfirmed;
    }
    match listener_standing(state, url).await {
        Standing::Instance(after) if after == before => ForwardProof::Confirmed,
        Standing::InsideAgent(agent) => ForwardProof::AnsweredFromAgent(agent),
        _ => ForwardProof::ListenerNotIdentified,
    }
}

/// Asks the other instances on this machine (the shared registry's loopback entries, never this one)
/// whether one of them sent this forward. The channel that did answers yes, once.
async fn forward_confirmed_by_a_sender(state: &AppState, block_id: &str, body: &str, msg_id: &str, recipients: usize) -> ForwardProof {
    let Some(dir) = crate::registry::resolve_shared_reactive_dir() else { return ForwardProof::NoInstanceConfirmed };
    let mut asked = std::collections::HashSet::new();
    let mut found = ForwardProof::NoInstanceConfirmed;
    for entry in crate::backend::reactive::registry::list_all_shared(&dir) {
        let url = entry.local_url.clone();
        let loopback = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost") || url.starts_with("http://[::1]");
        if !loopback || url == state.local_web_url || !asked.insert(url.clone()) {
            continue;
        }
        let proof = ask_instance(state, &url, &entry.auth_key, block_id, body, msg_id, recipients).await;
        if proof == ForwardProof::Confirmed {
            return proof;
        }
        if proof.weight() > found.weight() {
            found = proof;
        }
    }
    found
}

/// The receiving end of `forward_broadcast_to_channel`. A forward that another instance of this
/// machine confirms it sent (from the operator's window) is delivered as the person's broadcast: the
/// header is built here, from the target's own agent id, as any broadcast turn
/// (`MessageOrigin::broadcast()`). Any other forward is delivered as an ordinary agent message from
/// `swarm-forward`, labelled self-declared, never as the person. Audited on this side too, naming
/// which of the two it was.
pub(crate) async fn deliver_forwarded_broadcast(
    state: &AppState,
    block_id: &str,
    body: &str,
    msg_id: &str,
    recipients: usize,
) -> Result<(), String> {
    let proof = if state.reactive_handler.get_agent_by_block(block_id).is_some() {
        forward_confirmed_by_a_sender(state, block_id, body, msg_id, recipients).await
    } else {
        ForwardProof::NoInstanceConfirmed
    };
    deliver_forward(state, block_id, body, msg_id, recipients, proof).await.1
}

/// How a forwarded broadcast was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardedAs {
    /// The person's own broadcast: the sending instance confirmed it came from the operator's window.
    Person,
    /// An ordinary agent message from `swarm-forward`.
    AgentMessage,
}

async fn deliver_forward(
    state: &AppState,
    block_id: &str,
    body: &str,
    msg_id: &str,
    recipients: usize,
    proof: ForwardProof,
) -> (ForwardedAs, Result<(), String>) {
    let how = proof.how();
    let mut kind = ForwardedAs::Person;
    let result = match state.reactive_handler.get_agent_by_block(block_id) {
        None => Err(NO_REGISTERED_AGENT.to_string()),
        Some(agent) if proof == ForwardProof::Confirmed => {
            let text = broadcast_turn_message(&agent.agent_id, recipients, msg_id, body);
            let deps = AgentTurnDeps::from_state(state);
            deliver_broadcast_turn(&deps, block_id.to_string(), text, MessageOrigin::broadcast()).await
        }
        Some(agent) => {
            kind = ForwardedAs::AgentMessage;
            deliver_unconfirmed_forward(state, &agent.agent_id, body).await
        }
    };
    let target = state
        .reactive_handler
        .get_agent_by_block(block_id)
        .map(|a| a.agent_id)
        .unwrap_or_else(|| block_id.to_string());
    state.reactive_handler.log_fleet_action_audit(
        None,
        &target,
        block_id,
        FLEET_BROADCAST_AUDIT_ACTION,
        result.is_ok(),
        result.as_ref().err().map(String::as_str),
        msg_id,
        Some(how.as_str()),
    );
    (kind, result)
}

/// A forward nobody confirmed: an ordinary message to the agent from the sender name `swarm-forward`,
/// through the same delivery as any unattributed one (labelled self-declared, never host-verified).
async fn deliver_unconfirmed_forward(state: &AppState, agent_id: &str, body: &str) -> Result<(), String> {
    let req = crate::backend::reactive::InjectionRequest {
        target_agent: agent_id.to_string(),
        message: format!("A Swarm broadcast was forwarded to you but not proven to come from the person's window, so it is delivered as an ordinary message:\n{body}"),
        source_agent: Some("swarm-forward".to_string()),
        reply: Some("none".to_string()),
        ..Default::default()
    };
    let answer = crate::server::reactive::deliver(state, crate::server::ReactiveAuthVia::FullAuthKey, "", req).await;
    if answer.get("success").and_then(|v| v.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(answer.get("error").and_then(|v| v.as_str()).unwrap_or("not delivered").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_is_confirmed_once_and_only_as_it_was_sent() {
        let f = BroadcastForward { block_id: "blk-confirm", body: "ship it", msg_id: "m-confirm", recipients: 2 };
        let digest = body_digest("ship it");
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("ship"), "the words are not in what is sent to be confirmed");
        assert!(!confirm_forward("m-confirm", "blk-confirm", &digest, 2), "nothing was sent yet");
        vouch_for_forward(&f);
        assert!(!confirm_forward("m-confirm", "blk-confirm", &body_digest("something else"), 2), "another body is not that forward");
        assert!(!confirm_forward("m-confirm", "blk-confirm", &digest, 9), "another count is not that forward");
        assert!(!confirm_forward("m-confirm", "blk-other", &digest, 2), "another block is not that forward");
        assert!(confirm_forward("m-confirm", "blk-confirm", &digest, 2));
        assert!(!confirm_forward("m-confirm", "blk-confirm", &digest, 2), "a confirmation is spent");
    }

    #[tokio::test]
    async fn only_a_confirmed_forward_arrives_as_the_person() {
        let state = crate::server::tests::test_state();
        state.reactive_handler.register_agent("fwd-target", "blk-fwd-target", None).unwrap();
        let (kind, _) = deliver_forward(&state, "blk-fwd-target", "hi", "m-2", 1, ForwardProof::NoInstanceConfirmed).await;
        assert_eq!(kind, ForwardedAs::AgentMessage, "no instance vouched for it");
        let (kind, _) = deliver_forward(&state, "blk-fwd-target", "hi", "m-2", 1, ForwardProof::AnsweredFromAgent("some-agent".into())).await;
        assert_eq!(kind, ForwardedAs::AgentMessage, "an answer from inside an agent's tree does not count");
        let (kind, _) = deliver_forward(&state, "blk-fwd-target", "hi", "m-2", 1, ForwardProof::ListenerNotIdentified).await;
        assert_eq!(kind, ForwardedAs::AgentMessage, "a failed lookup falls back to an ordinary message");
        let (kind, _) = deliver_forward(&state, "blk-fwd-target", "hi", "m-2", 1, ForwardProof::Confirmed).await;
        assert_eq!(kind, ForwardedAs::Person);
    }

    #[test]
    fn each_case_is_recorded_as_what_it_was() {
        let hows: Vec<String> = [
            ForwardProof::Confirmed,
            ForwardProof::NoInstanceConfirmed,
            ForwardProof::AnsweredFromAgent("some-agent".into()),
            ForwardProof::ListenerNotIdentified,
        ]
        .iter()
        .map(ForwardProof::how)
        .collect();
        for (i, a) in hows.iter().enumerate() {
            for b in &hows[i + 1..] {
                assert_ne!(a, b, "two cases are recorded the same way");
            }
        }
        assert!(hows[2].contains("some-agent"), "the agent whose tree held the listener is named");
    }

    /// A listener that answers every confirmation "yes", and how many times it was asked.
    async fn confirming_listener() -> (String, std::net::SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = asked.clone();
        let app = axum::Router::new().route(
            "/agentmux/agent/broadcast/confirm",
            axum::routing::post(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::Json(serde_json::json!({ "confirmed": true }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        (format!("http://{addr}"), addr, asked)
    }

    #[tokio::test]
    async fn an_answer_from_a_real_second_instance_counts() {
        let state = crate::server::tests::test_state();
        let (url, addr, asked) = confirming_listener().await;
        hook::set(addr, Standing::Instance(vec![4242]));
        assert_eq!(ask_instance(&state, &url, "", "blk", "hi", "m-3", 1).await, ForwardProof::Confirmed);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_answer_from_a_listener_inside_an_agents_tree_does_not_count() {
        let state = crate::server::tests::test_state();
        let (url, addr, asked) = confirming_listener().await;
        hook::set(addr, Standing::InsideAgent("some-agent".into()));
        assert_eq!(ask_instance(&state, &url, "", "blk", "hi", "m-4", 1).await, ForwardProof::AnsweredFromAgent("some-agent".into()));
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0, "the question is not even sent to it");
    }

    #[tokio::test]
    async fn a_failed_lookup_does_not_count_and_nothing_is_sent() {
        let state = crate::server::tests::test_state();
        let (url, addr, asked) = confirming_listener().await;
        hook::set(addr, Standing::Unknown);
        assert_eq!(ask_instance(&state, &url, "", "blk", "hi", "m-5", 1).await, ForwardProof::ListenerNotIdentified);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_listener_that_changes_hands_during_the_question_does_not_count() {
        let state = crate::server::tests::test_state();
        let (url, addr, _) = confirming_listener().await;
        hook::set_sequence(addr, vec![Standing::Instance(vec![1]), Standing::Instance(vec![2])]);
        assert_eq!(ask_instance(&state, &url, "", "blk", "hi", "m-6", 1).await, ForwardProof::ListenerNotIdentified);
        hook::set_sequence(addr, vec![Standing::Instance(vec![1]), Standing::InsideAgent("some-agent".into())]);
        assert_eq!(ask_instance(&state, &url, "", "blk", "hi", "m-6", 1).await, ForwardProof::AnsweredFromAgent("some-agent".into()));
    }

    #[test]
    fn only_an_ipv4_loopback_url_is_looked_up() {
        assert_eq!(loopback_addr("http://127.0.0.1:7380"), Some("127.0.0.1:7380".parse().unwrap()));
        assert_eq!(loopback_addr("http://localhost:7380"), Some("127.0.0.1:7380".parse().unwrap()));
        assert_eq!(loopback_addr("http://[::1]:7380"), None);
        assert_eq!(loopback_addr("http://203.0.113.9:7380"), None);
        assert_eq!(loopback_addr("not a url"), None);
    }
}

/// What a test makes the standing of a listener: no process lookup, no tracker.
#[cfg(test)]
mod hook {
    use super::Standing;
    use std::collections::{HashMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::Mutex;

    static BY_ADDR: Mutex<Option<HashMap<SocketAddr, VecDeque<Standing>>>> = Mutex::new(None);

    pub(super) fn set(addr: SocketAddr, standing: Standing) {
        set_sequence(addr, vec![standing]);
    }

    /// The standing returned by each lookup in turn; the last one stays.
    pub(super) fn set_sequence(addr: SocketAddr, standing: Vec<Standing>) {
        BY_ADDR.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(addr, standing.into());
    }

    pub(super) fn standing_of(addr: SocketAddr) -> Option<Standing> {
        let mut all = BY_ADDR.lock().unwrap_or_else(|e| e.into_inner());
        let queue = all.as_mut()?.get_mut(&addr)?;
        if queue.len() > 1 { queue.pop_front() } else { queue.front().cloned() }
    }
}
