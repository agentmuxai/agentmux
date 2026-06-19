// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Stateful byte-stream OSC sequence extractor.
//!
//! Parses OSC 0 and OSC 2 sequences from PTY output and normalises
//! Claude Code window-title payloads into bare conversation-topic strings.
//! Used by the agent-pane PTY read loop to surface the session topic as
//! `term:activity` block metadata without leaking raw escape bytes into
//! the FileStore.
//!
//! Design notes:
//! - State machine buffers across PTY read() calls so sequences split
//!   at chunk boundaries are assembled correctly.
//! - Buffer is capped at MAX_PAYLOAD_BYTES; overflow discards the
//!   partial sequence and resets state (no unbounded memory growth).
//! - Non-UTF-8 bytes are replaced with U+FFFD via from_utf8_lossy.
//! - Terminal panes use xterm.js to handle OSC natively; this extractor
//!   is applied ONLY to agent-pane PTY streams.

const MAX_PAYLOAD_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
enum State {
    Idle,
    AfterEsc,
    InOsc,
    InOscAfterEsc,
}

/// A complete, normalised OSC title event.
pub struct OscEvent {
    /// OSC parameter number (0 or 2).
    pub ps: u16,
    /// Payload with Claude Code prefixes stripped and bare startup titles discarded.
    pub payload: String,
}

/// Stateful OSC extractor — create one instance per PTY stream and call
/// `feed()` for each chunk.
pub struct OscExtractor {
    state: State,
    payload_buf: Vec<u8>,
}

impl OscExtractor {
    pub fn new() -> Self {
        OscExtractor {
            state: State::Idle,
            payload_buf: Vec::new(),
        }
    }

    /// Process one PTY chunk.
    ///
    /// Returns `(cleaned_bytes, events)`:
    /// - `cleaned_bytes`: the input with all OSC sequences removed; write
    ///   this to FileStore in place of the raw chunk.
    /// - `events`: any complete OSC 0/2 title events found (payloads already
    ///   normalised; empty payload strings are never emitted).
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<OscEvent>) {
        let mut out = Vec::with_capacity(chunk.len());
        let mut events = Vec::new();

        let mut i = 0;
        while i < chunk.len() {
            let byte = chunk[i];
            i += 1;

            match self.state {
                State::Idle => {
                    if byte == 0x1b {
                        self.state = State::AfterEsc;
                    } else {
                        out.push(byte);
                    }
                }
                State::AfterEsc => {
                    if byte == 0x5d {
                        // ESC ] — OSC start
                        self.state = State::InOsc;
                        self.payload_buf.clear();
                    } else if byte == 0x1b {
                        // Another ESC — emit previous ESC verbatim, stay in AfterEsc
                        out.push(0x1b);
                    } else {
                        // Not an OSC — emit ESC + this byte verbatim
                        out.push(0x1b);
                        out.push(byte);
                        self.state = State::Idle;
                    }
                }
                State::InOsc => {
                    if byte == 0x07 {
                        // BEL terminator — sequence complete
                        if let Some(ev) = self.complete_osc() {
                            events.push(ev);
                        }
                        self.state = State::Idle;
                    } else if byte == 0x1b {
                        // Possible ST start (ESC \)
                        self.state = State::InOscAfterEsc;
                    } else if self.payload_buf.len() < MAX_PAYLOAD_BYTES {
                        self.payload_buf.push(byte);
                    } else {
                        // Buffer overflow — discard partial sequence and reset
                        self.payload_buf.clear();
                        self.state = State::Idle;
                        continue;
                    }
                }
                State::InOscAfterEsc => {
                    if byte == 0x5c {
                        // ST (ESC \) — sequence complete
                        if let Some(ev) = self.complete_osc() {
                            events.push(ev);
                        }
                        self.state = State::Idle;
                    } else {
                        // ESC was part of payload, not ST — push it and reprocess byte
                        if self.payload_buf.len() < MAX_PAYLOAD_BYTES {
                            self.payload_buf.push(0x1b);
                        } else {
                            self.payload_buf.clear();
                            self.state = State::Idle;
                            continue;
                        }
                        self.state = State::InOsc;
                        // Reprocess current byte in InOsc
                        if byte == 0x07 {
                            if let Some(ev) = self.complete_osc() {
                                events.push(ev);
                            }
                            self.state = State::Idle;
                        } else if byte == 0x1b {
                            self.state = State::InOscAfterEsc;
                        } else if self.payload_buf.len() < MAX_PAYLOAD_BYTES {
                            self.payload_buf.push(byte);
                        } else {
                            self.payload_buf.clear();
                            self.state = State::Idle;
                        }
                    }
                }
            }
        }

        (out, events)
    }

    fn complete_osc(&mut self) -> Option<OscEvent> {
        let raw = std::mem::take(&mut self.payload_buf);
        let s = String::from_utf8_lossy(&raw);

        // OSC payload format: "<ps>;<data>"
        let semicolon = s.find(';')?;
        let ps_str = &s[..semicolon];
        let ps: u16 = ps_str.parse().ok()?;

        // Only OSC 0 and 2 carry window title
        if ps != 0 && ps != 2 {
            return None;
        }

        let data = &s[semicolon + 1..];
        let payload = normalise_title(data);
        if payload.is_empty() {
            return None;
        }

        Some(OscEvent { ps, payload })
    }
}

