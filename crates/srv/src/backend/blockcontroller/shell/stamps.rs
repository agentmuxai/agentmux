// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Receive-time stamps for a range of transcript lines, from the
//! `output.tsidx` sidecar — without loading the sidecar whole.
//!
//! The sidecar holds one record per append, `{"off":<byte offset>,"ms":<unix
//! ms>}\n`, in append order, so `off` rises through the file. A line's stamp is
//! the newest record at or before the line's own byte offset (`0` = unknown).
//!
//! `blockfile:read_range` used to read the whole sidecar and parse every
//! record to stamp ~200 lines: 38 MB and ~1.1M records on a long-lived agent,
//! about 0.6 s per read. Instead, a binary search over the sidecar's bytes
//! finds the record the first line needs, and only a window around the
//! requested range is read and parsed.
//!
//! Records a little out of order (two srv instances mirroring into one global
//! zone, older builds stamping outside the append's transaction) are
//! tolerated: the window reaches [`MARGIN`] bytes past both ends of the range
//! and its records are sorted. A record further out of place than that can be
//! missed, so a line may get an earlier stamp — stamps are best-effort display
//! times, and a miss that far out would need thousands of records out of
//! order.

use crate::backend::storage::error::StoreError;

/// Bytes read before the range's first record and past its last, for
/// out-of-order records (~2,000 records at ~30 bytes each).
pub(crate) const MARGIN: i64 = 64 * 1024;
/// Bytes read at each binary-search probe; a record is ~30 bytes.
const PROBE: i64 = 512;
/// Once the search interval is this small, the window just covers it.
const SEARCH_STOP: i64 = 4096;
/// Chunk size for the window read.
const CHUNK: i64 = 64 * 1024;

#[derive(serde::Deserialize)]
struct Record {
    off: u64,
    ms: i64,
}

fn parse(line: &[u8]) -> Option<(u64, i64)> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    serde_json::from_slice::<Record>(line).ok().map(|r| (r.off, r.ms))
}

/// Stamps for `line_offsets` (ascending byte offsets in `output`), read from a
/// sidecar of `size` bytes through `read(offset, len)` — `None` from `read`
/// means those bytes aren't all stored (a writer mid-append), and so no stamps.
/// `Ok(None)`: no sidecar records near the range at all.
pub(crate) fn stamps_for(
    read: &dyn Fn(i64, i64) -> Result<Option<Vec<u8>>, StoreError>,
    size: i64,
    line_offsets: &[u64],
) -> Result<Option<Vec<i64>>, StoreError> {
    let (Some(&first), Some(&last)) = (line_offsets.first(), line_offsets.last()) else {
        return Ok(Some(Vec::new()));
    };
    if size <= 0 {
        return Ok(None);
    }

    // The latest record boundary whose record is at or before `first`.
    let (mut lo, mut hi) = (0i64, size);
    while hi - lo > SEARCH_STOP {
        let mid = lo + (hi - lo) / 2;
        match first_record_after(read, mid, hi)? {
            Probe::Found { start, off } if off <= first => lo = start,
            Probe::Found { .. } | Probe::NoneBefore => hi = mid,
            Probe::Unreadable => return Ok(None),
        }
    }

    // Everything from MARGIN before that record to MARGIN past the first
    // record beyond `last`.
    let from = (lo - MARGIN).max(0);
    let mut records: Vec<(u64, i64)> = Vec::new();
    let mut pos = from;
    let mut carry: Vec<u8> = Vec::new();
    let mut at_line_start = from == 0;
    let mut stop_at: Option<i64> = None;
    while pos < size && stop_at.map_or(true, |s| pos < s) {
        let len = CHUNK.min(size - pos);
        let Some(chunk) = read(pos, len)? else { return Ok(None) };
        carry.extend_from_slice(&chunk);
        pos += len;
        let Some(last_nl) = carry.iter().rposition(|&b| b == b'\n') else { continue };
        let complete: Vec<u8> = carry.drain(..=last_nl).collect();
        let mut lines = complete.split(|&b| b == b'\n');
        if !at_line_start {
            // The window began mid-record: skip to the first boundary.
            lines.next();
            at_line_start = true;
        }
        for line in lines {
            if let Some((off, ms)) = parse(line) {
                records.push((off, ms));
                if off > last && stop_at.is_none() {
                    stop_at = Some(pos + MARGIN);
                }
            }
        }
    }
    // An unterminated last record (a stamp mid-write) still counts if whole.
    if pos >= size && at_line_start {
        if let Some(rec) = parse(&carry) {
            records.push(rec);
        }
    }
    if records.is_empty() {
        return Ok(None);
    }
    records.sort_by_key(|&(off, _)| off);
    Ok(Some(
        line_offsets
            .iter()
            .map(|&line_off| match records.partition_point(|&(off, _)| off <= line_off) {
                0 => 0,
                p => records[p - 1].1,
            })
            .collect(),
    ))
}

