// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Codec 2 (`deltas.rs`): every byte comes back, only exact delta lines are
//! taken, and a damaged payload is an error, never a panic.

use super::*;
use proptest::prelude::*;

const SESSION: &str = "8d422a5b-eb35-49d9-b357-78a3105606f7";

fn uuid(n: u64) -> String {
    let h = format!("{:032x}", (n as u128).wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835));
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// A delta line as the CLI writes it. `text` is already JSON-escaped.
fn delta(kind: &str, index: u32, text: &str, parent: Option<&str>, n: u64) -> String {
    let (ty, field) = if kind == "text" { ("text_delta", "text") } else { ("input_json_delta", "partial_json") };
    let parent = parent.map_or("null".to_string(), |p| format!("\"{p}\""));
    format!(
        "{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\"index\":{index},\"delta\":{{\"type\":\"{ty}\",\"{field}\":\"{text}\"}}}},\"session_id\":\"{SESSION}\",\"parent_tool_use_id\":{parent},\"uuid\":\"{}\"}}\n",
        uuid(n)
    )
}

fn other(kind: &str, n: u64) -> String {
    format!("{{\"type\":\"stream_event\",\"event\":{{\"type\":\"{kind}\"}},\"session_id\":\"{SESSION}\",\"parent_tool_use_id\":null,\"uuid\":\"{}\"}}\n", uuid(n))
}

/// One reply with a text block and a tool block, a subagent streaming at the
/// same time, and the lines around a reply that must stay as they are.
fn reply() -> Vec<u8> {
    let mut s = String::new();
    s += "ail of a line that began in the frame before\"}\n";
    s += &other("message_start", 1);
    s += &delta("text", 0, "Hel", None, 2);
    s += &delta("text", 0, "lo \\\"q\\\" \\\\ back\\nslash é ✓ \\u001b", None, 3);
    s += &delta("text", 0, "", None, 4);
    s += &delta("text", 3, "sub", Some("toolu_01AaKPesd71WCueeP6gp7jgo"), 5);
    s += &other("ping", 6);
    s += &delta("text", 0, " world", None, 7);
    s += &delta("text", 3, "agent text", Some("toolu_01AaKPesd71WCueeP6gp7jgo"), 8);
    s += &delta("json", 12, "{\\\"file_path\\\": \\\"C:\\\\\\\\Users", None, 9);
    s += &delta("json", 12, "\\\\\\\\x\\\", \\\"n\\\": 3}", None, 10);
    s += "{\"type\":\"assistant\",\"message\":{\"id\":\"msg_1\",\"content\":[{\"type\":\"text\",\"text\":\"Hello world\"}]}}\n";
    s += &other("message_stop", 11);
    s += "{\"type\":\"user\",\"message\":{\"content\":\"thanks\"}}\n";
    s += "{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"the end of the frame cuts th";
    s.into_bytes()
}

#[test]
fn every_byte_comes_back_and_the_deltas_are_left_out() {
    let raw = reply();
    let (payload, stats) = encode(&raw).expect("has delta lines");
    assert_eq!(stats.lines, 8, "every exact delta line, main and subagent, text and tool input");
    assert_eq!(decode(&payload, raw.len()).unwrap(), raw);
    assert!(payload.len() < raw.len() / 2, "{} of {}", payload.len(), raw.len());
}

#[test]
fn a_line_that_is_not_exactly_the_template_stays_as_it_was() {
    let base = delta("text", 0, "abc", None, 1);
    let variants = [
        // An extra field, as thinking deltas carry.
        base.replace("\"}\n", "\",\"thinking_display\":\"updates\"}\n"),
        // An uppercase uuid: it would come back lowercase.
        base.replace(&uuid(1), &uuid(1).to_uppercase()),
        // A space the CLI never writes.
        base.replace("\"index\":0", "\"index\": 0"),
        // Fields in another order.
        base.replace(&format!("\"session_id\":\"{SESSION}\",\"parent_tool_use_id\":null"), &format!("\"parent_tool_use_id\":null,\"session_id\":\"{SESSION}\"")),
        // A thinking delta: another kind.
        base.replace("\"text_delta\",\"text\"", "\"thinking_delta\",\"thinking\""),
        // A leading zero in the index.
        base.replace("\"index\":0", "\"index\":00"),
        // A 36-byte uuid with a fifth dash (an odd number of digits), or with
        // its dashes elsewhere.
        base.replace(&uuid(1), &format!("{}-", &uuid(1)[..35])),
        base.replace(&uuid(1), &format!("{}-{}", &uuid(1)[..7], &uuid(1)[8..])),
        // No newline at the end.
        base.trim_end().to_string(),
    ];
    for v in variants {
        // The exact line first: the variant without a newline must end the frame.
        let raw = format!("{}{v}", delta("text", 0, "kept", None, 2));
        let (payload, stats) = encode(raw.as_bytes()).unwrap();
        assert_eq!(stats.lines, 1, "only the exact line is taken: {v}");
        assert_eq!(decode(&payload, raw.len()).unwrap(), raw.as_bytes());
    }
}

