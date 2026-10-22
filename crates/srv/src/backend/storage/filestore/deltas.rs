// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Codec 2 of a frame: the streaming deltas cleaned out
//! (SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md §8).
//!
//! A reply streams as one `stream_event` line per few characters, each line
//! mostly the same envelope around a little text:
//!
//! ```text
//! {"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}},"session_id":"…","parent_tool_use_id":null,"uuid":"…"}
//! ```
//!
//! This codec stores each such line as just its uuid (16 bytes) and the
//! length of its text, and the text of all the lines of one content block
//! once, end to end. A line is taken only if rendering it back from those
//! values gives exactly its bytes; anything else, and every other line, is
//! kept as it was. Nothing is parsed into a map and written again, and no
//! text is derived from another line, so decoding gives back every byte. The
//! encoder decodes what it produced and compares before it is used.
//!
//! Payload (then compressed with zstd by `frames.rs`), all integers LEB128:
//! `"AMD2"`, the strings (session ids, `parent_tool_use_id` values as they
//! appear, quotes included or `null`), the blocks (session, parent, index,
//! kind, text), then the records in order: `0` raw bytes, or `1` a delta
//! line (block, text length, uuid).

use std::collections::HashMap;

const MAGIC: &[u8; 4] = b"AMD2";
const TAG_RAW: u8 = 0;
const TAG_DELTA: u8 = 1;

/// The two delta kinds this codec takes: (delta `type`, its text field).
const KINDS: [(&[u8], &[u8]); 2] = [(b"text_delta", b"text"), (b"input_json_delta", b"partial_json")];

const P1: &[u8] = br#"{"type":"stream_event","event":{"type":"content_block_delta","index":"#;
const P2: &[u8] = br#","delta":{"type":""#;
const P3: &[u8] = br#"",""#;
const P4: &[u8] = br#"":""#;
const P5: &[u8] = br#""}},"session_id":""#;
const P6: &[u8] = br#"","parent_tool_use_id":"#;
const P7: &[u8] = br#","uuid":""#;
const P8: &[u8] = b"\"}\n";

/// One delta line, taken apart.
#[derive(Debug, PartialEq, Eq)]
struct Delta<'a> {
    index: u64,
    kind: u8,
    text: &'a [u8],
    session: &'a [u8],
    parent: &'a [u8],
    uuid: [u8; 16],
}

fn render(out: &mut Vec<u8>, index: u64, kind: u8, text: &[u8], session: &[u8], parent: &[u8], uuid: &[u8; 16]) {
    let (ty, field) = KINDS[kind as usize];
    out.extend_from_slice(P1);
    out.extend_from_slice(index.to_string().as_bytes());
    out.extend_from_slice(P2);
    out.extend_from_slice(ty);
    out.extend_from_slice(P3);
    out.extend_from_slice(field);
    out.extend_from_slice(P4);
    out.extend_from_slice(text);
    out.extend_from_slice(P5);
    out.extend_from_slice(session);
    out.extend_from_slice(P6);
    out.extend_from_slice(parent);
    out.extend_from_slice(P7);
    push_uuid(out, uuid);
    out.extend_from_slice(P8);
}

fn take<'a>(s: &mut &'a [u8], lit: &[u8]) -> Option<()> {
    if s.starts_with(lit) {
        *s = &s[lit.len()..];
        Some(())
    } else {
        None
    }
}

/// A JSON string's body up to its closing quote (not consumed).
fn string_body<'a>(s: &mut &'a [u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'"' => {
                let body = &s[..i];
                *s = &s[i..];
                return Some(body);
            }
            b'\\' => i += 2,
            b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// A canonical lowercase uuid as 16 bytes.
fn parse_uuid(s: &[u8]) -> Option<[u8; 16]> {
    if s.len() != 36 {
        return None;
    }
    // A dash at each of the four places and a hex digit everywhere else, so
    // there are exactly 32 digits.
    let mut out = [0u8; 16];
    let mut digit = 0;
    for (at, &c) in s.iter().enumerate() {
        if matches!(at, 8 | 13 | 18 | 23) {
            if c != b'-' {
                return None;
            }
            continue;
        }
        out[digit / 2] |= hex(c)? << if digit % 2 == 0 { 4 } else { 0 };
        digit += 1;
    }
    Some(out)
}

fn push_uuid(out: &mut Vec<u8>, u: &[u8; 16]) {
    const H: &[u8; 16] = b"0123456789abcdef";
    for (i, b) in u.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push(b'-');
        }
        out.push(H[(b >> 4) as usize]);
        out.push(H[(b & 15) as usize]);
    }
}

