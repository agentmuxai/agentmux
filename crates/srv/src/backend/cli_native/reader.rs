// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One agent pane's view of CLI-native messages: watches the frames its CLI
//! writes, audits each message from whichever end this pane is, and says which
//! frames the pane's transcript must not carry.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use super::frame::{self, Lifecycle, PeerFrame, Replay};
use super::sender::{self, ProcView, Sender};

/// Frames and messages remembered per pane.
const REMEMBERED: usize = 256;

/// Whether the transcript carries a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Keep,
    /// The CLI replayed it only because of `--replay-user-messages`: the human's own
    /// prompt (recorded by srv and the pane already) or a message handled here.
    Drop,
}

/// One state of one message, as seen from this pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The pane that saw it.
    pub block_id: String,
    pub msg_id: String,
    /// `sent` (the CLI took it into the other session's inbox), `held` (the
    /// receiving CLI kept it from its model), `read` (the receiving CLI handed it
    /// to its model), `done` (the turn it started ended).
    pub state: &'static str,
    /// `sender` or `receiver`: which end this pane is.
    pub seen_by: &'static str,
    /// Who sent it, as AgentMux names it: an agent, "outside session (pid N)" or
    /// "unverified sender". Never the name the message gave.
    pub source: String,
    /// `agent`, `outside` or `unverified`.
    pub identity: &'static str,
    /// Who it is for: the receiving agent, or from the sender's end the session
    /// name it addressed.
    pub target: String,
    /// The name the message gave for its sender (a claim).
    pub claimed_name: Option<String>,
    pub cli_to_name: Option<String>,
    pub sender_pid: Option<u32>,
    pub from_mode: Option<String>,
    pub text: String,
    /// Why a held message was held, as the CLI put it.
    pub reason: Option<String>,
}

/// What the reader needs from the running system.
pub trait Env {
    /// (block, CLI process) for every agent pane.
    fn cli_pids(&self) -> Vec<(String, u32)>;
    fn procs(&self) -> &dyn ProcView;
    /// The agent running in `block_id`.
    fn agent_name(&self, block_id: &str) -> Option<String>;
    fn record(&self, event: Event);
}

pub struct CliNativeReader {
    block_id: String,
    /// `SendMessage` calls waiting for the CLI's answer, by tool use id.
    sends: BoundedMap<frame::SendCall>,
    /// Messages handed to the model whose turn has not ended, by frame uuid.
    inbound: BoundedMap<Event>,
    seen: VecDeque<String>,
}

/// A map that forgets its oldest entry past [`REMEMBERED`].
struct BoundedMap<T> {
    map: HashMap<String, T>,
    order: VecDeque<String>,
}

