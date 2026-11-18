// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The stamp the container agents' gateway forwarder puts on a model-gateway request, and what srv checks.
//!
//! Where srv cannot see which container a request comes from (Docker Desktop: every container arrives from one address),
//! a small forwarder on the agents network sees the container's own address and says it to srv in three headers: the
//! address, the time, and a proof. The proof is an HMAC of the address, the time, the request's method and its target
//! (path and query), made with a secret only srv and the forwarder have, so a stamp cannot be moved to another request,
//! another address or another time. srv accepts a stamp only on the gateway's own paths and for [`WINDOW_SECS`].

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::IpAddr;

/// The model gateway's path prefix: the only paths the forwarder forwards and a stamp is accepted on.
pub const PATH_PREFIX: &str = "/api/v1/model/";
/// Every header the forwarder sets starts with this; it removes any such header a client sent.
pub const HEADER_PREFIX: &str = "x-amx-fwd-";
pub const HEADER_PEER: &str = "x-amx-fwd-peer";
pub const HEADER_TS: &str = "x-amx-fwd-ts";
pub const HEADER_PROOF: &str = "x-amx-fwd-proof";
/// How long a stamp is good for after the time it names, in seconds.
pub const WINDOW_SECS: u64 = 30;
/// How far ahead of this machine's clock a stamp's time may be, in seconds (two clocks on one machine).
const AHEAD_SECS: u64 = 5;

type HmacSha256 = Hmac<Sha256>;

/// Why a stamp is not accepted. Never shown to the caller beyond "refused".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A stamp header is missing.
    Missing,
    /// A header is not an address, a time or a proof.
    Malformed,
    /// The time is outside the window.
    Expired,
    /// The request's target is not under the gateway's path prefix, or is written in a way that could mean another path.
    OutsidePrefix,
    /// The proof does not match.
    BadProof,
}

/// The request target a stamp may cover: under [`PATH_PREFIX`] exactly as written, with nothing that could make a
/// different path of it (an encoded byte, a backslash, an empty or dot segment).
pub fn covered_target(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or("");
    path.starts_with(PATH_PREFIX) && !path.contains('%') && !path.contains('\\') && path[PATH_PREFIX.len()..].split('/').all(|seg| seg != ".." && seg != ".") && !path.contains("//")
}

