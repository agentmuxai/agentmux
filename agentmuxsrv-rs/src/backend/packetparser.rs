// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Packet parser: framing protocol for JSON packets in a line-oriented stream.
//! Port of Go's pkg/util/packetparser/packetparser.go.
//!
//! Protocol:
//! - Regular lines are passed through as-is
//! - JSON packets are framed with `##N` prefix where N is the byte length
//! - The packet data starts immediately after the N digits
//! - Single-line format: `##N{json}\n`
//! - Multi-line format: `##N{\nmore_data\n...\n`

#![allow(dead_code)]

use std::io::{self, BufRead, Write};

/// Prefix for packet frames.
const PACKET_PREFIX: &str = "##";

/// Try to parse a line as a packet header.
/// If the line starts with `##N` (digits), returns (byte_count, data_after_header).
/// The data starts from the first byte after the digits.
fn parse_packet_start(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
    if !trimmed.starts_with(PACKET_PREFIX) {
        return None;
    }
    let rest = &trimmed[PACKET_PREFIX.len()..];
    // Find where digits end
    let digit_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digit_end == 0 {
        return None;
    }
    let num_str = &rest[..digit_end];
    let count = num_str.parse::<usize>().ok()?;
    let data_part = &rest[digit_end..];
    Some((count, data_part))
}

/// Result of parsing: either a raw line or a complete JSON packet.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedItem {
    /// A regular text line (may include newline).
    Line(String),
    /// A complete JSON packet (the raw bytes).
    Packet(Vec<u8>),
}

/// Packet parser state machine.
pub struct PacketParser {
    /// Pending packet data being accumulated.
    pending: Option<PendingPacket>,
}

struct PendingPacket {
    expected: usize,
    data: Vec<u8>,
}

impl PacketParser {
    pub fn new() -> Self {
        Self { pending: None }
    }

    /// Feed a line to the parser and return any completed items.
    pub fn feed_line(&mut self, line: &str) -> Option<ParsedItem> {
        if let Some(ref mut pending) = self.pending {
            // We're accumulating a multi-line packet
            pending.data.extend_from_slice(line.as_bytes());
            if pending.data.len() >= pending.expected {
                let mut data = std::mem::take(&mut pending.data);
                data.truncate(pending.expected);
                self.pending = None;
                return Some(ParsedItem::Packet(data));
            }
            return None;
        }

        // Check if this is a packet header
        if let Some((expected, data_part)) = parse_packet_start(line) {
            if expected == 0 {
                return Some(ParsedItem::Packet(Vec::new()));
            }

            let mut data = Vec::with_capacity(expected);
            data.extend_from_slice(data_part.as_bytes());

            if data.len() >= expected {
                data.truncate(expected);
                return Some(ParsedItem::Packet(data));
            }

            self.pending = Some(PendingPacket { expected, data });
            return None;
        }

        // Regular line
        Some(ParsedItem::Line(line.to_string()))
    }

