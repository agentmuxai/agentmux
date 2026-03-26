// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use super::{MAX_MESSAGE_LENGTH, TRUNCATION_SUFFIX};

/// Sanitize a message by removing dangerous escape sequences and control characters.
///
/// 1. Removes ANSI escape sequences
/// 2. Removes OSC sequences (terminal commands)
/// 3. Removes CSI sequences
/// 4. Removes control characters except \n, \t, \r
/// 5. Truncates to MAX_MESSAGE_LENGTH with UTF-8 safety
pub fn sanitize_message(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len());

    let bytes = msg.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Check for ESC sequences
        if b == 0x1b && i + 1 < len {
            let next = bytes[i + 1];

            // CSI sequence: ESC [ ... <final byte>
            if next == b'[' {
                i += 2;
                while i < len && !(bytes[i] >= 0x40 && bytes[i] <= 0x7e) {
                    i += 1;
                }
                if i < len {
                    i += 1; // skip final byte
                }
                continue;
            }

            // OSC sequence: ESC ] ... BEL
            if next == b']' {
                i += 2;
                while i < len && bytes[i] != 0x07 {
                    // Also check for ST (ESC \)
                    if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                if i < len && bytes[i] == 0x07 {
                    i += 1;
                }
                continue;
            }

            // Other ESC sequences (2-byte)
            i += 2;
            continue;
        }

        // Remove control characters except whitespace
        if b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t' {
            i += 1;
            continue;
        }

        // DEL character
        if b == 0x7f {
            i += 1;
            continue;
        }

        // Keep printable characters and valid UTF-8
        if b < 0x80 {
            result.push(b as char);
            i += 1;
        } else {
            // UTF-8 multi-byte: determine sequence length
            let seq_len = if b >= 0xF0 {
                4
            } else if b >= 0xE0 {
                3
            } else if b >= 0xC0 {
                2
            } else {
                // Invalid continuation byte, skip
                i += 1;
                continue;
            };

            if i + seq_len <= len {
                let s = std::str::from_utf8(&bytes[i..i + seq_len]);
                if let Ok(valid) = s {
                    result.push_str(valid);
                }
                i += seq_len;
            } else {
                // Incomplete sequence
                i += 1;
            }
        }
    }

    // Truncate to max length, preserving UTF-8
    if result.len() > MAX_MESSAGE_LENGTH {
        let suffix_len = TRUNCATION_SUFFIX.len();
        let target = MAX_MESSAGE_LENGTH - suffix_len;
        // Find a valid UTF-8 boundary
        let mut end = target;
        while end > 0 && !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
        result.push_str(TRUNCATION_SUFFIX);
    }

    result
}

/// Validate an agent ID.
///
/// Must be 1-64 characters, only letters, digits, underscore, and hyphen.
pub fn validate_agent_id(agent_id: &str) -> bool {
    if agent_id.is_empty() || agent_id.len() > 64 {
        return false;
    }
    agent_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Format a message with optional source agent prefix.
pub fn format_injected_message(msg: &str, source_agent: Option<&str>, include_source: bool) -> String {
    if include_source {
        if let Some(source) = source_agent {
            if !source.is_empty() {
                return format!("@{}: {}", source, msg);
            }
        }
    }
    msg.to_string()
}

/// Validate an AgentMux URL for SSRF protection.
///
/// Only allows https:// or http://localhost/127.0.0.1/::1.
pub fn validate_agentmux_url(url_str: &str) -> Result<(), String> {
    if url_str.is_empty() {
        return Err("URL is empty".to_string());
    }

    // Parse URL
    if let Some(scheme_end) = url_str.find("://") {
        let scheme = &url_str[..scheme_end];
        let rest = &url_str[scheme_end + 3..];

        match scheme {
            "https" => Ok(()),
            "http" => {
                // Extract host (before port or path)
                let authority = rest.split('/').next().unwrap_or("");
                let host = if authority.starts_with('[') {
                    // IPv6 bracketed: [::1]:port
                    authority.split(']').next().unwrap_or("")
                } else {
                    authority.split(':').next().unwrap_or("")
                };
                // Normalize: strip brackets for comparison
                let host_clean = host.trim_start_matches('[').trim_end_matches(']');

                match host_clean {
                    "localhost" | "127.0.0.1" | "::1" => Ok(()),
                    _ => Err(format!(
                        "http URLs only allowed for localhost, got host: {}",
                        host_clean
                    )),
                }
            }
            _ => Err(format!("unsupported URL scheme: {}", scheme)),
        }
    } else {
        Err("invalid URL: missing scheme".to_string())
    }
}
