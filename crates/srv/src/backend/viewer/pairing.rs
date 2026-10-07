// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pairing codes and viewer tokens (agentmux-mobile's
//! SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §13.2).
//!
//! A code is shown in the desktop's QR, so whoever redeems it has seen this
//! screen. It is 10 characters of base32, works once, for 120 s, and only the
//! newest one shown works: showing a new QR cancels the last. Guessing is
//! bounded twice over: an address with five wrong codes in the last minute is
//! refused with 429 before its next code is even compared, and ten wrong codes
//! in a row, from anywhere, cancel every outstanding code.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use agentmux_common::secret_eq::secret_eq;
use base64::Engine as _;
use sha2::{Digest, Sha256};

pub const CODE_LEN: usize = 10;
pub const CODE_TTL: Duration = Duration::from_secs(120);
/// Wrong codes one address may send per [`WRONG_WINDOW`]; the next attempt
/// gets 429.
pub const WRONG_PER_ADDRESS: usize = 5;
pub const WRONG_WINDOW: Duration = Duration::from_secs(60);
/// Wrong codes in a row, from any address, that cancel every outstanding code.
pub const WRONG_IN_A_ROW: u32 = 10;
pub const TOKEN_PREFIX: &str = "amxv_";

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// `n` bytes from the operating system's CSPRNG (ring's, through rustls).
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut buf)
        .expect("the system random source failed");
    buf
}

/// A fresh pairing code. 32 divides 256, so `byte % 32` is uniform.
pub fn new_code() -> String {
    random_bytes::<CODE_LEN>().iter().map(|b| BASE32[(b % 32) as usize] as char).collect()
}

/// A fresh viewer token: `amxv_` + base64url (no padding) of 32 random bytes.
pub fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<32>())
    )
}

/// What a token is stored as: lowercase hex SHA-256.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The outcome of presenting a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redeem {
    Accepted,
    /// Unknown, expired, already used or cancelled: 401.
    Refused,
    /// Too many wrong codes from this address: 429.
    RateLimited,
}

#[derive(Default)]
struct Inner {
    /// Outstanding codes and when each stops working. At most one in practice.
    codes: Vec<(String, Instant)>,
    wrong_by_address: HashMap<Option<IpAddr>, VecDeque<Instant>>,
    wrong_in_a_row: u32,
}

#[derive(Default)]
pub struct Pairing {
    inner: parking_lot::Mutex<Inner>,
}

impl Pairing {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show a new code: cancels any earlier one. Returns it and how long it
    /// works.
    pub fn start(&self) -> (String, Duration) {
        self.start_at(Instant::now())
    }

    pub(crate) fn start_at(&self, now: Instant) -> (String, Duration) {
        let code = new_code();
        let mut inner = self.inner.lock();
        inner.codes = vec![(code.clone(), now + CODE_TTL)];
        (code, CODE_TTL)
    }

    /// Present `code` from `from` (`None` when the source is unknown, which
    /// counts as one address).
    pub fn redeem(&self, code: &str, from: Option<IpAddr>) -> Redeem {
        self.redeem_at(code, from, Instant::now())
    }