/// Take a line (with its `\n`) apart if it is a delta line this codec
/// stores, and only if rendering it back gives exactly its bytes.
fn parse_delta(line: &[u8]) -> Option<Delta<'_>> {
    let mut s = line;
    take(&mut s, P1)?;
    let digits = s.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let index: u64 = std::str::from_utf8(&s[..digits]).ok()?.parse().ok()?;
    s = &s[digits..];
    take(&mut s, P2)?;
    let kind = KINDS.iter().position(|(ty, _)| s.starts_with(ty))? as u8;
    let (ty, field) = KINDS[kind as usize];
    s = &s[ty.len()..];
    take(&mut s, P3)?;
    take(&mut s, field)?;
    take(&mut s, P4)?;
    let text = string_body(&mut s)?;
    take(&mut s, P5)?;
    let session = string_body(&mut s)?;
    if session.contains(&b'\\') {
        return None;
    }
    take(&mut s, P6)?;
    let parent = if s.starts_with(b"null") {
        &s[..4]
    } else if s.first() == Some(&b'"') {
        let mut rest = &s[1..];
        let body = string_body(&mut rest)?;
        if body.contains(&b'\\') {
            return None;
        }
        &s[..body.len() + 2]
    } else {
        return None;
    };
    s = &s[parent.len()..];
    take(&mut s, P7)?;
    if s.len() < 36 {
        return None;
    }
    let uuid = parse_uuid(&s[..36])?;
    s = &s[36..];
    if s != P8 {
        return None;
    }
    let delta = Delta { index, kind, text, session, parent, uuid };
    let mut back = Vec::with_capacity(line.len());
    render(&mut back, delta.index, delta.kind, delta.text, delta.session, delta.parent, &delta.uuid);
    (back == line).then_some(delta)
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

struct Reader<'a> {
    s: &'a [u8],
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let (&b, rest) = self.s.split_first().ok_or("payload ends inside a number")?;
            self.s = rest;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("number too long".to_string())
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.s.len() < n {
            return Err("payload ends inside a field".to_string());
        }
        let (head, rest) = self.s.split_at(n);
        self.s = rest;
        Ok(head)
    }
    fn len_prefixed(&mut self) -> Result<&'a [u8], String> {
        let n = self.varint()? as usize;
        self.bytes(n)
    }
}

/// The id of `s` in the string table, adding it the first time.
fn intern<'r>(s: &'r [u8], strings: &mut Vec<&'r [u8]>, ids: &mut HashMap<&'r [u8], u64>) -> u64 {
    if let Some(&id) = ids.get(s) {
        return id;
    }
    let id = strings.len() as u64;
    strings.push(s);
    ids.insert(s, id);
    id
}

/// What [`encode`] left out.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Delta lines stored as their uuid and length.
    pub lines: usize,
    /// Their bytes in the original.
    pub bytes: usize,
}

