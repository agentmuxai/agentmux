// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The frame test vector the offline recovery tool decodes with its own
//! decoder (`scripts/recover-shared-transcripts/testdata/frames.json`, read by
//! `frames.test.mjs`). This build must decode it too, so the two decoders
//! can't drift apart. To write it again after a format change, run this test
//! with `AGENTMUX_WRITE_FRAME_VECTOR=1`.

use super::*;
use base64::Engine as _;
use std::path::PathBuf;

fn vector_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/recover-shared-transcripts/testdata/frames.json")
}

/// One part of transcript with the awkward cases: a first line that began in
/// the part before, delta lines of both kinds with escapes and non-ASCII
/// text, a tool's `parent_tool_use_id`, other lines between them, and a last
/// line cut off by the part's end.
pub(super) fn vector_bytes() -> Vec<u8> {
    let session = "5f0c6a1e-0000-4000-8000-00000000000a";
    let mut s = String::from("e frame before\"}\n");
    let mut n = 0u32;
    while s.len() < PDS {
        if n % 9 == 8 {
            s += &format!("{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"done {n} \u{2713}\"}}]}}}}\n");
        }
        let (ty, field, text) = if n % 3 == 0 {
            ("input_json_delta", "partial_json", format!(r#"{{\"path\": \"C:\\\\src\\\\{n}.rs\", "#))
        } else {
            ("text_delta", "text", format!(r#"word {n} naïve {} \"q\"\n"#, "caf\u{e9} \u{2014}"))
        };
        let parent = if n % 5 == 0 { "\"toolu_01ABCdef\"" } else { "null" };
        let uuid = format!("{:08x}-1111-4222-8333-{:012x}", n, u64::from(n) * 7);
        s += &format!(
            "{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\"index\":{},\"delta\":{{\"type\":\"{ty}\",\"{field}\":\"{text}\"}}}},\"session_id\":\"{session}\",\"parent_tool_use_id\":{parent},\"uuid\":\"{uuid}\"}}\n",
            n % 4
        );
        n += 1;
    }
    let mut bytes = s.into_bytes();
    bytes.truncate(PDS);
    bytes
}

fn frame_of(v: &serde_json::Value) -> Frame {
    Frame {
        first_part: v["first_part"].as_i64().unwrap() as i32,
        part_count: v["part_count"].as_i64().unwrap() as i32,
        codec: v["codec"].as_i64().unwrap(),
        raw_len: v["raw_len"].as_i64().unwrap(),
        check_hash: hex::decode(v["check_hash"].as_str().unwrap()).unwrap(),
        data: base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap(),
    }
}

#[test]
fn the_recovery_tools_test_vector_decodes_here_too() {
    let raw = vector_bytes();
    if std::env::var_os("AGENTMUX_WRITE_FRAME_VECTOR").is_some() {
        let frames: Vec<serde_json::Value> = [CODEC_ZSTD, CODEC_DELTAS]
            .iter()
            .map(|&want| {
                let (codec, data) = encode(want, &raw).unwrap();
                assert_eq!(codec, want, "the vector has delta lines for codec 2 to take");
                serde_json::json!({
                    "first_part": 0,
                    "part_count": 1,
                    "codec": codec,
                    "raw_len": raw.len(),
                    "check_hash": hex::encode(sha256(&raw)),
                    "data": base64::engine::general_purpose::STANDARD.encode(&data),
                })
            })
            .collect();
        let vector = serde_json::json!({
            "about": "Frames written by crates/srv/src/backend/storage/filestore/frames.rs (tests_frame_vector.rs); decoded by lib/frames.mjs in frames.test.mjs.",
            "frames": frames,
        });
        std::fs::write(vector_path(), serde_json::to_string_pretty(&vector).unwrap() + "\n").unwrap();
    }
    let text = std::fs::read_to_string(vector_path()).expect("the test vector");
    let vector: serde_json::Value = serde_json::from_str(&text).unwrap();
    let frames = vector["frames"].as_array().unwrap();
    let codecs: Vec<i64> = frames.iter().map(|f| f["codec"].as_i64().unwrap()).collect();
    assert_eq!(codecs, vec![CODEC_ZSTD, CODEC_DELTAS]);
    for f in frames {
        let frame = frame_of(f);
        assert_eq!(decode_uncached(&frame).unwrap(), raw, "codec {}", frame.codec);
    }
}