fn mac(secret: &[u8], peer: IpAddr, ts: u64, method: &str, target: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("an HMAC takes a key of any length");
    for part in [b"amx-gateway-fwd-v1".as_slice(), peer.to_string().as_bytes(), ts.to_string().as_bytes(), method.to_ascii_uppercase().as_bytes(), target.as_bytes()] {
        mac.update(part);
        mac.update(b"\n");
    }
    mac
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// The proof for one request, as hex.
pub fn proof(secret: &[u8], peer: IpAddr, ts: u64, method: &str, target: &str) -> String {
    hex(&mac(secret, peer, ts, method, target).finalize().into_bytes())
}

/// The peer a stamp names, when it is good for this request now. The comparison of the proofs does not depend on where they
/// differ.
pub fn verify(secret: &[u8], peer: Option<&str>, ts: Option<&str>, proof: Option<&str>, method: &str, target: &str, now_secs: u64) -> Result<IpAddr, Refusal> {
    let (Some(peer), Some(ts), Some(proof)) = (peer, ts, proof) else { return Err(Refusal::Missing) };
    let peer: IpAddr = peer.parse().map_err(|_| Refusal::Malformed)?;
    let ts: u64 = ts.parse().map_err(|_| Refusal::Malformed)?;
    let given = unhex(proof).ok_or(Refusal::Malformed)?;
    if !covered_target(target) {
        return Err(Refusal::OutsidePrefix);
    }
    if ts > now_secs.saturating_add(AHEAD_SECS) || now_secs.saturating_sub(ts) > WINDOW_SECS {
        return Err(Refusal::Expired);
    }
    mac(secret, peer, ts, method, target).verify_slice(&given).map(|()| peer).map_err(|_| Refusal::BadProof)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-not-a-real-one";
    const TARGET: &str = "/api/v1/model/anthropic/v1/messages?beta=true";

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([172, 30, 0, last])
    }

    fn stamped(peer: IpAddr, ts: u64, method: &str, target: &str) -> (String, String, String) {
        (peer.to_string(), ts.to_string(), proof(SECRET, peer, ts, method, target))
    }

    fn check(s: &(String, String, String), method: &str, target: &str, now: u64) -> Result<IpAddr, Refusal> {
        verify(SECRET, Some(&s.0), Some(&s.1), Some(&s.2), method, target, now)
    }

    #[test]
    fn a_good_stamp_names_its_peer() {
        let s = stamped(ip(9), 1000, "POST", TARGET);
        assert_eq!(check(&s, "POST", TARGET, 1000), Ok(ip(9)));
        assert_eq!(check(&s, "post", TARGET, 1010), Ok(ip(9)), "the method's case does not matter");
        assert_eq!(check(&s, "POST", TARGET, 1000 + WINDOW_SECS), Ok(ip(9)));
    }

    #[test]
    fn a_stamp_cannot_be_moved_to_another_request_address_or_time() {
        let s = stamped(ip(9), 1000, "POST", TARGET);
        assert_eq!(check(&s, "GET", TARGET, 1000), Err(Refusal::BadProof), "another method");
        assert_eq!(check(&s, "POST", "/api/v1/model/anthropic/v1/messages", 1000), Err(Refusal::BadProof), "another target");
        let other_peer = (ip(10).to_string(), s.1.clone(), s.2.clone());
        assert_eq!(check(&other_peer, "POST", TARGET, 1000), Err(Refusal::BadProof), "another address");
        let later = (s.0.clone(), "1001".to_string(), s.2.clone());
        assert_eq!(check(&later, "POST", TARGET, 1001), Err(Refusal::BadProof), "another time");
        assert_eq!(verify(b"another-secret", Some(&s.0), Some(&s.1), Some(&s.2), "POST", TARGET, 1000), Err(Refusal::BadProof), "another secret");
    }

    #[test]
    fn a_stamp_is_good_only_inside_its_window() {
        let s = stamped(ip(9), 1000, "POST", TARGET);
        assert_eq!(check(&s, "POST", TARGET, 1000 + WINDOW_SECS + 1), Err(Refusal::Expired));
        assert_eq!(check(&s, "POST", TARGET, 1000 - 5), Ok(ip(9)), "a clock a few seconds behind");
        assert_eq!(check(&s, "POST", TARGET, 1000 - 6), Err(Refusal::Expired), "a time well ahead of this clock");
    }

    #[test]
    fn a_stamp_is_refused_on_any_path_outside_the_gateway_prefix() {
        for target in [
            "/api/v1/MODEL/anthropic/v1/messages",
            "/API/v1/model/anthropic/v1/messages",
            "/api/v1/%6dodel/anthropic/v1/messages",
            "/api/v1/model/anthropic/%2e%2e/%2e%2e/secrets",
            "/api/v1/model/../containers",
            "/api/v1/model/./anthropic",
            "/api/v1/model//anthropic",
            "/api/v1/model\\anthropic",
            "/api/v1/models/anthropic",
            "/api/v1/model",
            "/api/v1/containers",
            "/agentmux/hook/x",
            "",
        ] {
            let s = stamped(ip(9), 1000, "POST", target);
            assert_eq!(check(&s, "POST", target, 1000), Err(Refusal::OutsidePrefix), "{target}");
        }
        // A query may hold anything: it is not part of the path.
        let with_query = "/api/v1/model/anthropic/v1/messages?next=/../x%2f";
        assert_eq!(check(&stamped(ip(9), 1000, "POST", with_query), "POST", with_query, 1000), Ok(ip(9)));
    }

    #[test]
    fn a_missing_or_malformed_stamp_is_refused() {
        let s = stamped(ip(9), 1000, "POST", TARGET);
        assert_eq!(verify(SECRET, None, Some(&s.1), Some(&s.2), "POST", TARGET, 1000), Err(Refusal::Missing));
        assert_eq!(verify(SECRET, Some(&s.0), None, Some(&s.2), "POST", TARGET, 1000), Err(Refusal::Missing));
        assert_eq!(verify(SECRET, Some(&s.0), Some(&s.1), None, "POST", TARGET, 1000), Err(Refusal::Missing));
        assert_eq!(verify(SECRET, Some("not-an-address"), Some(&s.1), Some(&s.2), "POST", TARGET, 1000), Err(Refusal::Malformed));
        assert_eq!(verify(SECRET, Some(&s.0), Some("soon"), Some(&s.2), "POST", TARGET, 1000), Err(Refusal::Malformed));
        assert_eq!(verify(SECRET, Some(&s.0), Some(&s.1), Some("zz"), "POST", TARGET, 1000), Err(Refusal::Malformed));
        assert_eq!(verify(SECRET, Some(&s.0), Some(&s.1), Some("abc"), "POST", TARGET, 1000), Err(Refusal::Malformed));
        assert_eq!(verify(SECRET, Some(&s.0), Some(&s.1), Some(""), "POST", TARGET, 1000), Err(Refusal::BadProof), "an empty proof matches nothing");
    }
}
