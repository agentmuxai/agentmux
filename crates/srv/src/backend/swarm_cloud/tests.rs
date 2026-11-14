// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::muxbus::cloud_installs::CloudState;
use crate::muxbus::install_presence::{InstallPresence, PresenceAgent, PRESENCE_VERSION_LINES, PRESENCE_VERSION_V1};

const NOW: u64 = 10_000_000;

fn install(seed: u8, host: &str, channel: &str, v: u32, age_ms: u64) -> CloudInstall {
    let agent = |name: &str, kind: &str, state: &str, line: Option<&str>| PresenceAgent {
        name: name.into(),
        kind: kind.into(),
        state: Some(state.into()),
        line: line.map(Into::into),
    };
    let agents = [agent("Opaz", "host", "working", Some("Fixing the race")), agent("Lark", "container", "idle", None)];
    let record = InstallPresence::sign_version(v, &[seed; 32], host, channel, "linux", "0.60.1", 1, agents, 1).unwrap();
    CloudInstall { record, seen_at_ms: NOW - age_ms }
}

fn peer(host: &str, channel: &str) -> LanInstance {
    serde_json::from_value(serde_json::json!({
        "instance_id": "area54-29700", "hostname": host, "version": "0", "channel": channel,
        "address": "10.0.0.26", "port": 29700, "agents": [], "first_seen": 0, "last_seen": 0, "other_ttl_secs": 4500,
    }))
    .unwrap()
}

fn listed(installs: &[CloudInstall], own: Option<&str>, lan: &[LanInstance]) -> Vec<RemoteHost> {
    cloud_hosts(installs, own, "narko", &["stable".to_string(), "dev".to_string()], lan, NOW)
}

#[test]
fn a_record_is_one_host_with_one_channel_and_no_route() {
    let hosts = listed(&[install(7, "Charlie", "stable", PRESENCE_VERSION_LINES, 5_000)], None, &[]);
    assert_eq!(hosts.len(), 1);
    let host = &hosts[0];
    assert_eq!((host.tier, host.display_name.as_str(), host.channels.len()), ("cloud", "Charlie", 1));
    assert!(host.host_id.starts_with("cloud:charlie#"), "{}", host.host_id);
    let channel = &host.channels[0];
    assert_eq!((channel.channel.as_str(), channel.os.as_str(), channel.stale), ("stable", "linux", false));
    assert!(channel.route.is_none(), "a cloud channel cannot be addressed from here");
    let got: Vec<(&str, Option<AgentState>, Option<&str>, Option<RuntimeKind>)> =
        channel.agents.iter().map(|a| (a.name.as_str(), a.status, a.line.as_deref(), a.runtime)).collect();
    assert_eq!(
        got,
        [
            ("Lark", Some(AgentState::Idle), None, Some(RuntimeKind::Container)),
            ("Opaz", Some(AgentState::Working), Some("Fixing the race"), Some(RuntimeKind::Host)),
        ]
    );
    assert!(channel.agents.iter().all(|a| a.provider.is_none() && a.seat.is_none() && a.account.is_none()));
}

#[test]
fn an_old_record_says_no_state() {
    let hosts = listed(&[install(7, "starpower", "stable", PRESENCE_VERSION_V1, 0)], None, &[]);
    assert!(hosts[0].channels[0].agents.iter().all(|a| a.status.is_none() && a.line.is_none()));
}