impl<T> BoundedMap<T> {
    fn new() -> Self {
        Self { map: HashMap::new(), order: VecDeque::new() }
    }
    fn insert(&mut self, key: String, value: T) {
        if self.order.len() >= REMEMBERED {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
    fn remove(&mut self, key: &str) -> Option<T> {
        self.map.remove(key)
    }
}

impl CliNativeReader {
    pub fn new(block_id: &str) -> Self {
        Self { block_id: block_id.to_string(), sends: BoundedMap::new(), inbound: BoundedMap::new(), seen: VecDeque::new() }
    }

    /// Reads one stdout frame.
    pub fn observe(&mut self, v: &Value, env: &dyn Env) -> Disposition {
        if let Some(replay) = frame::replay(v) {
            if let Replay::Peer(peer) = replay {
                self.received(peer, env);
            }
            return Disposition::Drop;
        }
        if let Some(hold) = frame::peer_hold(v) {
            self.held(hold, env);
            return Disposition::Keep;
        }
        if let Some((uuid, Lifecycle::Completed)) = frame::lifecycle(v) {
            if let Some(read) = self.inbound.remove(uuid) {
                env.record(Event { state: "done", ..read });
            }
            return Disposition::Keep;
        }
        for call in frame::send_calls(v) {
            self.sends.insert(call.tool_use_id.clone(), call);
        }
        for result in frame::send_results(v) {
            let Some(call) = self.sends.remove(&result.tool_use_id) else { continue };
            let (true, Some(msg_id)) = (result.accepted, result.msg_id) else { continue };
            let me = env.agent_name(&self.block_id).unwrap_or_else(|| self.block_id.clone());
            env.record(Event {
                block_id: self.block_id.clone(),
                msg_id,
                state: "sent",
                seen_by: "sender",
                source: me,
                identity: "agent",
                target: call.to.clone(),
                claimed_name: None,
                cli_to_name: Some(call.to),
                sender_pid: None,
                from_mode: None,
                text: call.text,
                reason: None,
            });
        }
        Disposition::Keep
    }

    /// True the first time `id` is seen.
    fn first(&mut self, id: &str) -> bool {
        if self.seen.iter().any(|s| s == id) {
            return false;
        }
        if self.seen.len() >= REMEMBERED {
            self.seen.pop_front();
        }
        self.seen.push_back(id.to_string());
        true
    }

    /// The CLI kept a message from its model. It did not say who sent it, so the
    /// sender stays unverified, and not what it said.
    fn held(&mut self, hold: frame::PeerHold, env: &dyn Env) {
        if !self.first(&hold.message_uuid) {
            return;
        }
        env.record(Event {
            block_id: self.block_id.clone(),
            msg_id: hold.message_uuid,
            state: "held",
            seen_by: "receiver",
            source: "unverified sender".to_string(),
            identity: "unverified",
            target: env.agent_name(&self.block_id).unwrap_or_else(|| self.block_id.clone()),
            claimed_name: hold.from_name,
            cli_to_name: None,
            sender_pid: None,
            from_mode: None,
            text: String::new(),
            reason: Some(hold.cause),
        });
    }

    fn received(&mut self, peer: PeerFrame, env: &dyn Env) {
        if !self.first(&peer.msg_id) {
            return;
        }

        let sender = sender::resolve(peer.pid, peer.proc_start.as_deref(), &env.cli_pids(), env.procs());
        let (source, identity) = match &sender {
            Sender::Agent { block_id } => (env.agent_name(block_id).unwrap_or_else(|| block_id.clone()), "agent"),
            Sender::Outside { pid } => (format!("outside session (pid {pid})"), "outside"),
            Sender::Unverified => ("unverified sender".to_string(), "unverified"),
        };
        let event = Event {
            block_id: self.block_id.clone(),
            msg_id: peer.msg_id,
            state: "read",
            seen_by: "receiver",
            source,
            identity,
            target: env.agent_name(&self.block_id).unwrap_or_else(|| self.block_id.clone()),
            claimed_name: peer.claimed_name,
            cli_to_name: None,
            sender_pid: peer.pid,
            from_mode: peer.from_mode,
            text: peer.body,
            reason: None,
        };
        self.inbound.insert(peer.uuid, event.clone());
        env.record(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    struct Fake {
        events: Mutex<Vec<Event>>,
        parents: HashMap<u32, u32>,
    }

    impl Fake {
        fn new() -> Self {
            // Agent "alpha" runs its CLI as 100 (with a child 101); "beta" as 200; 300 is outside.
            Self { events: Mutex::new(Vec::new()), parents: HashMap::from([(100, 10), (101, 100), (200, 10), (300, 20)]) }
        }
        fn events(&self) -> Vec<Event> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ProcView for Fake {
        fn parent(&self, pid: u32) -> Option<u32> {
            self.parents.get(&pid).copied()
        }
        fn start_time(&self, _pid: u32) -> Option<u64> {
            None
        }
    }

    impl Env for Fake {
        fn cli_pids(&self) -> Vec<(String, u32)> {
            vec![("block-alpha".into(), 100), ("block-beta".into(), 200), ("block-me".into(), 400)]
        }
        fn procs(&self) -> &dyn ProcView {
            self
        }
        fn agent_name(&self, block_id: &str) -> Option<String> {
            block_id.strip_prefix("block-").map(str::to_string)
        }
        fn record(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn peer(pid: u64, claimed: &str, msg_id: &str, uuid: &str) -> Value {
        json!({"type":"user","message":{"role":"user","content":"<cross-session-message>…</cross-session-message>"},"uuid":uuid,"isReplay":true,"isSynthetic":true,
            "origin":{"kind":"peer","from":"uds:/tmp/cc-socks/x.sock","verifiedPeerPid":pid,"msg_id":msg_id,"name":claimed,"fromMode":"bypass","body":"please look at #5561"}})
    }

    #[test]
    fn a_message_is_read_then_done_under_the_name_agentmux_maps() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        assert_eq!(r.observe(&peer(101, "alpha-session-7", "m1", "u1"), &env), Disposition::Drop);
        assert_eq!(r.observe(&json!({"type":"command_lifecycle","command_uuid":"u1","state":"started"}), &env), Disposition::Keep);
        assert_eq!(r.observe(&json!({"type":"command_lifecycle","command_uuid":"u1","state":"completed"}), &env), Disposition::Keep);
        let ev = env.events();
        assert_eq!(ev.iter().map(|e| e.state).collect::<Vec<_>>(), ["read", "done"]);
        assert_eq!((ev[0].source.as_str(), ev[0].identity, ev[0].target.as_str()), ("alpha", "agent", "me"));
        assert_eq!((ev[0].seen_by, ev[0].sender_pid, ev[0].text.as_str()), ("receiver", Some(101), "please look at #5561"));
        assert_eq!(ev[1].msg_id, "m1");
    }

    #[test]
    fn a_claimed_name_never_becomes_the_sender() {
        // A process under beta's CLI says it is alpha; a stranger says it is beta.
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        r.observe(&peer(200, "alpha", "m1", "u1"), &env);
        r.observe(&peer(300, "beta", "m2", "u2"), &env);
        r.observe(&peer(0, "alpha", "m3", "u3"), &env);
        let ev = env.events();
        assert_eq!((ev[0].source.as_str(), ev[0].claimed_name.as_deref()), ("beta", Some("alpha")));
        assert_eq!((ev[1].source.as_str(), ev[1].identity, ev[1].claimed_name.as_deref()), ("outside session (pid 300)", "outside", Some("beta")));
        assert_eq!((ev[2].source.as_str(), ev[2].identity), ("unverified sender", "unverified"));
    }

    #[test]
    fn a_message_the_cli_holds_is_audited_as_held_with_its_cause_and_no_sender() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        let held = json!({"type":"system","subtype":"peer_message_hold","state":"held","message_uuid":"h1","from_name":"alpha","cause":"mode-mismatch"});
        assert_eq!(r.observe(&held, &env), Disposition::Keep);
        r.observe(&held, &env);
        let ev = env.events();
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].state, ev[0].identity, ev[0].source.as_str()), ("held", "unverified", "unverified sender"));
        assert_eq!((ev[0].reason.as_deref(), ev[0].claimed_name.as_deref(), ev[0].text.as_str()), (Some("mode-mismatch"), Some("alpha"), ""));
    }

    #[test]
    fn a_replayed_message_is_audited_once() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        r.observe(&peer(100, "alpha", "m1", "u1"), &env);
        assert_eq!(r.observe(&peer(100, "alpha", "m1", "u1-again"), &env), Disposition::Drop);
        assert_eq!(env.events().len(), 1);
    }

    #[test]
    fn the_humans_replayed_prompt_leaves_the_stream_and_records_nothing() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        let typed = json!({"type":"user","message":{"role":"user","content":"hello"},"uuid":"u","isReplay":true});
        assert_eq!(r.observe(&typed, &env), Disposition::Drop);
        assert!(env.events().is_empty());
    }