/// The payload for a frame's bytes, or `None` when it has no delta line to
/// leave out or the payload doesn't decode back to exactly `raw` (then the
/// frame is stored with codec 1).
pub(crate) fn encode(raw: &[u8]) -> Option<(Vec<u8>, Stats)> {
    let mut strings: Vec<&[u8]> = Vec::new();
    let mut string_ids: HashMap<&[u8], u64> = HashMap::new();
    // Blocks by (session, parent, index, kind), in order of first appearance.
    let mut block_ids: HashMap<(u64, u64, u64, u8), usize> = HashMap::new();
    let mut blocks: Vec<((u64, u64, u64, u8), Vec<u8>)> = Vec::new();
    enum Rec {
        Raw(usize, usize),
        Delta { block: usize, len: usize, uuid: [u8; 16] },
    }
    let mut recs: Vec<Rec> = Vec::new();
    let mut stats = Stats::default();
    let mut start = 0usize;
    while start < raw.len() {
        let end = raw[start..].iter().position(|&b| b == b'\n').map_or(raw.len(), |i| start + i + 1);
        let line = &raw[start..end];
        // The frame's first chunk may be the end of a line that began in the
        // frame before; a delta line never parses from such a piece anyway.
        match parse_delta(line) {
            Some(d) => {
                let session = intern(d.session, &mut strings, &mut string_ids);
                let parent = intern(d.parent, &mut strings, &mut string_ids);
                let key = (session, parent, d.index, d.kind);
                let block = *block_ids.entry(key).or_insert_with(|| {
                    blocks.push((key, Vec::new()));
                    blocks.len() - 1
                });
                blocks[block].1.extend_from_slice(d.text);
                recs.push(Rec::Delta { block, len: d.text.len(), uuid: d.uuid });
                stats.lines += 1;
                stats.bytes += line.len();
            }
            None => match recs.last_mut() {
                Some(Rec::Raw(_, e)) if *e == start => *e = end,
                _ => recs.push(Rec::Raw(start, end)),
            },
        }
        start = end;
    }
    if stats.lines == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    out.extend_from_slice(MAGIC);
    put_varint(&mut out, strings.len() as u64);
    for s in &strings {
        put_varint(&mut out, s.len() as u64);
        out.extend_from_slice(s);
    }
    put_varint(&mut out, blocks.len() as u64);
    for ((session, parent, index, kind), text) in &blocks {
        put_varint(&mut out, *session);
        put_varint(&mut out, *parent);
        put_varint(&mut out, *index);
        out.push(*kind);
        put_varint(&mut out, text.len() as u64);
        out.extend_from_slice(text);
    }
    put_varint(&mut out, recs.len() as u64);
    for rec in &recs {
        match rec {
            Rec::Raw(s, e) => {
                out.push(TAG_RAW);
                put_varint(&mut out, (e - s) as u64);
                out.extend_from_slice(&raw[*s..*e]);
            }
            Rec::Delta { block, len, uuid } => {
                out.push(TAG_DELTA);
                put_varint(&mut out, *block as u64);
                put_varint(&mut out, *len as u64);
                out.extend_from_slice(uuid);
            }
        }
    }
    // The operator's rule: what is stored must give back every byte.
    match decode(&out, raw.len()) {
        Ok(back) if back == raw => Some((out, stats)),
        Ok(_) => {
            tracing::warn!("filestore: a frame's deltas didn't rebuild exactly; kept as they were");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "filestore: a frame's deltas didn't decode; kept as they were");
            None
        }
    }
}

/// Rebuild a frame's bytes from its payload. `raw_len` bounds the output.
pub(crate) fn decode(payload: &[u8], raw_len: usize) -> Result<Vec<u8>, String> {
    let mut r = Reader { s: payload };
    if r.bytes(4)? != MAGIC {
        return Err("not a delta payload".to_string());
    }
    let n_strings = r.varint()? as usize;
    let mut strings = Vec::with_capacity(n_strings.min(4096));
    for _ in 0..n_strings {
        strings.push(r.len_prefixed()?);
    }
    let n_blocks = r.varint()? as usize;
    let mut blocks = Vec::with_capacity(n_blocks.min(65536));
    for _ in 0..n_blocks {
        let session = *strings.get(r.varint()? as usize).ok_or("bad string")?;
        let parent = *strings.get(r.varint()? as usize).ok_or("bad string")?;
        let index = r.varint()?;
        let kind = r.bytes(1)?[0];
        if kind as usize >= KINDS.len() {
            return Err("bad kind".to_string());
        }
        let text = r.len_prefixed()?;
        blocks.push((session, parent, index, kind, text, 0usize));
    }
    let n_recs = r.varint()? as usize;
    let mut out = Vec::with_capacity(raw_len);
    for _ in 0..n_recs {
        match r.bytes(1)?[0] {
            TAG_RAW => out.extend_from_slice(r.len_prefixed()?),
            TAG_DELTA => {
                let block = blocks.get_mut(r.varint()? as usize).ok_or("bad block")?;
                let len = r.varint()? as usize;
                let uuid: [u8; 16] = r.bytes(16)?.try_into().map_err(|_| "bad uuid")?;
                let (session, parent, index, kind, text, cursor) = block;
                let stop = cursor.checked_add(len).ok_or("bad length")?;
                let piece = text.get(*cursor..stop).ok_or("block text too short")?;
                *cursor += len;
                render(&mut out, *index, *kind, piece, session, parent, &uuid);
            }
            _ => return Err("bad record".to_string()),
        }
        if out.len() > raw_len {
            return Err("payload rebuilds past the frame's length".to_string());
        }
    }
    if !r.s.is_empty() {
        return Err("bytes after the last record".to_string());
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests_deltas.rs"]
mod tests;
