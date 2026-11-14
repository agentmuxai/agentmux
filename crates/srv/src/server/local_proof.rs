// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The proof `agentmux-srv open` and a headless srv give each other before `open` asks for anything.
//! SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI §3.
//!
//! `open` never sends the key. It sends a fresh nonce; srv answers with its own nonce and a MAC, made
//! with the key, over both nonces and the address srv is bound to. `open` checks that MAC against the
//! address it is talking to before it sends anything else, so a listener without the key, or one that
//! forwards to the real srv from another port, gets nothing it can reuse. Each later request then
//! carries a MAC of its method, path, body and a rising sequence number, under a key made from the
//! key and both nonces, and each answer carries one too. Every MAC has its own label, so a MAC made
//! for one purpose is never valid for another.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::secret_eq::secret_eq;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub(crate) const CHANNEL_HEADER: &str = "x-am-channel";
pub(crate) const SEQ_HEADER: &str = "x-am-seq";
pub(crate) const PROOF_HEADER: &str = "x-am-proof";

/// A channel lives this long: more than a sign-in takes (a minute for the browser, a minute for the answer).
const CHANNEL_LIFE: Duration = Duration::from_secs(5 * 60);
/// At most this many channels at once; a new one beyond it replaces the oldest that has proved nothing.
const MAX_CHANNELS: usize = 256;

type Hmac256 = Hmac<Sha256>;

/// MAC over length-prefixed parts under `key`, so no two part lists give the same input.
fn mac(key: &[u8], label: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Hmac256::new_from_slice(key).expect("hmac takes any key length");
    for part in std::iter::once(label.as_bytes()).chain(parts.iter().copied()) {
        m.update(&(part.len() as u64).to_be_bytes());
        m.update(part);
    }
    m.finalize().into_bytes().into()
}