    #[test]
    fn a_sent_message_is_audited_when_the_cli_accepts_it() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        let call = json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"SendMessage","input":{"to":"recv-82","message":"hello"}}]}});
        assert_eq!(r.observe(&call, &env), Disposition::Keep);
        assert!(env.events().is_empty(), "nothing before the CLI answers");
        let answer = json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"{\"success\":true,\"msg_id\":\"m9\"}"}]}]}});
        assert_eq!(r.observe(&answer, &env), Disposition::Keep);
        let ev = env.events();
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].state, ev[0].seen_by, ev[0].source.as_str(), ev[0].target.as_str()), ("sent", "sender", "me", "recv-82"));
        assert_eq!((ev[0].msg_id.as_str(), ev[0].cli_to_name.as_deref(), ev[0].text.as_str()), ("m9", Some("recv-82"), "hello"));
    }

    #[test]
    fn a_refused_send_and_another_tool_record_nothing() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        let calls = json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t1","name":"SendMessage","input":{"to":"x","message":"m"}},
            {"type":"tool_use","id":"t2","name":"mcp__agentmux__SendMessage","input":{"to":"y","message":"m"}}]}});
        r.observe(&calls, &env);
        let answers = json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"t1","content":"Error: no such session","is_error":true},
            {"type":"tool_result","tool_use_id":"t2","content":"{\"success\":true,\"msg_id\":\"jekt\"}"}]}});
        r.observe(&answers, &env);
        assert!(env.events().is_empty());
    }

    #[test]
    fn ordinary_frames_are_kept() {
        let (env, mut r) = (Fake::new(), CliNativeReader::new("block-me"));
        for f in [json!({"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}), json!({"type":"result"}), json!({"type":"system","subtype":"init"})] {
            assert_eq!(r.observe(&f, &env), Disposition::Keep);
        }
    }
}
