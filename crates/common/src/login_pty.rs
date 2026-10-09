// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared pieces for running a provider CLI's login under a PTY and reading
//! its OAuth URL out of the output: used by the desktop host
//! (`agentmux-cef` `commands/cli_login.rs`) and by srv's `auth.*`
//! (`agentmux-srv` `server/identity_auth_spawn.rs`,
//! `identity/auth_patterns.rs`). docs/specs/SPEC_PROVIDER_LOGIN_THROUGH_SRV_2026_10_09.md.

/// Columns for a login PTY. Wide enough that the CLI's own line-wrapping
/// never wraps the OAuth URL. At 80 cols it did — Claude Code prints
/// "If the browser didn't open, visit: <url>" as plain text and hard-wraps
/// it mid-query-string, so only a truncated fragment missing `client_id`
/// and everything after it was captured (issue #2429). 4096 comfortably
/// covers any realistic query string.
pub const LOGIN_PTY_COLS: u16 = 4096;

/// Device Status Report cursor-position query — some CLIs (confirmed:
/// Claude Code, on detecting a real TTY) send this immediately and BLOCK
/// waiting for the terminal to reply with `ESC[<row>;<col>R` before
/// printing anything else at all. A bare `portable_pty` handle has no
/// attached terminal emulator to answer this automatically — nothing
/// does, on any platform, without code specifically watching for it — so
/// the child hangs forever and the whole capture loop below times out
/// having seen zero bytes. This is the confirmed root cause of issue
/// #2429 ("no PTY output captured... despite correct binary resolution"):
/// isolated repro (a standalone portable_pty harness spawning the exact
/// same binary/args) showed the child's first and ONLY output was these
/// 4 bytes, then silence: answering this one query unblocked it
/// immediately, and it printed its OAuth URL within half a second.
pub const DSR_CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
/// Synthetic "row 1, col 1" reply. The actual reported position doesn't
/// matter to these CLIs — confirmed by the repro above — they only need
/// *something* to answer so their TTY-capability probe stops blocking.
pub const DSR_CURSOR_POSITION_REPLY: &[u8] = b"\x1b[1;1R";

/// Wraps the raw PTY reader and transparently answers a cursor-position
/// query (see [`DSR_CURSOR_POSITION_QUERY`]) the moment it appears
/// anywhere in the stream — not just at startup, since a TUI could
/// plausibly re-probe after a resize — passing every byte through
/// unchanged to the caller (the query's own bytes included; they're
/// harmless noise to the caller's line-scanning loop, not worth the extra
/// complexity of stripping them from the pass-through).
///
/// Generic over `on_query` (rather than baking in `Arc<AppState>`
/// directly) so the byte-scanning logic — the actually bug-prone part —
/// is unit-testable without a PTY. A caller's closure writes
/// [`DSR_CURSOR_POSITION_REPLY`] through the same writer that delivers a
/// pasted OAuth code, since `portable_pty`'s writer is a single-owner handle.
pub struct DsrRespondingReader<R, F> {
    inner: R,
    on_query: F,
    /// Carry-over from the previous `read()` call — bounded to
    /// `DSR_CURSOR_POSITION_QUERY.len() - 1` bytes — so a query split
    /// across two `read()` calls (e.g. a slow/busy child) is still
    /// detected instead of silently missed at the chunk boundary.
    tail: Vec<u8>,
}

impl<R, F: FnMut()> DsrRespondingReader<R, F> {
    pub fn new(inner: R, on_query: F) -> Self {
        Self {
            inner,
            on_query,
            tail: Vec::new(),
        }
    }
}

impl<R: std::io::Read, F: FnMut()> std::io::Read for DsrRespondingReader<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            return Ok(0);
        }
        self.tail.extend_from_slice(&buf[..n]);
        if self
            .tail
            .windows(DSR_CURSOR_POSITION_QUERY.len())
            .any(|w| w == DSR_CURSOR_POSITION_QUERY)
        {
            (self.on_query)();
        }
        let keep = DSR_CURSOR_POSITION_QUERY.len().saturating_sub(1);
        if self.tail.len() > keep {
            let drop = self.tail.len() - keep;
            self.tail.drain(0..drop);
        }
        Ok(n)
    }
}

/// A line of terminal output with its escape sequences removed: the visible
/// `text`, and the target of every OSC-8 hyperlink it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLine {
    pub text: String,
    pub link_uris: Vec<String>,
}

