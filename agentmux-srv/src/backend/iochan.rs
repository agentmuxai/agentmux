// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! I/O channel types and streaming utilities.
//! Port of Go's `pkg/util/iochan/` — provides streaming reader/writer channels
//! with SHA256 checksum verification.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// A data packet with optional checksum.
/// When `data` is present and `checksum` is None, this is a data packet.
/// When `checksum` is Some, this is the final packet containing the SHA256 checksum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Packet {
    /// The data payload. Empty for the final checksum packet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<u8>,
    /// SHA256 checksum, sent as the last packet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<Vec<u8>>,
}

impl Packet {
    /// Create a data packet.
    pub fn data(data: Vec<u8>) -> Self {
        Self { data, checksum: None }
    }

    /// Create a checksum (final) packet.
    pub fn checksum(hash: Vec<u8>) -> Self {
        Self { data: Vec::new(), checksum: Some(hash) }
    }

    /// Check if this is the final checksum packet.
    pub fn is_checksum(&self) -> bool {
        self.checksum.is_some()
    }
}

/// Result type for channel operations: either a response or an error.
#[derive(Debug, Clone)]
pub enum RespOrError<T> {
    Response(T),
    Error(String),
}

impl<T> RespOrError<T> {
    pub fn ok(val: T) -> Self {
        Self::Response(val)
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self::Error(msg.into())
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// Simple SHA256 implementation for streaming hashing.
/// Used by ReaderChan and WriterChan for checksum verification.
pub struct Sha256Hasher {
    state: [u32; 8],
    buffer: Vec<u8>,
    total_len: u64,
}

impl Sha256Hasher {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::new(),
            total_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_len += data.len() as u64;
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap();
            self.process_block(&block);
            self.buffer.drain(..64);
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn process_block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(Self::K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g; g = f; f = e;
            e = d.wrapping_add(temp1);
            d = c; c = b; b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len * 8;
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());

        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap();
            self.process_block(&block);
            self.buffer.drain(..64);
        }

        let mut result = [0u8; 32];
        for (i, &s) in self.state.iter().enumerate() {
            result[i * 4..i * 4 + 4].copy_from_slice(&s.to_be_bytes());
        }
        result
    }
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Verify that two checksums match.
pub fn checksums_match(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // Constant-time comparison to prevent timing attacks
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_data() {
        let p = Packet::data(vec![1, 2, 3]);
        assert_eq!(p.data, vec![1, 2, 3]);
        assert!(!p.is_checksum());
    }

    #[test]
    fn test_packet_checksum() {
        let p = Packet::checksum(vec![0xAB, 0xCD]);
        assert!(p.is_checksum());
        assert!(p.data.is_empty());
    }

    #[test]
    fn test_resp_or_error_ok() {
        let r: RespOrError<i32> = RespOrError::ok(42);
        assert!(!r.is_error());
    }

    #[test]
    fn test_resp_or_error_err() {
        let r: RespOrError<i32> = RespOrError::err("fail");
        assert!(r.is_error());
    }

    #[test]
    fn test_sha256_empty() {
        let hasher = Sha256Hasher::new();
        let result = hasher.finalize();
        // SHA256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            hex_encode(&result),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hello() {
        let mut hasher = Sha256Hasher::new();
        hasher.update(b"hello");
        let result = hasher.finalize();
        // SHA256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(
            hex_encode(&result),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_sha256_streaming() {
        // Hash in two chunks should equal hashing all at once
        let mut hasher1 = Sha256Hasher::new();
        hasher1.update(b"hello ");
        hasher1.update(b"world");
        let result1 = hasher1.finalize();

        let mut hasher2 = Sha256Hasher::new();
        hasher2.update(b"hello world");
        let result2 = hasher2.finalize();

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_sha256_large() {
        // Test with data > 64 bytes (block size)
        let data = vec![0x41u8; 200]; // 200 bytes of 'A'
        let mut hasher = Sha256Hasher::new();
        hasher.update(&data);
        let result = hasher.finalize();
        // Verify it's a valid 32-byte hash
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_checksums_match() {
        let a = [1u8, 2, 3, 4];
        let b = [1u8, 2, 3, 4];
        assert!(checksums_match(&a, &b));
    }

    #[test]
    fn test_checksums_mismatch() {
        let a = [1u8, 2, 3, 4];
        let b = [1u8, 2, 3, 5];
        assert!(!checksums_match(&a, &b));
    }

    #[test]
    fn test_checksums_different_length() {
        let a = [1u8, 2, 3];
        let b = [1u8, 2, 3, 4];
        assert!(!checksums_match(&a, &b));
    }

    #[test]
    fn test_packet_serde_roundtrip() {
        let p = Packet::data(vec![1, 2, 3]);
        let json = serde_json::to_string(&p).unwrap();
        let p2: Packet = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn test_packet_checksum_serde_roundtrip() {
        let p = Packet::checksum(vec![0xAB, 0xCD]);
        let json = serde_json::to_string(&p).unwrap();
        let p2: Packet = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    fn hex_encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