#[test]
fn a_uuid_is_taken_only_in_its_canonical_form() {
    let u = uuid(7);
    let bytes = parse_uuid(u.as_bytes()).unwrap();
    let mut back = Vec::new();
    push_uuid(&mut back, &bytes);
    assert_eq!(back, u.as_bytes());
    for bad in [
        format!("{}-", &u[..35]),
        format!("{}-{}", &u[..7], &u[8..]),
        format!("{}0", &u[..35]).replacen('-', "0", 1),
        u.to_uppercase(),
        u.replace('-', ""),
        format!("{}x", &u[..35]),
    ] {
        assert_eq!(parse_uuid(bad.as_bytes()), None, "{bad}");
    }
}

/// A delta line as the current CLI writes it: the message id comes after the uuid.
fn delta_api(kind: &str, index: u32, text: &str, parent: Option<&str>, n: u64, api: &str) -> String {
    let base = delta(kind, index, text, parent, n);
    format!("{}\",\"api_message_id\":\"{api}\"}}\n", base.strip_suffix("\"}\n").unwrap())
}

/// Two replies, each with a text block 0 and a tool block 1, then the same block numbers again for the
/// next message, as the CLI numbers them.
fn api_replies() -> Vec<u8> {
    let mut s = String::new();
    let mut n = 0u64;
    for api in ["msg_01AAAAAAAAAAAAAAAAAAAAAA", "msg_01BBBBBBBBBBBBBBBBBBBBBB"] {
        s += &other("message_start", n);
        for i in 0..6 {
            n += 1;
            s += &delta_api("text", 0, &format!("word {i} "), None, n, api);
        }
        for i in 0..6 {
            n += 1;
            s += &delta_api("json", 1, &format!("{{\\\"n\\\": {i}}}"), None, n, api);
        }
        s += &other("message_stop", n);
        n += 1;
        s += "{\"type\":\"assistant\"}\n";
    }
    s.into_bytes()
}

#[test]
fn a_line_with_an_api_message_id_is_taken_and_comes_back_exactly() {
    let raw = api_replies();
    let (payload, stats) = encode(&raw).expect("has delta lines");
    assert_eq!(stats.lines, 24, "every line, each with its message id");
    assert!(stats.api);
    assert_eq!(&payload[..4], b"AMD3", "a frame with a message id is a codec 3 payload");
    assert_eq!(decode(&payload, raw.len()).unwrap(), raw);
    assert!(payload.len() < raw.len() / 2, "{} of {}", payload.len(), raw.len());
}

#[test]
fn the_same_block_number_in_two_messages_is_two_blocks() {
    // Block 0's text of message A is not followed by block 0's text of message B: each message's block is one run.
    let raw = api_replies();
    let (payload, _) = encode(&raw).unwrap();
    let rebuilt = decode(&payload, raw.len()).unwrap();
    assert_eq!(rebuilt, raw);
    let text = String::from_utf8(rebuilt).unwrap();
    assert!(text.find("msg_01AAAAAAAAAAAAAAAAAAAAAA").unwrap() < text.find("msg_01BBBBBBBBBBBBBBBBBBBBBB").unwrap());
}

#[test]
fn a_frame_with_no_message_id_stays_a_codec_two_payload() {
    let (payload, stats) = encode(&reply()).unwrap();
    assert!(!stats.api);
    assert_eq!(&payload[..4], b"AMD2", "so a build that reads codecs 1 and 2 still reads it");
}

#[test]
fn lines_with_and_without_a_message_id_mix() {
    let mut s = String::new();
    s += &delta("text", 0, "plain ", None, 1);
    s += &delta_api("text", 0, "with id ", None, 2, "msg_01X");
    s += &delta_api("text", 0, "again ", None, 3, "msg_01X");
    s += &delta("text", 0, "plain again", None, 4);
    let raw = s.into_bytes();
    let (payload, stats) = encode(&raw).unwrap();
    assert_eq!((stats.lines, stats.api), (4, true));
    assert_eq!(decode(&payload, raw.len()).unwrap(), raw);
}

