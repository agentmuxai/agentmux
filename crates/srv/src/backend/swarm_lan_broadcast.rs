// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Swarm broadcast to agents on a LAN peer
//! (`docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md` §4.2).
//!
//! On this machine a broadcast is the user's own turn. On another machine it
//! can't be: the LAN key that delivers it is readable by anything on the
//! network and names an instance, not a person
//! (`SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md` §6). So
//! it goes as a jekt from this instance, signed with this instance's own LAN key
//! under a reserved sender name ([`instance_sender_id`]). The receiving instance
//! checks it as it checks any agent's LAN jekt
//! (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`): it asks the LAN peers for the
//! sender's public key, which this instance's `/agentmux/reactive/agent?id=`
//! answers, pins it on first use, and marks the message `TRUST=lan-verified`.
//! It claims no user authority: normal tier, from this instance, in its own
//! words.

use crate::backend::reactive::types::InjectionRequest;
use crate::backend::swarm_remote::{PeerRoute, RemoteHost};

/// The prefix of a Swarm selection key for an agent on another machine
/// (the frontend's `remoteAgentKey`).
pub const REMOTE_KEY_PREFIX: &str = "remote:";

/// How long a peer gets to take the message.
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// This instance's sender name on the LAN: `swarm-<host>-<hash>`, a valid agent
/// id (letters, digits, `-`, at most 39 characters). The hash of the hostname and
/// channel keeps two instances apart, including two machines with one name, and
/// stays the same across restarts, so a peer's first-use pin of its key holds.
pub fn instance_sender_id(hostname: &str, channel: &str) -> String {
    let host: String = hostname
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(24)
        .collect::<String>()
        .to_lowercase();
    let host = if host.is_empty() { "host".to_string() } else { host };
    let hash = crate::backend::swarm_remote::fnv1a32(&format!("{hostname}\n{channel}"));
    format!("swarm-{host}-{hash:08x}")
}

/// This instance's own sender name.
pub fn this_sender_id(hostname: &str) -> String {
    instance_sender_id(hostname, &crate::backend::reactive::registry::local_channel_id())
}

/// `remote:<host id>/<channel>/<name>` as `(section key, agent name)`, the
/// section key being `<host id>/<channel>`. `None` for any other key.
pub fn parse_remote_key(key: &str) -> Option<(&str, &str)> {
    let (section, name) = key.strip_prefix(REMOTE_KEY_PREFIX)?.rsplit_once('/')?;
    (!section.is_empty() && !name.is_empty()).then_some((section, name))
}

/// The route to the LAN instance a section key names, from the hosts the Swarm
/// lists. `None` when it isn't a LAN instance, isn't listed, or has gone stale.
pub fn route_for(hosts: &[RemoteHost], section_key: &str) -> Option<PeerRoute> {
    hosts
        .iter()
        .filter(|h| h.tier == "lan")
        .flat_map(|h| h.channels.iter().map(move |c| (h, c)))
        .find(|(h, c)| !c.stale && format!("{}/{}", h.host_id, c.channel) == section_key)
        .and_then(|(_, c)| c.route.clone())
}

/// What the receiving agent reads: where it came from and that it went to every
/// agent selected, then the message.
pub fn broadcast_text(hostname: &str, recipients: usize, message: &str) -> String {
    let to = if recipients == 1 { "1 agent".to_string() } else { format!("{recipients} agents") };
    format!("Swarm broadcast from {hostname}, sent by its operator to {to}:\n\n{message}")
}

/// The signed request for one agent. `None` when the private key is unusable.
pub fn signed_request(sender_id: &str, private_key_b64: &str, target: &str, text: &str, msg_id: &str, ts_secs: i64) -> Option<InjectionRequest> {
    let key = agentmux_common::jekt_sign::decode_key(private_key_b64)?;
    let sig = agentmux_common::jekt_sign::sign_lan_jekt(&key, msg_id, sender_id, target, ts_secs, text)?;
    Some(InjectionRequest {
        target_agent: target.to_string(),
        message: text.to_string(),
        source_agent: Some(sender_id.to_string()),
        request_id: Some(msg_id.to_string()),
        ts_secs: Some(ts_secs),
        lan_sig: Some(sig),
        ..Default::default()
    })
}

