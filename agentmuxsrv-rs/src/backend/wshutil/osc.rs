// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! OSC (Operating System Command) encoding/decoding for terminal RPC messages.
//! Port of Go's `pkg/wshutil/wshutil.go` — OSC escape sequence handling.

#![allow(dead_code)]

use base64::Engine;

/// OSC number for AgentMux client messages.
pub const WAVE_OSC: &str = "23198";
/// OSC number for AgentMux server responses.
pub const WAVE_SERVER_OSC: &str = "23199";

/// BEL character (OSC terminator).
pub const BEL: u8 = 0x07;
/// String Terminator.
pub const ST: u8 = 0x9C;
/// Escape character.
pub const ESC: u8 = 0x1B;

/// Default channel buffer sizes.
pub const DEFAULT_OUTPUT_CH_SIZE: usize = 32;
pub const DEFAULT_INPUT_CH_SIZE: usize = 32;

/// Hex characters for encoding.
const HEX_CHARS: &[u8] = b"0123456789ABCDEF";

/// Make an OSC prefix: ESC ] <oscnum> ;
pub fn make_osc_prefix(osc_num: &str) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(3 + osc_num.len());
    prefix.push(ESC);
    prefix.push(b']');
    prefix.extend_from_slice(osc_num.as_bytes());
    prefix.push(b';');
    prefix
}

/// Encode bytes as a AgentMux OSC escape sequence.
///
/// Format: ESC ] <oscnum> ; <payload> BEL
///
/// If the payload contains control characters, it's base64 encoded.
/// Otherwise, sent as raw JSON.
pub fn encode_wave_osc_bytes(osc_num: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    if osc_num.len() != 5 {
        return Err("osc_num must be 5 characters".to_string());
    }

    let needs_base64 = data.iter().any(|&b| b < 0x20 || b == 0x7F);

    let prefix = make_osc_prefix(osc_num);
    let payload = if needs_base64 {
        base64::engine::general_purpose::STANDARD.encode(data).into_bytes()
    } else {
        data.to_vec()
    };

    let mut result = Vec::with_capacity(prefix.len() + payload.len() + 1);
    result.extend_from_slice(&prefix);
    result.extend_from_slice(&payload);
    result.push(BEL);
    Ok(result)
}

/// Decode a AgentMux OSC escape sequence, returning the payload bytes.
///
/// Strips the OSC prefix and terminator (BEL or ST).
/// If payload starts with '{', it's JSON. Otherwise, base64 decode.
pub fn decode_wave_osc_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 9 {
        return Err("data too short for OSC message".to_string());
    }

    // Verify ESC ]
    if data[0] != ESC || data[1] != b']' {
        return Err("invalid OSC prefix".to_string());
    }

    // Find the semicolon separator
    let sep_pos = data.iter().position(|&b| b == b';')
        .ok_or("missing semicolon in OSC message")?;

    // Find the terminator (BEL or ST)
    let end_pos = data.len() - 1;
    let terminator = data[end_pos];
    if terminator != BEL && terminator != ST {
        return Err(format!("invalid OSC terminator: 0x{:02X}", terminator));
    }

    let payload = &data[sep_pos + 1..end_pos];

    // If starts with '{', it's raw JSON
    if !payload.is_empty() && payload[0] == b'{' {
        return Ok(payload.to_vec());
    }

    // Otherwise, base64 decode
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("base64 decode error: {}", e))
}

/// Check if bytes represent a AgentMux OSC message.
pub fn is_wave_osc(data: &[u8]) -> bool {
    if data.len() < 9 {
        return false;
    }
    data[0] == ESC && data[1] == b']' && (
        data[2..7] == *WAVE_OSC.as_bytes() ||
        data[2..7] == *WAVE_SERVER_OSC.as_bytes()
    )
}

/// Get the OSC number from an OSC message.
pub fn get_osc_num(data: &[u8]) -> Option<&str> {
    if data.len() < 8 || data[0] != ESC || data[1] != b']' {
        return None;
    }
    std::str::from_utf8(&data[2..7]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_json() {
        let json = b"{\"command\":\"test\"}";
        let encoded = encode_wave_osc_bytes(WAVE_OSC, json).unwrap();

        assert_eq!(encoded[0], ESC);
        assert_eq!(encoded[1], b']');
        assert_eq!(*encoded.last().unwrap(), BEL);
        assert!(is_wave_osc(&encoded));

        let decoded = decode_wave_osc_bytes(&encoded).unwrap();
        assert_eq!(decoded, json);
    }

    #[test]
    fn test_encode_decode_base64() {
        let data = b"\x00\x01\x02binary data";
        let encoded = encode_wave_osc_bytes(WAVE_OSC, data).unwrap();
        let decoded = decode_wave_osc_bytes(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_osc_prefix() {
        let prefix = make_osc_prefix(WAVE_OSC);
        assert_eq!(prefix.len(), 8); // ESC + ] + 5 digits + ;
        assert_eq!(prefix[0], ESC);
    }

    #[test]
    fn test_get_osc_num() {
        let encoded = encode_wave_osc_bytes(WAVE_OSC, b"{}").unwrap();
        assert_eq!(get_osc_num(&encoded), Some(WAVE_OSC));
    }

    #[test]
    fn test_invalid_osc_num_length() {
        let result = encode_wave_osc_bytes("123", b"data");
        assert!(result.is_err());
    }
}