#[test]
fn a_machine_is_not_listed_twice() {
    let own = install(7, "narko", "stable", PRESENCE_VERSION_LINES, 0);
    let other_channel_here = install(8, "NARKO", "dev", PRESENCE_VERSION_LINES, 0);
    let on_the_lan = install(9, "Area54", "stable", PRESENCE_VERSION_LINES, 0);
    let elsewhere = install(10, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    let same_name_other_channel = install(11, "Area54", "dev", PRESENCE_VERSION_LINES, 0);
    let all = [own.clone(), other_channel_here.clone(), on_the_lan, elsewhere, same_name_other_channel];
    let lan = [peer("area54", "STABLE")];
    let got = listed(&all, Some(&own.record.instance_id), &lan);
    let names: Vec<(&str, &str)> = got.iter().map(|h| (h.display_name.as_str(), h.channels[0].channel.as_str())).collect();
    // This install's own record, this machine's other channel and the LAN peer (matched by host and channel,
    // whatever the case) are left out; the same host's other channel, which the LAN does not list, stays.
    assert_eq!(names, [("Area54", "dev"), ("charlie", "stable")]);
    // This machine's own channel is also its own when the id is not known (the fallback): host and channel.
    assert!(listed(&[other_channel_here], None, &[]).is_empty());
}

#[test]
fn a_peer_that_never_said_its_channel_hides_nothing() {
    let hosts = listed(&[install(7, "area54", "stable", PRESENCE_VERSION_LINES, 0)], None, &[peer("area54", "")]);
    assert_eq!(hosts.len(), 1);
}

#[test]
fn a_record_goes_stale_after_three_minutes_and_is_hidden_after_ten() {
    let at = |age: u64| listed(&[install(7, "charlie", "stable", PRESENCE_VERSION_LINES, age)], None, &[]);
    assert!(!at(STALE_AFTER_MS).first().unwrap().channels[0].stale);
    assert!(at(STALE_AFTER_MS + 1)[0].channels[0].stale);
    assert_eq!(at(HIDE_AFTER_MS).len(), 1);
    assert!(at(HIDE_AFTER_MS + 1).is_empty());
}

#[test]
fn the_answer_carries_no_id_key_signature_or_address() {
    let i = install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    let json = serde_json::to_string(&listed(std::slice::from_ref(&i), None, &[])).unwrap();
    for secret in [&i.record.instance_id, &i.record.instance_public_key, &i.record.sig] {
        assert!(!json.contains(secret.as_str()), "{json}");
    }
}

fn snapshot() -> SwarmOtherInstancesResult {
    SwarmOtherInstancesResult { hostname: "narko".into(), channel: "stable".into(), hosts: Vec::new(), cloud: None }
}

#[test]
fn the_answer_says_how_the_cloud_read_went() {
    let installs = vec![install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0)];
    let mut off = snapshot();
    add_cloud(&mut off, None, None, &[], NOW);
    assert_eq!((off.cloud, off.hosts.len()), (Some("off"), 0));

    let mut ok = snapshot();
    add_cloud(&mut ok, Some(&CloudView { installs: installs.clone(), state: CloudState::Ok }), None, &[], NOW);
    assert_eq!((ok.cloud, ok.hosts.len()), (Some("ok"), 1));
    assert_eq!(serde_json::to_value(&ok).unwrap()["cloud"], "ok");

    let mut down = snapshot();
    add_cloud(&mut down, Some(&CloudView { installs: Vec::new(), state: CloudState::Unavailable }), None, &[], NOW);
    assert_eq!((down.cloud, down.hosts.len()), (Some("unavailable"), 0));
    // Before the cloud was asked there is no field at all.
    assert!(serde_json::to_value(snapshot()).unwrap().get("cloud").is_none());
}

#[test]
fn another_channel_the_registry_lists_is_not_a_cloud_machine() {
    let mut snap = snapshot();
    snap.hosts.push(RemoteHost {
        host_id: "host:narko".into(),
        display_name: "narko".into(),
        tier: "host",
        channels: vec![RemoteChannel { channel: "dev-fix".into(), seen_at_ms: NOW, stale: false, os: String::new(), agents: Vec::new(), route: None }],
    });
    let installs = vec![install(7, "narko", "dev-fix", PRESENCE_VERSION_LINES, 0), install(8, "narko", "unlisted", PRESENCE_VERSION_LINES, 0)];
    add_cloud(&mut snap, Some(&CloudView { installs, state: CloudState::Ok }), None, &[], NOW);
    let cloud: Vec<&str> = snap.hosts.iter().filter(|h| h.tier == "cloud").map(|h| h.channels[0].channel.as_str()).collect();
    assert_eq!(cloud, ["unlisted"]);
}