#[test]
fn a_message_id_that_is_not_plain_leaves_the_line_as_it_was() {
    for tail in [
        "\",\"api_message_id\":\"msg_\\u0031\"}\n",    // an escape in the id
        "\",\"api_message_id\":\"msg_1\",\"x\":1}\n",   // something after it
        "\",\"api_message_id\":null}\n",                 // not a string
        "\",\"api_message_id\":\"msg_1\"}",              // no end of line
        "\",\"api_message_id\":\"msg_1\n\"}\n",          // a line break inside
    ] {
        let base = delta("text", 0, "hi", None, 1);
        let line = format!("{}{tail}", base.strip_suffix("\"}\n").unwrap());
        assert!(parse_delta(line.as_bytes()).is_none(), "{tail:?}");
    }
}

#[test]
fn a_frame_stored_with_codec_three_reads_like_any_other() {
    use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};
    let fs = FileStore::open_in_memory().unwrap();
    let (z, n) = ("agent:a:current", "output");
    let mut data = Vec::new();
    let mut k = 0u64;
    while data.len() < 20 * 65536 {
        data.extend_from_slice(other("message_start", k).as_bytes());
        let api = format!("msg_01{k:022}");
        for i in 0..40 {
            data.extend_from_slice(delta_api("text", 0, &format!("word {i} of reply {k}, "), None, k * 100 + i, &api).as_bytes());
        }
        data.extend_from_slice(format!("{{\"type\":\"assistant\",\"n\":{k}}}\n").as_bytes());
        k += 1;
    }
    fs.make_file(z, n, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(z, n, &data).unwrap();
    let plain = super::super::frames::encode(super::super::frames::CODEC_ZSTD, &data[..16 * 65536]).unwrap().1.len();
    let out = fs.frame_parts(z, n, 0, 16, super::super::frames::CODEC_DELTA_API).unwrap();
    let super::super::frames::FrameOutcome::Framed { stored, codec, .. } = out else { panic!("{out:?}") };
    assert_eq!(codec, super::super::frames::CODEC_DELTA_API);
    assert!(stored < plain, "codec 3 {stored} < codec 1 {plain}");
    assert_eq!(fs.read_file(z, n).unwrap().unwrap(), data);
    let size = data.len() as i64;
    assert_eq!(fs.read_bytes_db(z, n, 0, size).unwrap(), data);
}

#[test]
fn asking_for_codec_two_leaves_a_line_with_a_message_id_alone() {
    use super::super::frames::{encode as frame_encode, CODEC_DELTAS, CODEC_DELTA_API, CODEC_ZSTD};
    // Only lines with a message id: codec 2 has nothing to take, so the frame is plain zstd, which every build reads.
    let raw = api_replies();
    assert_eq!(frame_encode(CODEC_DELTAS, &raw).unwrap().0, CODEC_ZSTD);
    assert_eq!(frame_encode(CODEC_DELTA_API, &raw).unwrap().0, CODEC_DELTA_API);
    // Mixed with plain lines: codec 2 takes those and keeps the rest as they were, in a payload codec 2 readers read.
    let mut mixed = reply();
    mixed.extend_from_slice(&raw);
    let (codec, stored) = frame_encode(CODEC_DELTAS, &mixed).unwrap();
    assert_eq!(codec, CODEC_DELTAS);
    let payload = zstd::bulk::decompress(&stored, mixed.len() + 65536).unwrap();
    assert_eq!(&payload[..4], b"AMD2");
    assert_eq!(decode(&payload, mixed.len()).unwrap(), mixed);
    // A frame with no message id is codec 2 either way.
    assert_eq!(frame_encode(CODEC_DELTA_API, &reply()).unwrap().0, CODEC_DELTAS);
}

#[test]
fn a_frame_without_delta_lines_is_left_to_codec_one() {
    assert!(encode(b"{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n").is_none());
    assert!(encode(b"").is_none());
}

#[test]
fn a_damaged_payload_is_an_error() {
    let raw = reply();
    let (payload, _) = encode(&raw).unwrap();
    assert!(decode(&payload[..payload.len() - 1], raw.len()).is_err(), "truncated");
    assert!(decode(b"XXXX", raw.len()).is_err(), "bad magic");
    assert!(decode(&payload, raw.len() - 1).is_err(), "rebuilds past the frame");
    let mut extra = payload.clone();
    extra.push(0);
    assert!(decode(&extra, raw.len() + 10).is_err(), "bytes after the last record");
}