/// Sends one signed request to the peer and says whether it delivered, with the
/// peer's reason when it didn't.
pub async fn send(http: &reqwest::Client, route: &PeerRoute, req: &InjectionRequest) -> Result<(), String> {
    let resp = http
        .post(format!("{}/agentmux/reactive/inject", route.url))
        .header(agentmux_common::AUTH_KEY_HEADER, &route.auth_key)
        .timeout(SEND_TIMEOUT)
        .json(req)
        .send()
        .await
        .map_err(|e| format!("couldn't reach that machine: {e}"))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    if status.is_success() && body.get("success").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(());
    }
    let reason = body
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("that machine answered {status}"));
    Err(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::swarm_remote::RemoteChannel;

    #[test]
    fn the_sender_id_is_a_valid_agent_id_and_stable() {
        let id = instance_sender_id("Area54 (office)!", "local-main-b28b7a-1713be7f");
        assert!(crate::backend::reactive::sanitize::validate_agent_id(&id), "{id}");
        assert!(id.starts_with("swarm-area54office-"), "{id}");
        assert_eq!(id, instance_sender_id("Area54 (office)!", "local-main-b28b7a-1713be7f"));
        // Another channel, or another machine of the same name, is another sender.
        assert_ne!(id, instance_sender_id("Area54 (office)!", "stable"));
        let long = instance_sender_id(&"x".repeat(200), "stable");
        assert!(long.len() <= 64 && crate::backend::reactive::sanitize::validate_agent_id(&long), "{long}");
        assert!(instance_sender_id("", "stable").starts_with("swarm-host-"));
    }

    #[test]
    fn a_remote_key_splits_into_its_section_and_name() {
        assert_eq!(parse_remote_key("remote:lan:area54#1a2b3c4d/stable/Manoz"), Some(("lan:area54#1a2b3c4d/stable", "Manoz")));
        assert_eq!(parse_remote_key("blk-123"), None);
        assert_eq!(parse_remote_key("remote:nosep"), None);
        assert_eq!(parse_remote_key("remote:lan:x/stable/"), None);
    }

    fn host(tier: &'static str, channel: &str, stale: bool) -> RemoteHost {
        RemoteHost {
            host_id: "lan:area54#1".into(),
            display_name: "Area54".into(),
            tier,
            channels: vec![RemoteChannel {
                channel: channel.into(),
                seen_at_ms: 1,
                stale,
                os: String::new(),
                agents: vec![],
                route: Some(PeerRoute { url: "http://192.0.2.7:29700".into(), auth_key: "k".into() }),
            }],
        }
    }

    #[test]
    fn a_route_is_found_only_for_a_listed_fresh_lan_instance() {
        let hosts = [host("lan", "stable", false)];
        assert_eq!(route_for(&hosts, "lan:area54#1/stable").map(|r| r.url), Some("http://192.0.2.7:29700".to_string()));
        assert!(route_for(&hosts, "lan:area54#1/dev").is_none());
        assert!(route_for(&[host("lan", "stable", true)], "lan:area54#1/stable").is_none(), "stale");
        assert!(route_for(&[host("host", "stable", false)], "lan:area54#1/stable").is_none(), "not LAN");
    }

    #[test]
    fn the_route_never_prints_its_key() {
        let route = PeerRoute { url: "http://192.0.2.7:29700".into(), auth_key: "lan-secret".into() };
        assert!(!format!("{route:?}").contains("lan-secret"));
    }

    #[test]
    fn the_request_is_signed_so_the_receiver_can_verify_it() {
        let (public, seed) = agentmux_common::jekt_sign::generate_lan_keypair([7u8; 32]);
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        let text = broadcast_text("Area54", 2, "merge on approval");
        let req = signed_request("swarm-area54-0000abcd", &BASE64.encode(seed), "Manoz", &text, "msg-1", 1_000).unwrap();
        assert_eq!(req.source_agent.as_deref(), Some("swarm-area54-0000abcd"));
        assert!(req.delivery_tier.is_none(), "the receiver sets the tier from how the request authenticated");
        assert!(agentmux_common::jekt_sign::verify_lan_jekt(
            &public,
            "msg-1",
            "swarm-area54-0000abcd",
            "Manoz",
            1_000,
            &text,
            req.lan_sig.as_deref().unwrap(),
        ));
        assert!(signed_request("s", "not base64!", "Manoz", &text, "m", 1).is_none());
    }

    #[test]
    fn the_text_says_where_it_came_from_and_how_many_got_it() {
        assert_eq!(
            broadcast_text("Area54", 3, "stop and commit"),
            "Swarm broadcast from Area54, sent by its operator to 3 agents:\n\nstop and commit"
        );
        assert!(broadcast_text("Area54", 1, "x").contains("to 1 agent:"));
    }
}