    pub(crate) fn redeem_at(&self, code: &str, from: Option<IpAddr>, now: Instant) -> Redeem {
        let mut inner = self.inner.lock();
        inner.codes.retain(|(_, until)| *until > now);
        let recent = inner.wrong_by_address.entry(from).or_default();
        while recent.front().is_some_and(|at| now.duration_since(*at) >= WRONG_WINDOW) {
            recent.pop_front();
        }
        if recent.len() >= WRONG_PER_ADDRESS {
            return Redeem::RateLimited;
        }
        let presented = code.trim().to_ascii_uppercase();
        // Every outstanding code is compared, in constant time, whatever matched.
        let mut matched = None;
        for (i, (c, _)) in inner.codes.iter().enumerate() {
            if secret_eq(c.as_bytes(), presented.as_bytes()) {
                matched = Some(i);
            }
        }
        match matched {
            Some(i) => {
                inner.codes.remove(i);
                inner.wrong_in_a_row = 0;
                Redeem::Accepted
            }
            None => {
                inner.wrong_by_address.entry(from).or_default().push_back(now);
                inner.wrong_in_a_row += 1;
                if inner.wrong_in_a_row >= WRONG_IN_A_ROW {
                    tracing::warn!(attempts = inner.wrong_in_a_row, "too many wrong pairing codes; cancelling the outstanding code");
                    inner.codes.clear();
                    inner.wrong_in_a_row = 0;
                }
                // Forget addresses with nothing recent, so the map stays small.
                inner.wrong_by_address.retain(|_, v| v.back().is_some_and(|at| now.duration_since(*at) < WRONG_WINDOW));
                Redeem::Refused
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn outstanding(&self) -> usize {
        self.inner.lock().codes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(last: u8) -> Option<IpAddr> {
        Some(IpAddr::from([198, 51, 100, last]))
    }

    #[test]
    fn a_code_is_ten_base32_characters() {
        for _ in 0..50 {
            let code = new_code();
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.bytes().all(|b| BASE32.contains(&b)), "{code}");
        }
        assert_ne!(new_code(), new_code());
    }

    #[test]
    fn a_token_has_the_prefix_and_43_base64url_characters() {
        let t = new_token();
        let body = t.strip_prefix(TOKEN_PREFIX).unwrap();
        assert_eq!(body.len(), 43, "32 bytes, unpadded");
        assert!(body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body).unwrap().len(), 32);
        assert_eq!(token_hash(&t).len(), 64);
        assert_ne!(token_hash(&t), token_hash(&new_token()));
    }

    #[test]
    fn a_code_works_once() {
        let p = Pairing::new();
        let (code, ttl) = p.start();
        assert_eq!(ttl, CODE_TTL);
        assert_eq!(p.redeem(&code, addr(1)), Redeem::Accepted);
        assert_eq!(p.redeem(&code, addr(1)), Redeem::Refused, "single use");
    }

    #[test]
    fn a_code_is_case_and_whitespace_tolerant() {
        let p = Pairing::new();
        let (code, _) = p.start();
        assert_eq!(p.redeem(&format!(" {} ", code.to_lowercase()), addr(1)), Redeem::Accepted);
    }

    #[test]
    fn a_code_expires_after_120_seconds() {
        let p = Pairing::new();
        let t0 = Instant::now();
        let (code, _) = p.start_at(t0);
        assert_eq!(p.redeem_at(&code, addr(1), t0 + CODE_TTL), Redeem::Refused);
        let (code, _) = p.start_at(t0);
        assert_eq!(p.redeem_at(&code, addr(1), t0 + CODE_TTL - Duration::from_secs(1)), Redeem::Accepted);
    }

    #[test]
    fn a_new_code_cancels_the_last() {
        let p = Pairing::new();
        let (old, _) = p.start();
        let (new, _) = p.start();
        assert_eq!(p.redeem(&old, addr(1)), Redeem::Refused);
        assert_eq!(p.redeem(&new, addr(1)), Redeem::Accepted);
    }

    #[test]
    fn five_wrong_codes_in_a_minute_give_429_to_that_address_only() {
        let p = Pairing::new();
        let t0 = Instant::now();
        let (code, _) = p.start_at(t0);
        for i in 0..WRONG_PER_ADDRESS {
            assert_eq!(p.redeem_at("AAAAAAAAAA", addr(1), t0 + Duration::from_secs(i as u64)), Redeem::Refused);
        }
        let t = t0 + Duration::from_secs(10);
        assert_eq!(p.redeem_at(&code, addr(1), t), Redeem::RateLimited, "even the right code");
        assert_eq!(p.redeem_at(&code, addr(2), t), Redeem::Accepted, "another address is not limited");

        // A minute after the first wrong code the address may try again.
        let (code, _) = p.start_at(t);
        let later = t0 + WRONG_WINDOW + Duration::from_secs(5);
        assert_eq!(p.redeem_at(&code, addr(1), later), Redeem::Accepted);
    }

    #[test]
    fn ten_wrong_codes_in_a_row_cancel_the_outstanding_code() {
        let p = Pairing::new();
        let t0 = Instant::now();
        let (code, _) = p.start_at(t0);
        // From several addresses, so no single one is rate limited.
        for i in 0..WRONG_IN_A_ROW {
            let from = addr((i / 2) as u8);
            assert_eq!(p.redeem_at("BBBBBBBBBB", from, t0), Redeem::Refused);
        }
        assert_eq!(p.outstanding(), 0);
        assert_eq!(p.redeem_at(&code, addr(200), t0), Redeem::Refused, "cancelled");
    }

    #[test]
    fn a_right_code_resets_the_run_of_wrong_ones() {
        let p = Pairing::new();
        let t0 = Instant::now();
        for i in 0..(WRONG_IN_A_ROW - 1) {
            p.redeem_at("CCCCCCCCCC", addr(i as u8), t0);
        }
        let (code, _) = p.start_at(t0);
        assert_eq!(p.redeem_at(&code, addr(100), t0), Redeem::Accepted);
        let (code, _) = p.start_at(t0);
        p.redeem_at("CCCCCCCCCC", addr(101), t0);
        assert_eq!(p.outstanding(), 1, "the run started over");
        assert_eq!(p.redeem_at(&code, addr(102), t0), Redeem::Accepted);
    }
}
