// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const KEY: &[u8] = b"the-instance-key";
const BOUND: &str = "127.0.0.1:7380";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A hello as `open` and srv would carry it, for `bound` on both ends unless a test says otherwise.
fn begin(channels: &Channels, srv_key: &[u8], srv_bound: &str, open_key: &[u8], open_bound: &str, now: Instant) -> Result<Client, NotProved> {
    let nonce_c = new_nonce();
    let h = channels.hello(srv_key, &nonce_c, srv_bound, "", now).unwrap();
    Client::accept(open_key, &nonce_c, open_bound, HelloReply { channel: h.channel, nonce: h.nonce, declared: String::new(), proof: h.proof })
}

#[test]
fn a_srv_with_the_key_at_the_address_is_accepted() {
    let channels = Channels::default();
    assert!(begin(&channels, KEY, BOUND, KEY, BOUND, Instant::now()).is_ok());
}

#[test]
fn a_listener_without_the_key_is_not_accepted() {
    let channels = Channels::default();
    assert!(begin(&channels, b"another-key", BOUND, KEY, BOUND, Instant::now()).is_err());
    // A made-up answer, shaped right: nothing but luck makes the MAC.
    let nonce_c = new_nonce();
    let fake = HelloReply { channel: new_nonce(), nonce: new_nonce(), declared: String::new(), proof: "0".repeat(64) };
    assert!(Client::accept(KEY, &nonce_c, BOUND, fake).is_err());
    let fake = HelloReply { channel: "x".into(), nonce: "y".into(), declared: String::new(), proof: String::new() };
    assert!(Client::accept(KEY, &nonce_c, BOUND, fake).is_err());
}

/// A forwarder on another port passes srv's hello on unchanged; the proof is for the address srv is
/// bound to, so it does not suit the address `open` is talking to.
#[test]
fn a_forwarder_on_another_port_is_not_accepted() {
    let channels = Channels::default();
    assert!(begin(&channels, KEY, BOUND, KEY, "127.0.0.1:7381", Instant::now()).is_err());
    assert!(begin(&channels, KEY, BOUND, KEY, "127.0.0.1:738", Instant::now()).is_err());
}

#[test]
fn a_hello_answer_does_not_suit_another_hello() {
    let channels = Channels::default();
    let now = Instant::now();
    let first = new_nonce();
    let h = channels.hello(KEY, &first, BOUND, "", now).unwrap();
    // The same answer, replayed against a second hello of `open`.
    let second = new_nonce();
    let reply = HelloReply { channel: h.channel, nonce: h.nonce, declared: String::new(), proof: h.proof };
    assert!(Client::accept(KEY, &second, BOUND, reply).is_err());
}

#[test]
fn the_declared_origin_is_covered_by_the_proof() {
    let channels = Channels::default();
    let nonce_c = new_nonce();
    let h = channels.hello(KEY, &nonce_c, BOUND, "http://127.0.0.1:8080", Instant::now()).unwrap();
    let reply = |declared: &str| HelloReply { channel: h.channel.clone(), nonce: h.nonce.clone(), declared: declared.into(), proof: h.proof.clone() };
    let client = Client::accept(KEY, &nonce_c, BOUND, reply("http://127.0.0.1:8080")).unwrap();
    assert_eq!(client.declared(), Some("http://127.0.0.1:8080"));
    assert!(Client::accept(KEY, &nonce_c, BOUND, reply("http://127.0.0.1:9090")).is_err(), "a changed declaration");
    assert!(Client::accept(KEY, &nonce_c, BOUND, reply("")).is_err(), "a removed declaration");
}

#[test]
fn a_hello_takes_only_a_real_nonce() {
    let channels = Channels::default();
    for bad in ["", "abc", &"g".repeat(64), &"A".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
        assert!(channels.hello(KEY, bad, BOUND, "", Instant::now()).is_none(), "{bad:?}");
    }
    assert_eq!(channels.count(), 0);
}

