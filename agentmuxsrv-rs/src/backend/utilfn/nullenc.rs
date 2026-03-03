// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::collections::HashMap;

// ---- Constants ----

const NULL_ENCODE_ESC_BYTE: u8 = b'\\';
const NULL_ENCODE_SEP_BYTE: u8 = b'|';
const NULL_ENCODE_EQ_BYTE: u8 = b'=';
const NULL_ENCODE_ZERO_BYTE_ESC: u8 = b'0';
const NULL_ENCODE_ESC_BYTE_ESC: u8 = b'\\';
const NULL_ENCODE_SEP_BYTE_ESC: u8 = b's';
const NULL_ENCODE_EQ_BYTE_ESC: u8 = b'e';

// ---- Encoding / Decoding ----

/// Encode a string, escaping null bytes, backslashes, separators (`|`), and equals (`=`).
///
/// - `\0` → `\0`
/// - `\` → `\\`
/// - `|` → `\s`
/// - `=` → `\e`
pub fn null_encode_str(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    // Fast path: if no special bytes, return as-is
    if !bytes.iter().any(|&b| b == 0 || b == NULL_ENCODE_ESC_BYTE || b == NULL_ENCODE_SEP_BYTE || b == NULL_ENCODE_EQ_BYTE) {
        return bytes.to_vec();
    }
    let mut rtn = Vec::with_capacity(bytes.len() + 8);
    for &b in bytes {
        match b {
            0 => {
                rtn.push(NULL_ENCODE_ESC_BYTE);
                rtn.push(NULL_ENCODE_ZERO_BYTE_ESC);
            }
            b if b == NULL_ENCODE_ESC_BYTE => {
                rtn.push(NULL_ENCODE_ESC_BYTE);
                rtn.push(NULL_ENCODE_ESC_BYTE_ESC);
            }
            b if b == NULL_ENCODE_SEP_BYTE => {
                rtn.push(NULL_ENCODE_ESC_BYTE);
                rtn.push(NULL_ENCODE_SEP_BYTE_ESC);
            }
            b if b == NULL_ENCODE_EQ_BYTE => {
                rtn.push(NULL_ENCODE_ESC_BYTE);
                rtn.push(NULL_ENCODE_EQ_BYTE_ESC);
            }
            _ => rtn.push(b),
        }
    }
    rtn
}

/// Decode a null-encoded byte slice back to a string.
pub fn null_decode_str(barr: &[u8]) -> Result<String, String> {
    if !barr.contains(&NULL_ENCODE_ESC_BYTE) {
        return Ok(String::from_utf8_lossy(barr).into_owned());
    }
    let mut rtn = Vec::with_capacity(barr.len());
    let mut i = 0;
    while i < barr.len() {
        let cur = barr[i];
        if cur == NULL_ENCODE_ESC_BYTE {
            i += 1;
            if i >= barr.len() {
                return Err("invalid null encoding: escape at end of string".to_string());
            }
            let next = barr[i];
            match next {
                NULL_ENCODE_ZERO_BYTE_ESC => rtn.push(0),
                NULL_ENCODE_ESC_BYTE_ESC => rtn.push(NULL_ENCODE_ESC_BYTE),
                NULL_ENCODE_SEP_BYTE_ESC => rtn.push(NULL_ENCODE_SEP_BYTE),
                NULL_ENCODE_EQ_BYTE_ESC => rtn.push(NULL_ENCODE_EQ_BYTE),
                _ => return Err(format!("invalid null encoding: {}", next)),
            }
        } else {
            rtn.push(cur);
        }
        i += 1;
    }
    Ok(String::from_utf8_lossy(&rtn).into_owned())
}

/// Encode a map of strings using null encoding, sorted by key.
/// Format: `key1=val1|key2=val2`
pub fn encode_string_map(m: &HashMap<String, String>) -> Vec<u8> {
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort();
    let mut buf = Vec::new();
    for (idx, key) in keys.iter().enumerate() {
        let val = &m[*key];
        buf.extend_from_slice(&null_encode_str(key));
        buf.push(NULL_ENCODE_EQ_BYTE);
        buf.extend_from_slice(&null_encode_str(val));
        if idx < keys.len() - 1 {
            buf.push(NULL_ENCODE_SEP_BYTE);
        }
    }
    buf
}

/// Decode a null-encoded byte slice into a map of strings.
pub fn decode_string_map(barr: &[u8]) -> Result<HashMap<String, String>, String> {
    if barr.is_empty() {
        return Ok(HashMap::new());
    }
    let mut rtn = HashMap::new();
    for part in split_bytes(barr, NULL_ENCODE_SEP_BYTE) {
        let kv: Vec<&[u8]> = splitn_bytes(part, NULL_ENCODE_EQ_BYTE, 2);
        if kv.len() != 2 {
            return Err(format!("invalid null encoding: {}", String::from_utf8_lossy(part)));
        }
        let key = null_decode_str(kv[0])?;
        let val = null_decode_str(kv[1])?;
        rtn.insert(key, val);
    }
    Ok(rtn)
}

/// Encode a string array using null encoding.
/// Format: `elem1|elem2|elem3`
pub fn encode_string_array(arr: &[String]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (idx, s) in arr.iter().enumerate() {
        buf.extend_from_slice(&null_encode_str(s));
        if idx < arr.len() - 1 {
            buf.push(NULL_ENCODE_SEP_BYTE);
        }
    }
    buf
}

/// Decode a null-encoded byte slice into a string array.
pub fn decode_string_array(barr: &[u8]) -> Result<Vec<String>, String> {
    if barr.is_empty() {
        return Ok(Vec::new());
    }
    let mut rtn = Vec::new();
    for part in split_bytes(barr, NULL_ENCODE_SEP_BYTE) {
        rtn.push(null_decode_str(part)?);
    }
    Ok(rtn)
}

/// Check if an encoded string array has the given first value.
pub fn encoded_string_array_has_first_val(encoded: &[u8], first_key: &str) -> bool {
    let first_key_bytes = null_encode_str(first_key);
    if !encoded.starts_with(&first_key_bytes) {
        return false;
    }
    encoded.len() == first_key_bytes.len() || encoded[first_key_bytes.len()] == NULL_ENCODE_SEP_BYTE
}

/// Get the first value from an encoded string array without decoding the whole array.
pub fn encoded_string_array_get_first_val(encoded: &[u8]) -> String {
    let sep_idx = encoded.iter().position(|&b| b == NULL_ENCODE_SEP_BYTE);
    let slice = match sep_idx {
        Some(idx) => &encoded[..idx],
        None => encoded,
    };
    null_decode_str(slice).unwrap_or_default()
}

// ---- Internal helpers ----

fn split_bytes(data: &[u8], delim: u8) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == delim {
            parts.push(&data[start..i]);
            start = i + 1;
        }
    }
    parts.push(&data[start..]);
    parts
}

fn splitn_bytes(data: &[u8], delim: u8, n: usize) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == delim && parts.len() < n - 1 {
            parts.push(&data[start..i]);
            start = i + 1;
        }
    }
    parts.push(&data[start..]);
    parts
}