#[test]
fn a_frame_stored_with_codec_two_reads_like_any_other() {
    use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};
    let fs = FileStore::open_in_memory().unwrap();
    let (z, n) = ("agent:a:current", "output");
    let mut data = Vec::new();
    let mut k = 0u64;
    while data.len() < 20 * 65536 {
        data.extend_from_slice(other("message_start", k).as_bytes());
        for i in 0..40 {
            data.extend_from_slice(delta("text", 0, &format!("word {i} of reply {k}, "), None, k * 100 + i).as_bytes());
        }
        data.extend_from_slice(format!("{{\"type\":\"assistant\",\"n\":{k}}}\n").as_bytes());
        k += 1;
    }
    fs.make_file(z, n, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(z, n, &data).unwrap();
    let plain = super::super::frames::encode(super::super::frames::CODEC_ZSTD, &data[..16 * 65536]).unwrap().1.len();
    let out = fs.frame_parts(z, n, 0, 16, super::super::frames::CODEC_DELTAS).unwrap();
    let super::super::frames::FrameOutcome::Framed { stored, codec, .. } = out else { panic!("{out:?}") };
    assert_eq!(codec, super::super::frames::CODEC_DELTAS);
    assert!(stored < plain, "codec 2 {stored} < codec 1 {plain}");
    assert_eq!(fs.read_file(z, n).unwrap().unwrap(), data);
    let size = data.len() as i64;
    assert_eq!(fs.read_bytes_db(z, n, 0, size).unwrap(), data);
}

fn escaped_text() -> impl Strategy<Value = String> {
    // JSON-escaped strings with the escapes and characters the CLI writes.
    prop::collection::vec(
        prop_oneof![
            "[a-zA-Z0-9 ,.:{}]{0,12}",
            Just("\\n".to_string()),
            Just("\\\"".to_string()),
            Just("\\\\".to_string()),
            Just("\\u001b".to_string()),
            Just("é✓".to_string()),
        ],
        0..6,
    )
    .prop_map(|v| v.concat())
}

#[derive(Debug, Clone)]
enum Line {
    Delta { json: bool, index: u32, text: String, sub: bool, api: Option<u8> },
    Other(String),
}