/// A nonce is 32 random bytes as 64 lowercase hex characters.
pub(crate) fn is_nonce(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn new_nonce() -> String {
    super::local_session::random_secret()
}

/// What srv proves in its hello: it holds the key, and is bound to `bound` with `declared` as the
/// other origin it accepts (empty when none).
fn hello_proof(key: &[u8], nonce_c: &str, nonce_s: &str, bound: &str, declared: &str) -> String {
    hex::encode(mac(key, "amx-srv-hello-v1", &[nonce_c.as_bytes(), nonce_s.as_bytes(), bound.as_bytes(), declared.as_bytes()]))
}

/// The key of one channel: made from the key and everything the hello proved.
fn channel_key(key: &[u8], nonce_c: &str, nonce_s: &str, bound: &str, declared: &str) -> [u8; 32] {
    mac(key, "amx-channel-v1", &[nonce_c.as_bytes(), nonce_s.as_bytes(), bound.as_bytes(), declared.as_bytes()])
}

fn request_proof(ck: &[u8; 32], seq: u64, method: &str, path: &str, body: &[u8]) -> String {
    hex::encode(mac(ck, "amx-req-v1", &[&seq.to_be_bytes(), method.as_bytes(), path.as_bytes(), &Sha256::digest(body)]))
}

/// The proof on srv's answer to request number `seq`.
pub(crate) fn response_proof(ck: &ChannelKey, seq: u64, status: u16, body: &[u8]) -> String {
    hex::encode(mac(&ck.0, "amx-resp-v1", &[&seq.to_be_bytes(), &status.to_be_bytes(), &Sha256::digest(body)]))
}

/// A channel's key, kept in memory only.
#[derive(Clone)]
pub(crate) struct ChannelKey([u8; 32]);

// ---- srv's side ----

/// srv's answer to a hello.
pub(crate) struct Hello {
    pub channel: String,
    pub nonce: String,
    pub proof: String,
}

struct Channel {
    key: ChannelKey,
    last_seq: u64,
    born: Instant,
}

/// The channels `open` has begun. Memory only.
#[derive(Default)]
pub(crate) struct Channels {
    inner: Mutex<HashMap<String, Channel>>,
}

/// A request that proved itself: the answer to it is proved with this.
pub(crate) struct Admitted {
    pub key: ChannelKey,
    pub seq: u64,
}

impl Channels {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Channel>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Begin a channel. `None` when the caller's nonce is not a nonce. Takes no secret from the caller:
    /// anyone may ask, and what comes back proves only that srv holds the key.
    pub(crate) fn hello(&self, key: &[u8], nonce_c: &str, bound: &str, declared: &str, now: Instant) -> Option<Hello> {
        if !is_nonce(nonce_c) {
            return None;
        }
        let nonce_s = new_nonce();
        let id = new_nonce();
        let mut map = self.lock();
        map.retain(|_, c| now.duration_since(c.born) <= CHANNEL_LIFE);
        if map.len() >= MAX_CHANNELS {
            // A caller without the key can never move a channel off 0, so a flood replaces channels
            // that have proved nothing first; one that has proved a request goes only when all have.
            let victim = map
                .iter()
                .filter(|(_, c)| c.last_seq == 0)
                .min_by_key(|(_, c)| c.born)
                .or_else(|| map.iter().min_by_key(|(_, c)| c.born))
                .map(|(id, _)| id.clone());
            if let Some(victim) = victim {
                map.remove(&victim);
            }
        }
        let ck = ChannelKey(channel_key(key, nonce_c, &nonce_s, bound, declared));
        map.insert(id.clone(), Channel { key: ck, last_seq: 0, born: now });
        Some(Hello { channel: id, nonce: nonce_s.clone(), proof: hello_proof(key, nonce_c, &nonce_s, bound, declared) })
    }

    /// A request on a channel: its number is higher than any before it, and its proof covers the
    /// method, path and body as srv received them. A request that fails changes nothing.
    pub(crate) fn admit(&self, id: &str, seq: u64, method: &str, path: &str, body: &[u8], proof: &str, now: Instant) -> Option<Admitted> {
        let mut map = self.lock();
        let channel = map.get_mut(id)?;
        if now.duration_since(channel.born) > CHANNEL_LIFE || seq <= channel.last_seq {
            return None;
        }
        let expected = request_proof(&channel.key.0, seq, method, path, body);
        if !secret_eq(proof.as_bytes(), expected.as_bytes()) {
            return None;
        }
        channel.last_seq = seq;
        Some(Admitted { key: channel.key.clone(), seq })
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.lock().len()
    }
}

// ---- open's side ----

/// What srv said in its hello.
pub(crate) struct HelloReply {
    pub channel: String,
    pub nonce: String,
    /// The other origin srv says it accepts; covered by the proof, empty when none.
    pub declared: String,
    pub proof: String,
}

/// A hello that did not prove srv holds the key, or is bound to the address `open` is talking to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NotProved;

/// `open`'s end of a channel, after srv proved itself.
pub(crate) struct Client {
    channel: String,
    key: ChannelKey,
    seq: u64,
    declared: String,
}

impl Client {
    /// Accept srv's hello only when its proof is made with this key, over this `nonce_c`, for exactly
    /// the address `open` is talking to (`bound`, as `host:port`).
    pub(crate) fn accept(key: &[u8], nonce_c: &str, bound: &str, reply: HelloReply) -> Result<Client, NotProved> {
        if !is_nonce(&reply.nonce) || !is_nonce(&reply.channel) {
            return Err(NotProved);
        }
        let expected = hello_proof(key, nonce_c, &reply.nonce, bound, &reply.declared);
        if !secret_eq(reply.proof.as_bytes(), expected.as_bytes()) {
            return Err(NotProved);
        }
        let ck = ChannelKey(channel_key(key, nonce_c, &reply.nonce, bound, &reply.declared));
        Ok(Client { channel: reply.channel, key: ck, seq: 0, declared: reply.declared })
    }

    /// The other origin srv accepts, as srv proved it.
    pub(crate) fn declared(&self) -> Option<&str> {
        Some(self.declared.as_str()).filter(|d| !d.is_empty())
    }

    /// The headers of the next request: channel, number and proof.
    pub(crate) fn sign(&mut self, method: &str, path: &str, body: &[u8]) -> Signed {
        self.seq += 1;
        Signed { channel: self.channel.clone(), seq: self.seq, proof: request_proof(&self.key.0, self.seq, method, path, body) }
    }

    /// Does srv's answer to request `seq` carry the proof of the channel?
    pub(crate) fn answer_ok(&self, seq: u64, status: u16, body: &[u8], proof: &str) -> bool {
        secret_eq(proof.as_bytes(), response_proof(&self.key, seq, status, body).as_bytes())
    }
}

pub(crate) struct Signed {
    pub channel: String,
    pub seq: u64,
    pub proof: String,
}

#[cfg(test)]
#[path = "tests/local_proof_tests.rs"]
mod tests;