/// Remove terminal escape sequences from `line`. Two families matter here:
///   * CSI  — `ESC [ … <final 0x40..=0x7e>` (colors, cursor moves)
///   * OSC  — `ESC ] … (BEL | ST)` — an OSC-8 hyperlink embeds the URL in the
///     sequence params AND repeats it as visible link text. A naive pass that
///     only knew CSI left the raw `]8;;https://…<BEL>` in place and captured
///     the URL twice (doubled) whenever OSC-8 IS present, so the OSC sequence
///     is discarded but any URI it carried is kept in `link_uris`.
pub fn strip_terminal_codes(line: &str) -> TerminalLine {
    //   * CSI  — `ESC [ … <final 0x40..=0x7e>` (colors, cursor moves)
    //   * OSC  — `ESC ] … (BEL | ST)` — the Claude CLI can, in principle,
    //     emit an OSC-8 hyperlink here (embeds the URL in the sequence
    //     params AND repeats it as visible link text), though a live
    //     capture under the fixed PTY (see cols comment in run_cli_login)
    //     only ever showed an OSC-0 window-title sequence, no OSC-8 — so
    //     this is defense-in-depth, not the thing that actually fixed
    //     #2429's client_id truncation (the PTY width did). A naive pass
    //     that only knew CSI left the raw `]8;;https://…<BEL>` in place and
    //     captured the URL twice (doubled) whenever OSC-8 IS present, so we
    //     still discard the OSC sequence but stash any URI it carried as a
    //     fallback.
    let mut clean = String::with_capacity(line.len());
    let mut osc_uris: Vec<String> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // CSI: ESC [ … <final byte in 0x40..=0x7e>
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            i += 1; // consume the final byte
        } else if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b']' {
            // OSC: ESC ] … terminated by BEL (0x07) or ST (ESC \).
            let seq_start = i + 2;
            i = seq_start;
            let mut seq_end = bytes.len();
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    seq_end = i;
                    i += 1;
                    break;
                }
                if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                    seq_end = i;
                    i += 2;
                    break;
                }
                i += 1;
            }
            // OSC-8 hyperlink: "8;<params>;<URI>". Stash the URI as a fallback
            // in case the visible link text isn't itself the URL.
            if let Ok(seq) = std::str::from_utf8(&bytes[seq_start..seq_end]) {
                if let Some(rest) = seq.strip_prefix("8;") {
                    if let Some(uri) = rest.splitn(2, ';').nth(1) {
                        if !uri.is_empty() {
                            osc_uris.push(uri.to_string());
                        }
                    }
                }
            }
        } else if bytes[i] == 0x1b {
            // Lone / unrecognised ESC: drop the ESC byte.
            i += 1;
        } else {
            clean.push(bytes[i] as char);
            i += 1;
        }
    }
    TerminalLine {
        text: clean,
        link_uris: osc_uris,
    }
}

/// Extract an OAuth URL from a line of CLI output: an `https://` URL that
/// looks like an auth URL, from an OSC-8 link target first, else from the
/// visible text.
pub fn extract_auth_url(line: &str) -> Option<String> {
    let TerminalLine {
        text: clean,
        link_uris: osc_uris,
    } = strip_terminal_codes(line);
    // Find https:// and extract until whitespace, a quote, or a stray BEL.
    let pick = |s: &str| -> Option<String> {
        let start = s.find("https://")?;
        let rest = &s[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\u{7}')
            .unwrap_or(rest.len());
        let url = &rest[..end];
        if url.contains("oauth") || url.contains("auth") || url.contains("login") {
            Some(url.to_string())
        } else {
            None
        }
    };

    // Prefer any OSC-8 URI: it's carried inside an escape-sequence payload,
    // so it can never be truncated by the terminal's column-width wrapping.
    // The visible text is only a fallback for CLIs that don't emit OSC-8 —
    // it CAN be wrapped mid-URL by the PTY (see issue #2429 follow-up: the
    // plain "If the browser didn't open, visit: ..." line got hard-wrapped
    // at col 80, silently dropping `client_id` from the captured URL).
    osc_uris
        .iter()
        .find_map(|u| pick(u))
        .or_else(|| pick(&clean))
}

#[cfg(test)]
mod dsr_responding_reader_tests {
    use super::{DsrRespondingReader, DSR_CURSOR_POSITION_QUERY};
    use std::cell::Cell;
    use std::io::Read;

    /// Yields each element of `chunks` on successive `read()` calls,
    /// copying as much as fits in the caller's buffer — lets a test force
    /// a specific byte-boundary split, which a plain `std::io::Cursor`
    /// (single-buffer, fills as much as requested in one call) can't do.
    struct ChunkedReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
    }

    impl ChunkedReader {
        fn new(chunks: Vec<&[u8]>) -> Self {
            Self {
                chunks: chunks.into_iter().map(|c| c.to_vec()).collect(),
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some(chunk) = self.chunks.front_mut() else {
                return Ok(0);
            };
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            chunk.drain(..n);
            if chunk.is_empty() {
                self.chunks.pop_front();
            }
            Ok(n)
        }
    }

    #[test]
    fn invokes_callback_when_query_arrives_in_one_read() {
        let inner = ChunkedReader::new(vec![DSR_CURSOR_POSITION_QUERY]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        let n = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], DSR_CURSOR_POSITION_QUERY);
        assert_eq!(
            calls.get(),
            1,
            "callback must fire exactly once for the query"
        );
    }

    #[test]
    fn does_not_invoke_callback_for_unrelated_bytes() {
        let inner = ChunkedReader::new(vec![b"Opening browser to sign in...\r\n"]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        let n = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"Opening browser to sign in...\r\n");
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn detects_query_split_across_two_read_calls() {
        // The exact failure mode a naive single-read scan would miss:
        // "\x1b[6" in one chunk, "n" arriving in the next.
        let inner = ChunkedReader::new(vec![b"\x1b[6", b"n"]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        let n1 = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n1], b"\x1b[6");
        assert_eq!(calls.get(), 0, "must not fire on the incomplete first half");
        let n2 = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n2], b"n");
        assert_eq!(
            calls.get(),
            1,
            "must fire once the second half completes the pattern"
        );
    }

    #[test]
    fn passes_every_byte_through_unchanged_query_included() {
        // The query's own bytes are harmless noise to the line-scanning
        // loop downstream — verify they're never stripped, only observed.
        let payload = [DSR_CURSOR_POSITION_QUERY, b"Paste code here if prompted > "].concat();
        let inner = ChunkedReader::new(vec![&payload]);
        let mut r = DsrRespondingReader::new(inner, || {});
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn does_not_refire_on_subsequent_unrelated_reads_after_the_query() {
        let inner = ChunkedReader::new(vec![DSR_CURSOR_POSITION_QUERY, b"more output\r\n"]);
        let calls = Cell::new(0);
        let mut r = DsrRespondingReader::new(inner, || calls.set(calls.get() + 1));
        let mut buf = [0u8; 64];
        r.read(&mut buf).unwrap();
        assert_eq!(calls.get(), 1);
        r.read(&mut buf).unwrap();
        assert_eq!(
            calls.get(),
            1,
            "later unrelated bytes must not re-trigger the callback"
        );
    }
}