fn with_agent_state(seed: u8, state: &str, age_ms: u64) -> CloudInstall {
    let mut c = install(seed, "charlie", "stable", PRESENCE_VERSION_LINES, age_ms);
    c.record.agents.iter_mut().find(|a| a.name == "Opaz").unwrap().state = Some(state.into());
    c
}

#[test]
fn an_agent_has_the_state_its_newest_shown_record_gives() {
    let view = |installs| CloudView { installs, state: CloudState::Ok };
    let one = view(vec![install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0)]);
    assert_eq!(agent_state(&one, "opaz", None, NOW), Some("working"), "by name, whatever the case");
    assert_eq!(agent_state(&one, "LARK", None, NOW), Some("idle"));
    assert_eq!(agent_state(&one, "nobody", None, NOW), None);
    let old = view(vec![install(8, "charlie", "stable", PRESENCE_VERSION_V1, 0)]);
    assert_eq!(agent_state(&old, "Opaz", None, NOW), None, "a record from before states has none");

    // The same name on two machines: the newer record's state.
    let two = view(vec![with_agent_state(1, "idle", 90_000), with_agent_state(2, "waiting", 10_000)]);
    assert_eq!(agent_state(&two, "Opaz", None, NOW), Some("waiting"));
}

#[test]
fn a_record_the_swarm_does_not_show_gives_no_state() {
    let hidden = CloudView { installs: vec![with_agent_state(1, "working", HIDE_AFTER_MS + 1)], state: CloudState::Ok };
    assert_eq!(agent_state(&hidden, "Opaz", None, NOW), None, "too old");
    let own = with_agent_state(2, "working", 0);
    let id = own.record.instance_id.clone();
    let view = CloudView { installs: vec![own], state: CloudState::Ok };
    assert_eq!(agent_state(&view, "Opaz", Some(&id), NOW), None, "this install's own record");
    assert_eq!(agent_state(&view, "Opaz", None, NOW), Some("working"));
}

fn peer_with_id(host: &str, channel: &str, install_id: &str) -> LanInstance {
    let mut p = peer(host, channel);
    p.install_id = install_id.to_string();
    p
}

#[test]
fn a_lan_peer_with_the_records_id_hides_that_machines_row() {
    let record = install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    let id = record.record.instance_id.clone();
    assert!(listed(std::slice::from_ref(&record), None, &[peer_with_id("charlie", "stable", &id)]).is_empty());
}

#[test]
fn two_machines_with_one_name_and_channel_are_told_apart_by_id() {
    let on_the_lan = install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    let elsewhere = install(8, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    let lan = [peer_with_id("charlie", "stable", &on_the_lan.record.instance_id)];
    let got = listed(&[on_the_lan, elsewhere.clone()], None, &lan);
    assert_eq!(got.len(), 1, "only the machine the LAN lists is left out");
    assert_eq!(got[0].host_id, format!("cloud:charlie#{:08x}", fnv1a32(&elsewhere.record.instance_id)));
}

#[test]
fn a_claimed_id_alone_hides_nothing() {
    let record = install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    let id = record.record.instance_id.clone();
    // Another host name, another channel, or no channel: the id is a public identifier the peer chose, not proof.
    for lan in [peer_with_id("area54", "stable", &id), peer_with_id("charlie", "dev", &id), peer_with_id("charlie", "", &id)] {
        assert_eq!(listed(std::slice::from_ref(&record), None, &[lan]).len(), 1);
    }
}

#[test]
fn a_peer_with_no_id_is_matched_by_host_and_channel_as_before() {
    let record = install(7, "charlie", "stable", PRESENCE_VERSION_LINES, 0);
    assert!(listed(std::slice::from_ref(&record), None, &[peer("charlie", "stable")]).is_empty());
}