#[test]
fn a_request_is_admitted_once_and_only_as_signed() {
    let channels = Channels::default();
    let now = Instant::now();
    let nonce_c = new_nonce();
    let h = channels.hello(KEY, &nonce_c, BOUND, "", now).unwrap();
    let mut client = Client::accept(KEY, &nonce_c, BOUND, HelloReply { channel: h.channel.clone(), nonce: h.nonce, declared: String::new(), proof: h.proof }).unwrap();

    let s = client.sign("POST", "/auth/code", b"{}");
    assert!(channels.admit(&s.channel, s.seq, "POST", "/auth/confirm", b"{}", &s.proof, now).is_none(), "another path");
    assert!(channels.admit(&s.channel, s.seq, "GET", "/auth/code", b"{}", &s.proof, now).is_none(), "another method");
    assert!(channels.admit(&s.channel, s.seq, "POST", "/auth/code", b"{\"x\":1}", &s.proof, now).is_none(), "another body");
    assert!(channels.admit(&s.channel, s.seq + 1, "POST", "/auth/code", b"{}", &s.proof, now).is_none(), "another number");
    assert!(channels.admit("unknown", s.seq, "POST", "/auth/code", b"{}", &s.proof, now).is_none(), "another channel");
    let admitted = channels.admit(&s.channel, s.seq, "POST", "/auth/code", b"{}", &s.proof, now).expect("as signed");
    assert_eq!(admitted.seq, 1);
    assert!(channels.admit(&s.channel, s.seq, "POST", "/auth/code", b"{}", &s.proof, now).is_none(), "the same request again");

    // The next one carries a higher number; an older number is not taken even when it was never used.
    let s2 = client.sign("POST", "/auth/pending", b"{}");
    assert!(channels.admit(&s2.channel, s2.seq, "POST", "/auth/pending", b"{}", &s2.proof, now).is_some());
    let stale = request_proof(&client.key.0, 1, "POST", "/auth/pending", b"{}");
    assert!(channels.admit(&s2.channel, 1, "POST", "/auth/pending", b"{}", &stale, now).is_none());
}

#[test]
fn a_request_proof_of_one_channel_is_not_valid_on_another() {
    let channels = Channels::default();
    let now = Instant::now();
    let mk = || {
        let nonce_c = new_nonce();
        let h = channels.hello(KEY, &nonce_c, BOUND, "", now).unwrap();
        let channel = h.channel.clone();
        (Client::accept(KEY, &nonce_c, BOUND, HelloReply { channel: h.channel, nonce: h.nonce, declared: String::new(), proof: h.proof }).unwrap(), channel)
    };
    let (mut a, _) = mk();
    let (_, b_channel) = mk();
    let s = a.sign("POST", "/auth/code", b"{}");
    assert!(channels.admit(&b_channel, s.seq, "POST", "/auth/code", b"{}", &s.proof, now).is_none());
}

#[test]
fn an_answer_is_proved_for_its_request_status_and_body() {
    let channels = Channels::default();
    let now = Instant::now();
    let nonce_c = new_nonce();
    let h = channels.hello(KEY, &nonce_c, BOUND, "", now).unwrap();
    let mut client = Client::accept(KEY, &nonce_c, BOUND, HelloReply { channel: h.channel, nonce: h.nonce, declared: String::new(), proof: h.proof }).unwrap();
    let s = client.sign("POST", "/auth/code", b"{}");
    let admitted = channels.admit(&s.channel, s.seq, "POST", "/auth/code", b"{}", &s.proof, now).unwrap();
    let body = br#"{"code":"abc"}"#;
    let proof = response_proof(&admitted.key, admitted.seq, 200, body);
    assert!(client.answer_ok(s.seq, 200, body, &proof));
    assert!(!client.answer_ok(s.seq, 200, br#"{"code":"abd"}"#, &proof), "another body");
    assert!(!client.answer_ok(s.seq, 403, body, &proof), "another status");
    assert!(!client.answer_ok(s.seq + 1, 200, body, &proof), "another request");
    assert!(!client.answer_ok(s.seq, 200, body, ""), "no proof");
}

/// A MAC made for one purpose is not valid for another, even over the same fields.
#[test]
fn each_proof_has_its_own_label() {
    let a = hex::encode(mac(KEY, "amx-srv-hello-v1", &[b"x"]));
    let b = hex::encode(mac(KEY, "amx-req-v1", &[b"x"]));
    let c = hex::encode(mac(KEY, "amx-resp-v1", &[b"x"]));
    assert!(a != b && b != c && a != c);
    // Parts do not run together.
    assert_ne!(mac(KEY, "l", &[b"ab", b"c"]), mac(KEY, "l", &[b"a", b"bc"]));
}

#[test]
fn channels_expire_and_their_number_is_capped() {
    let channels = Channels::default();
    let t0 = Instant::now();
    let nonce_c = new_nonce();
    let h = channels.hello(KEY, &nonce_c, BOUND, "", t0).unwrap();
    let mut client = Client::accept(KEY, &nonce_c, BOUND, HelloReply { channel: h.channel, nonce: h.nonce, declared: String::new(), proof: h.proof }).unwrap();
    let s = client.sign("POST", "/auth/code", b"{}");
    assert!(channels.admit(&s.channel, s.seq, "POST", "/auth/code", b"{}", &s.proof, t0 + CHANNEL_LIFE + secs(1)).is_none(), "expired");

    let flood = Channels::default();
    for _ in 0..(MAX_CHANNELS * 3) {
        flood.hello(KEY, &new_nonce(), BOUND, "", t0).unwrap();
    }
    assert_eq!(flood.count(), MAX_CHANNELS);
    // They age out: a hello a while later clears them.
    flood.hello(KEY, &new_nonce(), BOUND, "", t0 + CHANNEL_LIFE + secs(1)).unwrap();
    assert_eq!(flood.count(), 1);
}