/// Strip Claude Code title prefixes; discard bare startup/idle titles.
///
/// Observed formats (GitHub issues #21677, #23355, #27197):
///   "claude - auth refactor"  → "auth refactor"
///   "Claude: editing auth.rs" → "editing auth.rs"
///   "Claude Code: summary"    → "summary"
///   "claude"                  → discard (startup idle title, no topic)
///   "Claude Code"             → discard (post-launch before topic, no topic)
fn normalise_title(s: &str) -> String {
    let stripped = if let Some(rest) = s.strip_prefix("claude - ") {
        rest
    } else if let Some(rest) = s.strip_prefix("Claude - ") {
        rest
    } else if let Some(rest) = s.strip_prefix("Claude: ") {
        rest
    } else if let Some(rest) = s.strip_prefix("Claude Code: ") {
        rest
    } else {
        s
    };

    let trimmed = stripped.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("claude")
        || trimmed.eq_ignore_ascii_case("claude code")
    {
        return String::new();
    }

    trimmed.to_string()
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_str(ext: &mut OscExtractor, s: &str) -> (String, Vec<String>) {
        let (cleaned, events) = ext.feed(s.as_bytes());
        (
            String::from_utf8_lossy(&cleaned).into_owned(),
            events.into_iter().map(|e| e.payload).collect(),
        )
    }

    #[test]
    fn bel_terminator() {
        let mut ext = OscExtractor::new();
        let (cleaned, evs) = feed_str(&mut ext, "\x1b]0;claude - auth refactor\x07hello");
        assert_eq!(cleaned, "hello");
        assert_eq!(evs, ["auth refactor"]);
    }

    #[test]
    fn st_terminator() {
        let mut ext = OscExtractor::new();
        let (cleaned, evs) = feed_str(&mut ext, "\x1b]0;Claude: editing auth.rs\x1b\\hello");
        assert_eq!(cleaned, "hello");
        assert_eq!(evs, ["editing auth.rs"]);
    }

    #[test]
    fn osc2_handled() {
        let mut ext = OscExtractor::new();
        let (_, evs) = feed_str(&mut ext, "\x1b]2;Claude Code: summary\x07");
        assert_eq!(evs, ["summary"]);
    }

    #[test]
    fn non_title_osc_stripped_no_event() {
        let mut ext = OscExtractor::new();
        let (cleaned, evs) = feed_str(&mut ext, "\x1b]7;file:///home/user\x07text");
        assert_eq!(cleaned, "text");
        assert!(evs.is_empty());
    }

    #[test]
    fn bare_startup_title_discarded() {
        for title in &["claude", "CLAUDE", "Claude Code", "claude code", "CLAUDE CODE"] {
            let input = format!("\x1b]0;{}\x07", title);
            let mut ext = OscExtractor::new();
            let (_, evs) = ext.feed(input.as_bytes());
            assert!(evs.is_empty(), "expected discard for '{title}' but got event");
        }
    }

    #[test]
    fn cross_chunk_split_bel() {
        let mut ext = OscExtractor::new();
        // Split right before BEL
        let (_, evs1) = ext.feed(b"\x1b]0;claude - auth refactor");
        assert!(evs1.is_empty());
        let (cleaned, evs2) = ext.feed(b"\x07rest");
        assert_eq!(evs2.iter().map(|e| e.payload.as_str()).collect::<Vec<_>>(), ["auth refactor"]);
        assert_eq!(String::from_utf8_lossy(&cleaned), "rest");
    }

    #[test]
    fn cross_chunk_split_at_every_byte() {
        let input = b"\x1b]0;claude - topic\x07";
        // Split at every possible byte offset
        for split in 1..input.len() {
            let mut ext = OscExtractor::new();
            let (_, evs1) = ext.feed(&input[..split]);
            let (_, evs2) = ext.feed(&input[split..]);
            let all_evs: Vec<_> = evs1.iter().chain(evs2.iter()).map(|e| e.payload.as_str()).collect();
            assert_eq!(all_evs, ["topic"], "split at byte {split} failed");
        }
    }

    #[test]
    fn buffer_overflow_guard() {
        let mut ext = OscExtractor::new();
        // Payload larger than MAX_PAYLOAD_BYTES — should not panic; state resets
        let big: Vec<u8> = std::iter::once(b'\x1b')
            .chain(std::iter::once(b']'))
            .chain(b"0;".iter().copied())
            .chain(vec![b'x'; MAX_PAYLOAD_BYTES + 10])
            .chain(std::iter::once(b'\x07'))
            .collect();
        let (_, evs) = ext.feed(&big);
        assert!(evs.is_empty());
        // Extractor should work normally after overflow
        let (_, evs2) = ext.feed(b"\x1b]0;claude - recovery\x07");
        assert_eq!(evs2.iter().map(|e| e.payload.as_str()).collect::<Vec<_>>(), ["recovery"]);
    }

    #[test]
    fn non_utf8_replaced() {
        let mut ext = OscExtractor::new();
        // "claude - " prefix + invalid UTF-8 byte 0xFF
        let payload: Vec<u8> = b"\x1b]0;claude - topic\xff\x07".to_vec();
        let (_, evs) = ext.feed(&payload);
        // Should produce an event (not panic); payload contains replacement char
        assert_eq!(evs.len(), 1);
        assert!(evs[0].payload.contains("topic"));
    }

    #[test]
    fn passthrough_bytes_unchanged() {
        let mut ext = OscExtractor::new();
        let (cleaned, _) = ext.feed(b"hello world");
        assert_eq!(cleaned, b"hello world");
    }

    #[test]
    fn esc_not_followed_by_bracket_passed_through() {
        let mut ext = OscExtractor::new();
        let (cleaned, evs) = feed_str(&mut ext, "\x1b[32mgreen\x1b[0m");
        // CSI sequences (ESC [) should pass through unchanged
        assert_eq!(cleaned, "\x1b[32mgreen\x1b[0m");
        assert!(evs.is_empty());
    }
}