    /// Check if the parser is currently accumulating a packet.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl Default for PacketParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse all items from a buffered reader.
pub fn parse_reader<R: BufRead>(reader: R) -> io::Result<Vec<ParsedItem>> {
    let mut parser = PacketParser::new();
    let mut items = Vec::new();

    for line_result in reader.lines() {
        let line = line_result?;
        let line_with_newline = format!("{}\n", line);
        if let Some(item) = parser.feed_line(&line_with_newline) {
            items.push(item);
        }
    }

    Ok(items)
}

/// Write a JSON packet with proper framing.
/// Writes `##N{json_content}\n` where N = len(json_bytes).
/// json_bytes should be the full JSON string (starting with `{`).
pub fn write_packet<W: Write>(writer: &mut W, json_bytes: &[u8]) -> io::Result<()> {
    write!(writer, "{}{}", PACKET_PREFIX, json_bytes.len())?;
    writer.write_all(json_bytes)?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Format a JSON value as a framed packet string.
pub fn format_packet(value: &serde_json::Value) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    format!("{}{}{}\n", PACKET_PREFIX, json.len(), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_packet_start_valid() {
        let (count, data) = parse_packet_start("##10{\"a\":1}").unwrap();
        assert_eq!(count, 10);
        assert_eq!(data, "{\"a\":1}");
    }

    #[test]
    fn test_parse_packet_start_header_only() {
        let (count, data) = parse_packet_start("##20{").unwrap();
        assert_eq!(count, 20);
        assert_eq!(data, "{");
    }

    #[test]
    fn test_parse_packet_start_invalid() {
        assert!(parse_packet_start("hello").is_none());
        assert!(parse_packet_start("##abc{").is_none());
        assert!(parse_packet_start("#10{").is_none());
        assert!(parse_packet_start("").is_none());
        assert!(parse_packet_start("##").is_none());
    }

    #[test]
    fn test_parser_regular_lines() {
        let mut parser = PacketParser::new();
        assert_eq!(
            parser.feed_line("hello\n"),
            Some(ParsedItem::Line("hello\n".to_string()))
        );
        assert_eq!(
            parser.feed_line("world\n"),
            Some(ParsedItem::Line("world\n".to_string()))
        );
    }

    #[test]
    fn test_parser_single_line_packet() {
        let mut parser = PacketParser::new();
        // Packet: ##13{"key":"val"}
        let json = r#"{"key":"val"}"#;
        let line = format!("##{}{}\n", json.len(), json);
        let result = parser.feed_line(&line);
        assert!(result.is_some());
        if let Some(ParsedItem::Packet(data)) = result {
            assert_eq!(String::from_utf8(data).unwrap(), json);
        }
    }

    #[test]
    fn test_parser_multi_line_packet() {
        let mut parser = PacketParser::new();

        // Total data will be: {"key":"value"} = 15 bytes
        // Header line provides "{" (1 byte), need 14 more
        assert!(parser.feed_line("##15{\n").is_none());
        assert!(parser.is_pending());

        // Feed data: "key":"value"} (14 bytes) + \n
        let result = parser.feed_line("\"key\":\"value\"}\n");
        assert!(result.is_some());
        if let Some(ParsedItem::Packet(data)) = result {
            assert_eq!(data.len(), 15);
            assert_eq!(data[0], b'{');
        }
    }

    #[test]
    fn test_parser_mixed_lines_and_packets() {
        let mut parser = PacketParser::new();
        let mut items = Vec::new();

        if let Some(item) = parser.feed_line("line one\n") {
            items.push(item);
        }

        // Single-line packet
        let json = r#"{"ok":true}"#;
        let pkt_line = format!("##{}{}\n", json.len(), json);
        if let Some(item) = parser.feed_line(&pkt_line) {
            items.push(item);
        }

        if let Some(item) = parser.feed_line("line two\n") {
            items.push(item);
        }

        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], ParsedItem::Line(_)));
        assert!(matches!(&items[1], ParsedItem::Packet(_)));
        assert!(matches!(&items[2], ParsedItem::Line(_)));
    }

    #[test]
    fn test_format_packet() {
        let value = serde_json::json!({"key": "val"});
        let pkt = format_packet(&value);
        assert!(pkt.starts_with("##"));
        assert!(pkt.ends_with('\n'));
        // Should be parseable as a single-line packet
        let (count, data) = parse_packet_start(pkt.trim_end()).unwrap();
        assert_eq!(data.len(), count);
    }

    #[test]
    fn test_write_packet() {
        let json = b"{\"hello\":\"world\"}";
        let mut buf = Vec::new();
        write_packet(&mut buf, json).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.starts_with("##"));
        assert!(output.contains("{\"hello\":\"world\"}"));
        assert!(output.ends_with('\n'));
        // Verify the count
        let expected = format!("##17{{\"hello\":\"world\"}}\n");
        assert_eq!(output, expected);
    }

    #[test]
    fn test_format_and_parse_roundtrip() {
        let value = serde_json::json!({"name": "test", "count": 42});
        let pkt = format_packet(&value);

        let mut parser = PacketParser::new();
        let result = parser.feed_line(&pkt);
        assert!(result.is_some());
        if let Some(ParsedItem::Packet(data)) = result {
            let parsed: serde_json::Value =
                serde_json::from_slice(&data).unwrap();
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn test_parse_reader() {
        // Build input with a proper single-line packet
        let json = r#"{"ok":true}"#;
        let input = format!("line1\n##{}{}\nline2\n", json.len(), json);
        let items = parse_reader(io::Cursor::new(input.as_bytes())).unwrap();
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], ParsedItem::Line(s) if s.contains("line1")));
        assert!(matches!(&items[1], ParsedItem::Packet(_)));
        assert!(matches!(&items[2], ParsedItem::Line(s) if s.contains("line2")));
    }

    #[test]
    fn test_parser_empty_packet() {
        let mut parser = PacketParser::new();
        let result = parser.feed_line("##0\n");
        assert_eq!(result, Some(ParsedItem::Packet(Vec::new())));
    }

    #[test]
    fn test_parser_default() {
        let parser = PacketParser::default();
        assert!(!parser.is_pending());
    }

    #[test]
    fn test_write_and_parse_roundtrip() {
        let json = b"{\"test\":123}";
        let mut buf = Vec::new();
        write_packet(&mut buf, json).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let mut parser = PacketParser::new();
        let result = parser.feed_line(&output);
        assert!(result.is_some());
        if let Some(ParsedItem::Packet(data)) = result {
            assert_eq!(data, json);
        }
    }
}
