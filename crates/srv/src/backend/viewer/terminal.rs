// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pure pieces of a paired device's terminal feed
//! (`GET /agentmux/viewer/terminals/:block_id/feed`, the route in
//! `server::http_viewer_terminal`): where a snapshot of a terminal's output starts,
//! and how each published append moves the feed's byte cursor.
//!
//! A terminal's output is the block's `term` file: the PTY's raw bytes, ANSI
//! escapes and all, appended in order. Each append is published with the
//! byte offset it is written at, before it is written, so a feed that
//! subscribes, then reads the file's size, can place every later event: an
//! event that ends at or before the cursor was in the snapshot, one that
//! starts past it means bytes were missed.

/// How much of a terminal's output a snapshot holds, at most.
pub const SNAPSHOT_TAIL_BYTES: u64 = 256 * 1024;
/// How far into a tail a snapshot looks for a line start.
const LINE_START_WINDOW: usize = 64 * 1024;

/// Where a snapshot of `data`, read from byte `offset` of the file, starts:
/// right after its first newline when it doesn't start the file, so the
/// device's terminal doesn't open in the middle of an escape sequence or a
/// character; all of it when it starts the file, or has no newline early on.
pub fn tail_start(offset: u64, data: &[u8]) -> usize {
    if offset == 0 {
        return 0;
    }
    let window = &data[..data.len().min(LINE_START_WINDOW)];
    window.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1)
}

/// What to do with one published append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermStep {
    /// Send these bytes, written at `offset`; the cursor has moved past them.
    Send { offset: u64, data: Vec<u8> },
    /// Already sent (the snapshot held them).
    Skip,
    /// The feed can't continue from here: reset and send a fresh snapshot.
    Resync(&'static str),
}

/// Apply one published append, written at `offset`, to a cursor at `next`.
pub fn apply_term_append(next: &mut u64, offset: Option<u64>, data: &[u8]) -> TermStep {
    let Some(offset) = offset else {
        // A writer that didn't say where: nothing to place it by.
        return TermStep::Resync("offset");
    };
    let end = offset + data.len() as u64;
    if end <= *next {
        return TermStep::Skip;
    }
    if offset > *next {
        return TermStep::Resync("gap");
    }
    let skip = (*next - offset) as usize;
    let at = *next;
    *next = end;
    TermStep::Send { offset: at, data: data[skip..].to_vec() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_starts_after_its_first_newline_unless_it_starts_the_file() {
        assert_eq!(tail_start(0, b"\x1b[1mbold\nnext"), 0, "the file's start");
        assert_eq!(tail_start(10, b"[1mbold\nnext\n"), 8, "mid escape: from the next line");
        assert_eq!(tail_start(10, b"no newline at all"), 0);
        let mut late = vec![b'x'; LINE_START_WINDOW];
        late.extend_from_slice(b"\nrest");
        assert_eq!(tail_start(10, &late), 0, "a newline past the window isn't looked for");
    }

    #[test]
    fn appends_continue_the_cursor_and_drop_what_was_sent() {
        let mut next = 10;
        assert_eq!(apply_term_append(&mut next, Some(10), b"abc"), TermStep::Send { offset: 10, data: b"abc".to_vec() });
        assert_eq!(next, 13);
        assert_eq!(apply_term_append(&mut next, Some(8), b"xyz"), TermStep::Skip, "ends at the cursor");
        assert_eq!(apply_term_append(&mut next, Some(11), b"cdef"), TermStep::Send { offset: 13, data: b"ef".to_vec() });
        assert_eq!(next, 15);
    }

    #[test]
    fn a_gap_or_an_unplaced_append_resyncs() {
        let mut next = 10;
        assert_eq!(apply_term_append(&mut next, Some(11), b"x"), TermStep::Resync("gap"));
        assert_eq!(apply_term_append(&mut next, None, b"x"), TermStep::Resync("offset"));
        assert_eq!(next, 10, "nothing moved");
    }
}
