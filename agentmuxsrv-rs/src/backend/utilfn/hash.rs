// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

// ---- Public API ----

/// SHA1 hash of data, returned as base64 string.
pub fn sha1_hash(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    base64_encode(&result)
}

/// FNV-64a hash of a string, returned as base64url (no padding) string.
pub fn quick_hash_string(s: &str) -> String {
    let mut hasher = Fnv64a::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    base64_url_encode(&result)
}

// ---- SHA1 implementation ----

struct Sha1 {
    state: [u32; 5],
    count: u64,
    buffer: [u8; 64],
    buffer_len: usize,
}

impl Sha1 {
    fn new() -> Self {
        Self {
            state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            count: 0,
            buffer: [0u8; 64],
            buffer_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        let mut i = 0;
        self.count += data.len() as u64;
        if self.buffer_len > 0 {
            let space = 64 - self.buffer_len;
            let copy_len = std::cmp::min(space, data.len());
            self.buffer[self.buffer_len..self.buffer_len + copy_len].copy_from_slice(&data[..copy_len]);
            self.buffer_len += copy_len;
            i = copy_len;
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.process_block(&block);
                self.buffer_len = 0;
            }
        }
        while i + 64 <= data.len() {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[i..i + 64]);
            self.process_block(&block);
            i += 64;
        }
        if i < data.len() {
            let remaining = data.len() - i;
            self.buffer[..remaining].copy_from_slice(&data[i..]);
            self.buffer_len = remaining;
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn process_block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = self.state;
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }

    fn finalize(mut self) -> [u8; 20] {
        let bit_count = self.count * 8;
        // Padding
        let mut padding = vec![0x80u8];
        let pad_len = if self.buffer_len < 56 {
            56 - self.buffer_len - 1
        } else {
            120 - self.buffer_len - 1
        };
        padding.extend(std::iter::repeat_n(0u8, pad_len));
        padding.extend_from_slice(&bit_count.to_be_bytes());
        self.update(&padding);

        let mut result = [0u8; 20];
        for (i, &s) in self.state.iter().enumerate() {
            result[i * 4..i * 4 + 4].copy_from_slice(&s.to_be_bytes());
        }
        result
    }
}

// ---- FNV-64a implementation ----

struct Fnv64a {
    hash: u64,
}

impl Fnv64a {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    fn new() -> Self {
        Self { hash: Self::OFFSET_BASIS }
    }

    fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.hash ^= b as u64;
            self.hash = self.hash.wrapping_mul(Self::PRIME);
        }
    }

    fn finalize(&self) -> [u8; 8] {
        self.hash.to_be_bytes()
    }
}

// ---- Base64 encoding ----

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if i + 2 < data.len() {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

fn base64_url_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        }
        i += 3;
    }
    result
}