enum Probe {
    /// The first parsable record starting after `pos`.
    Found { start: i64, off: u64 },
    /// No parsable record starts in `[pos, hi)`.
    NoneBefore,
    /// Bytes not stored.
    Unreadable,
}

/// The first parsable record that starts after `pos` (at a line boundary
/// strictly past it) and before `hi`.
fn first_record_after(
    read: &dyn Fn(i64, i64) -> Result<Option<Vec<u8>>, StoreError>,
    pos: i64,
    hi: i64,
) -> Result<Probe, StoreError> {
    let mut at = pos;
    let mut buf: Vec<u8> = Vec::new();
    let mut boundary: Option<i64> = None;
    while at < hi {
        let len = PROBE.min(hi - at);
        let Some(chunk) = read(at, len)? else { return Ok(Probe::Unreadable) };
        buf.extend_from_slice(&chunk);
        at += len;
        let base = pos;
        if boundary.is_none() {
            let Some(nl) = buf.iter().position(|&b| b == b'\n') else { continue };
            boundary = Some(base + nl as i64 + 1);
        }
        // Parse complete records from the boundary on.
        let mut start = boundary.unwrap();
        loop {
            let rel = (start - base) as usize;
            if rel >= buf.len() || start >= hi {
                break;
            }
            let Some(len) = buf[rel..].iter().position(|&b| b == b'\n') else { break };
            if let Some((off, _)) = parse(&buf[rel..rel + len]) {
                return Ok(Probe::Found { start, off });
            }
            start += len as i64 + 1;
        }
        boundary = Some(start);
    }
    Ok(Probe::NoneBefore)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// What `read_range` computed before: every record, sorted.
    fn reference(sidecar: &[u8], line_offsets: &[u64]) -> Option<Vec<i64>> {
        let mut entries: Vec<(u64, i64)> = sidecar.split(|&b| b == b'\n').filter_map(parse).collect();
        if entries.is_empty() {
            return None;
        }
        entries.sort_by_key(|&(off, _)| off);
        Some(
            line_offsets
                .iter()
                .map(|&l| match entries.partition_point(|&(off, _)| off <= l) {
                    0 => 0,
                    p => entries[p - 1].1,
                })
                .collect(),
        )
    }

    fn windowed(sidecar: &[u8], line_offsets: &[u64]) -> Option<Vec<i64>> {
        let read = |off: i64, len: i64| -> Result<Option<Vec<u8>>, StoreError> {
            let (a, b) = (off as usize, ((off + len) as usize).min(sidecar.len()));
            Ok(Some(sidecar[a..b].to_vec()))
        };
        stamps_for(&read, sidecar.len() as i64, line_offsets).unwrap()
    }

    fn sidecar(records: &[(u64, i64)]) -> Vec<u8> {
        records.iter().map(|(o, m)| format!("{{\"off\":{o},\"ms\":{m}}}\n")).collect::<String>().into_bytes()
    }

    #[test]
    fn a_large_sidecar_gives_the_same_stamps_reading_a_small_window() {
        let records: Vec<(u64, i64)> = (0..200_000u64).map(|i| (i * 300, 1_700_000_000_000 + i as i64)).collect();
        let bytes = sidecar(&records);
        let lines: Vec<u64> = (0..200u64).map(|k| 45_000_000 + k * 150).collect();
        let read_bytes = std::cell::Cell::new(0i64);
        let read = |off: i64, len: i64| -> Result<Option<Vec<u8>>, StoreError> {
            read_bytes.set(read_bytes.get() + len);
            let (a, b) = (off as usize, ((off + len) as usize).min(bytes.len()));
            Ok(Some(bytes[a..b].to_vec()))
        };
        let got = stamps_for(&read, bytes.len() as i64, &lines).unwrap();
        assert_eq!(got, reference(&bytes, &lines));
        assert!(read_bytes.get() < 400 * 1024, "read {} of {} bytes", read_bytes.get(), bytes.len());
    }

    #[test]
    fn lines_before_every_record_are_unknown_and_an_empty_sidecar_has_none() {
        let bytes = sidecar(&[(100, 7), (200, 8)]);
        assert_eq!(windowed(&bytes, &[0, 99, 100, 250]), Some(vec![0, 0, 7, 8]));
        assert_eq!(windowed(b"", &[1]), None);
        assert_eq!(windowed(b"garbage\n", &[1]), None);
        assert_eq!(windowed(&bytes, &[]), Some(vec![]));
    }

    #[test]
    fn bytes_not_stored_mean_no_stamps() {
        let bytes = sidecar(&(0..5000u64).map(|i| (i * 10, i as i64)).collect::<Vec<_>>());
        let read = |off: i64, len: i64| -> Result<Option<Vec<u8>>, StoreError> {
            Ok((off + len < 60_000).then(|| bytes[off as usize..(off + len) as usize].to_vec()))
        };
        assert_eq!(stamps_for(&read, bytes.len() as i64, &[49_000]).unwrap(), None);
    }

    proptest! {
        /// Sorted sidecars with local disorder, garbage lines, CRLF and an
        /// unterminated last record: the windowed read matches the full parse.
        #[test]
        fn windowed_stamps_equal_the_full_parse(
            n in 1usize..6000,
            gaps in prop::collection::vec(0u64..400, 1..64),
            swaps in prop::collection::vec(0usize..6000, 0..40),
            garbage in prop::collection::vec(0usize..6000, 0..10),
            crlf in any::<bool>(),
            unterminated in any::<bool>(),
            picks in prop::collection::vec(0u64..2_500_000, 1..60),
        ) {
            let mut off = 0u64;
            let mut records: Vec<(u64, i64)> = (0..n).map(|i| { off += gaps[i % gaps.len()]; (off, i as i64 + 1) }).collect();
            // Local disorder: neighbours swapped.
            for s in swaps { if s + 1 < records.len() { records.swap(s, s + 1); } }
            let mut text = String::new();
            for (i, (o, m)) in records.iter().enumerate() {
                if garbage.contains(&i) { text.push_str("{\"off\":oops\n"); }
                text.push_str(&format!("{{\"off\":{o},\"ms\":{m}}}"));
                text.push_str(if crlf { "\r\n" } else { "\n" });
            }
            if unterminated { let t = text.trim_end().len(); text.truncate(t); }
            let bytes = text.into_bytes();
            let mut lines = picks.clone();
            lines.sort_unstable();
            // A read_range asks for one contiguous run of lines: bound the span.
            let lo = lines[0];
            let lines: Vec<u64> = lines.into_iter().filter(|&l| l - lo < 20_000).collect();
            prop_assert_eq!(windowed(&bytes, &lines), reference(&bytes, &lines));
        }
    }
}