fn line() -> impl Strategy<Value = Line> {
    prop_oneof![
        3 => (any::<bool>(), 0u32..20, escaped_text(), any::<bool>(), prop::option::of(0u8..3)).prop_map(|(json, index, text, sub, api)| Line::Delta { json, index, text, sub, api }),
        1 => "[a-z{}\":,]{0,40}".prop_map(|s| Line::Other(format!("{s}\n"))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// Any mix of delta lines (both kinds, main and subagent, any index) and
    /// other lines, cut anywhere at the start and end, comes back exactly.
    #[test]
    fn any_mix_comes_back_exactly(lines in prop::collection::vec(line(), 0..60), cut_head in 0usize..50, cut_tail in 0usize..50) {
        let mut raw = String::new();
        for (n, l) in lines.iter().enumerate() {
            match l {
                Line::Delta { json, index, text, sub, api } => {
                    let (kind, parent) = (if *json { "json" } else { "text" }, sub.then_some("toolu_01X"));
                    raw += &match api {
                        Some(a) => delta_api(kind, *index, text, parent, n as u64, &format!("msg_0{a}")),
                        None => delta(kind, *index, text, parent, n as u64),
                    }
                }
                Line::Other(s) => raw += s,
            }
        }
        let bytes = raw.into_bytes();
        let start = cut_head.min(bytes.len());
        let end = bytes.len().saturating_sub(cut_tail).max(start);
        let raw = &bytes[start..end];
        if let Some((payload, _)) = encode(raw) {
            prop_assert_eq!(decode(&payload, raw.len()).unwrap(), raw.to_vec());
        }
    }

    /// Any damage to a payload is an error or a different result, never a panic.
    #[test]
    fn damaged_payloads_never_panic(flips in prop::collection::vec((any::<usize>(), any::<u8>()), 1..8), cut in any::<usize>()) {
        let raw = reply();
        let (mut payload, _) = encode(&raw).unwrap();
        for (at, b) in flips {
            let i = at % payload.len();
            payload[i] ^= b;
        }
        let cut = cut % (payload.len() + 1);
        let _ = decode(&payload[..cut], raw.len());
        let _ = decode(&payload, raw.len());
    }
}

/// The offline replay of spec §9, by hand: every full frame of every agent
/// transcript in a COPY of a real store (`AGENTMUX_FRAMES_REPLAY_DB`, opened
/// read-only), encoded with both codecs, decoded and compared byte for byte,
/// with the sizes added up. `cargo test -- --ignored replay_a_real_store --nocapture`.
#[test]
#[ignore]
fn replay_a_real_store() {
    use super::super::frames::{decode_uncached, encode, Frame, CODEC_DELTAS, CODEC_DELTA_API, CODEC_ZSTD, FRAME_PARTS};
    use sha2::{Digest, Sha256};
    let Ok(path) = std::env::var("AGENTMUX_FRAMES_REPLAY_DB") else { return };
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let files: Vec<(String, i64)> = conn
        .prepare("SELECT zoneid, size FROM db_wave_file WHERE name = 'output' AND zoneid LIKE 'agent:%' ORDER BY size DESC")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let pds = 65536usize;
    let (mut raw_total, mut z1, mut z2, mut z3, mut frames, mut taken_lines, mut c2_frames, mut c3_frames, mut skipped) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for (zone, size) in &files {
        let parts = (*size as usize).div_ceil(pds);
        let mut first = 0usize;
        while first + (FRAME_PARTS as usize) < parts {
            let mut raw = Vec::with_capacity(FRAME_PARTS as usize * pds);
            let mut ok = true;
            for idx in first..first + FRAME_PARTS as usize {
                let row: Option<Vec<u8>> = conn
                    .query_row("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = 'output' AND partidx = ?2", rusqlite::params![zone, idx as i32], |r| r.get(0))
                    .ok();
                match row {
                    Some(d) if d.len() == pds => raw.extend_from_slice(&d),
                    _ => ok = false,
                }
            }
            first += FRAME_PARTS as usize;
            if !ok {
                skipped += 1;
                continue;
            }
            let hash = Sha256::digest(&raw).to_vec();
            let mk = |codec: i64, data: Vec<u8>| Frame { first_part: 0, part_count: FRAME_PARTS, codec, raw_len: raw.len() as i64, check_hash: hash.clone(), data };
            let (c1, d1) = encode(CODEC_ZSTD, &raw).unwrap();
            assert_eq!(decode_uncached(&mk(c1, d1.clone())).unwrap(), raw, "{zone} frame {first}: codec 1");
            let (c2, d2) = encode(CODEC_DELTAS, &raw).unwrap();
            assert_eq!(decode_uncached(&mk(c2, d2.clone())).unwrap(), raw, "{zone} frame {first}: codec {c2}");
            if c2 == CODEC_DELTAS || c2 == CODEC_DELTA_API {
                c2_frames += 1;
                taken_lines += encode_stats(&raw);
            }
            // Codec 3: the same frame, also taking the lines that carry an api_message_id.
            let (c3, d3) = encode(CODEC_DELTA_API, &raw).unwrap();
            assert_eq!(decode_uncached(&mk(c3, d3.clone())).unwrap(), raw, "{zone} frame {first}: codec {c3}");
            c3_frames += usize::from(c3 == CODEC_DELTA_API);
            raw_total += raw.len();
            z1 += d1.len();
            z2 += d2.len();
            z3 += d3.len();
            frames += 1;
        }
    }
    let mb = |n: usize| n as f64 / 1e6;
    println!(
        "REPLAY files={} frames={frames} (codec 2: {c2_frames}, codec 3: {c3_frames}) skipped={skipped} raw={:.1} MB codec1={:.1} MB ({:.2}x) codec2={:.1} MB ({:.2}x, {:.1}% smaller than codec 1) codec3={:.1} MB ({:.2}x, {:.1}% smaller than codec 1) delta lines left out={taken_lines}",
        files.len(),
        mb(raw_total),
        mb(z1),
        raw_total as f64 / z1.max(1) as f64,
        mb(z2),
        raw_total as f64 / z2.max(1) as f64,
        100.0 * (1.0 - z2 as f64 / z1.max(1) as f64),
        mb(z3),
        raw_total as f64 / z3.max(1) as f64,
        100.0 * (1.0 - z3 as f64 / z1.max(1) as f64),
    );
}

fn encode_stats(raw: &[u8]) -> usize {
    encode(raw).map_or(0, |(_, s)| s.lines)
}