#[cfg(test)]
mod extract_url_claude_authorize_tests {
    use super::extract_auth_url as extract_url;

    // Pins tier-1 URL capture for the revived in-app Claude login
    // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §2): the pinned CLI
    // (2.1.198+) prints the PKCE authorize URL as a plain fallback line —
    // and, in some renderings, OSC-8-hyperlink-wrapped. Both forms were
    // observed in the 2026-08-03 live probes and both must yield the exact
    // URL (not doubled, not truncated) or tier 1 silently falls back to the
    // terminal tiers for a CLI that fully supports the in-app flow.
    const AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc-123&code_challenge=xyz_456&code_challenge_method=S256&state=st-789";

    #[test]
    fn captures_plain_fallback_line() {
        let line = format!("If the browser didn't open, visit: {AUTHORIZE_URL}");
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn captures_osc8_hyperlink_bel_terminated() {
        // OSC-8 with the URL as both the sequence param and the visible link
        // text (what the CLI actually emits) — the de-escaped visible text
        // must win, single and intact.
        let line = format!(
            "If the browser didn't open, visit: \u{1b}]8;;{AUTHORIZE_URL}\u{7}{AUTHORIZE_URL}\u{1b}]8;;\u{7}"
        );
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn captures_osc8_hyperlink_st_terminated_with_non_url_link_text() {
        // ST-terminated OSC-8 whose visible text is NOT the URL — the URI
        // stashed from the escape sequence itself must be used as fallback.
        let line = format!(
            "Visit \u{1b}]8;;{AUTHORIZE_URL}\u{1b}\\this link\u{1b}]8;;\u{1b}\\ to sign in"
        );
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn captures_url_wrapped_in_csi_color_codes() {
        let line = format!("\u{1b}[1m\u{1b}[36m{AUTHORIZE_URL}\u{1b}[0m");
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }

    #[test]
    fn ignores_non_auth_urls() {
        assert_eq!(extract_url("see https://claude.com/docs for details"), None);
    }

    #[test]
    fn prefers_osc8_uri_when_the_visible_fallback_line_was_wrapped_by_the_pty() {
        // Regression for the #2429 follow-up: at an 80-column PTY width, the
        // CLI's own line-wrapping of the plain "visit: ..." text can split
        // the URL mid-query-string before a `\r\n` is ever reached, so the
        // OSC-8 payload (never subject to that wrapping) is the only place
        // the full URL — with client_id intact — actually appears.
        let truncated_visible = "https://claude.com/cai/oauth/authorize?code=t";
        let line = format!(
            "\u{1b}]8;;{AUTHORIZE_URL}\u{7}link text\u{1b}]8;;\u{7}If the browser didn't open, visit: {truncated_visible}"
        );
        assert_eq!(extract_url(&line), Some(AUTHORIZE_URL.to_string()));
    }
}
