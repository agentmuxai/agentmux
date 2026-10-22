// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Compressed frames (docs/specs/SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md
// §4, §8): a row of `db_file_frame` holds up to 64 consecutive whole 64 KB parts of
// one file, and those parts have no row in `db_file_data`. A part's row always
// wins over a frame. This is the tool's own decoder for both codecs, written from
// crates/srv/src/backend/storage/filestore/frames.rs and deltas.rs; the test vector
// in testdata/frames.json, written by the Rust encoder, keeps the two in step.

import { createHash } from "node:crypto";
import { zstdDecompressSync } from "node:zlib";

const PART = 64 * 1024;
const MAX_FRAME_PARTS = 64;
const CODEC_ZSTD = 1;
const CODEC_DELTAS = 2;
// The most a codec 2 payload may exceed its frame by (frames.rs DELTAS_SLACK).
const DELTAS_SLACK = 64 * 1024;

/** Whether the database has the frame table (a store from before frames doesn't). */
export function hasFrames(db) {
    return !!db.prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'db_file_frame'").get();
}

/** Every frame of a file, by first part. */
export function framesOf(db, zone, name) {
    if (!hasFrames(db)) return [];
    return db
        .prepare("SELECT first_part, part_count, codec, raw_len, check_hash, data FROM db_file_frame WHERE zoneid = ? AND name = ? ORDER BY first_part")
        .all(zone, name);
}

// Codec 2's template (deltas.rs P1..P8): a delta line is these pieces around its values.
const KINDS = [
    ["text_delta", "text"],
    ["input_json_delta", "partial_json"],
];
const P1 = '{"type":"stream_event","event":{"type":"content_block_delta","index":';
const P2 = ',"delta":{"type":"';
const P3 = '","';
const P4 = '":"';
const P5 = '"}},"session_id":"';
const P6 = '","parent_tool_use_id":';
const P7 = ',"uuid":"';
const P8 = '"}\n';

function uuidText(u) {
    const h = Buffer.from(u).toString("hex");
    return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

/** Rebuild a codec 2 frame's bytes from its (decompressed) payload; throws on any damage. */
export function decodeDeltas(payload, rawLen) {
    let at = 0;
    const bytes = (n) => {
        if (at + n > payload.length) throw new Error("payload ends inside a field");
        const out = payload.subarray(at, at + n);
        at += n;
        return out;
    };
    const varint = () => {
        let v = 0;
        for (let shift = 0; shift <= 49; shift += 7) {
            const b = bytes(1)[0];
            v += (b & 0x7f) * 2 ** shift;
            if ((b & 0x80) === 0) return v;
        }
        throw new Error("number too long");
    };
    if (bytes(4).toString("latin1") !== "AMD2") throw new Error("not a delta payload");
    const strings = [];
    for (let n = varint(); n > 0; n--) strings.push(bytes(varint()));
    const blocks = [];
    for (let n = varint(); n > 0; n--) {
        const session = strings[varint()];
        const parent = strings[varint()];
        if (!session || !parent) throw new Error("bad string");
        const index = varint();
        const kind = bytes(1)[0];
        if (kind >= KINDS.length) throw new Error("bad kind");
        blocks.push({ session, parent, index, kind, text: bytes(varint()), cursor: 0 });
    }
    const out = [];
    let length = 0;
    const push = (b) => {
        out.push(b);
        length += b.length;
    };
    for (let n = varint(); n > 0; n--) {
        const tag = bytes(1)[0];
        if (tag === 0) push(bytes(varint()));
        else if (tag === 1) {
            const block = blocks[varint()];
            if (!block) throw new Error("bad block");
            const len = varint();
            const uuid = bytes(16);
            if (block.cursor + len > block.text.length) throw new Error("block text too short");
            const piece = block.text.subarray(block.cursor, block.cursor + len);
            block.cursor += len;
            const [type, field] = KINDS[block.kind];
            push(Buffer.from(`${P1}${block.index}${P2}${type}${P3}${field}${P4}`, "latin1"));
            push(piece);
            push(Buffer.from(P5, "latin1"));
            push(block.session);
            push(Buffer.from(P6, "latin1"));
            push(block.parent);
            push(Buffer.from(`${P7}${uuidText(uuid)}${P8}`, "latin1"));
        } else throw new Error("bad record");
        if (length > rawLen) throw new Error("payload rebuilds past the frame's length");
    }
    if (at !== payload.length) throw new Error("bytes after the last record");
    return Buffer.concat(out);
}

/** A frame's logical bytes, checked against its length and SHA-256; throws on any damage. */
export function decodeFrame(f) {
    const rawLen = Number(f.raw_len);
    const count = Number(f.part_count);
    const where = `frame at part ${f.first_part}`;
    if (count < 1 || count > MAX_FRAME_PARTS || rawLen !== count * PART) throw new Error(`${where} has a bad shape`);
    const data = Buffer.from(f.data);
    let raw;
    const codec = Number(f.codec);
    if (codec === CODEC_ZSTD) raw = zstdDecompressSync(data, { maxOutputLength: rawLen });
    else if (codec === CODEC_DELTAS) raw = decodeDeltas(zstdDecompressSync(data, { maxOutputLength: rawLen + DELTAS_SLACK }), rawLen);
    else throw new Error(`${where}: unknown codec ${codec}`);
    if (raw.length !== rawLen) throw new Error(`${where}: ${raw.length} bytes, expected ${rawLen}`);
    if (!createHash("sha256").update(raw).digest().equals(Buffer.from(f.check_hash))) throw new Error(`${where}: bytes don't match its hash`);
    return raw;
}

/**
 * A file's parts, rows and frames together: [{ partidx, len, framed }] in order. A part with a
 * row is the row, as the app reads it. Parts of a frame are listed without decoding it.
 */
export function partIndex(db, zone, name) {
    const parts = new Map();
    for (const r of db.prepare("SELECT partidx, LENGTH(data) AS len FROM db_file_data WHERE zoneid = ? AND name = ?").all(zone, name)) {
        parts.set(Number(r.partidx), { partidx: Number(r.partidx), len: Number(r.len), framed: false });
    }
    for (const f of framesOf(db, zone, name)) {
        for (let i = 0; i < Number(f.part_count); i++) {
            const idx = Number(f.first_part) + i;
            if (!parts.has(idx)) parts.set(idx, { partidx: idx, len: PART, framed: true });
        }
    }
    return [...parts.values()].sort((a, b) => a.partidx - b.partidx);
}

/**
 * A file's bytes: every part it has, in order, a row where there is one and otherwise its
 * frame, decoded. A frame that doesn't decode throws.
 */
export function readFileBytes(db, zone, name) {
    const parts = new Map();
    for (const r of db.prepare("SELECT partidx, data FROM db_file_data WHERE zoneid = ? AND name = ?").all(zone, name)) {
        parts.set(Number(r.partidx), Buffer.from(r.data));
    }
    for (const f of framesOf(db, zone, name)) {
        const first = Number(f.first_part);
        const count = Number(f.part_count);
        let raw = null;
        for (let i = 0; i < count; i++) {
            if (parts.has(first + i)) continue;
            raw ??= decodeFrame(f);
            parts.set(first + i, raw.subarray(i * PART, (i + 1) * PART));
        }
    }
    return Buffer.concat([...parts.entries()].sort((a, b) => a[0] - b[0]).map((e) => e[1]));
}

/**
 * Every frame of every `output` file (or of every file when `name` is null), decoded:
 * { bad: [{ zone, name, first, error }], nul: [{ zone, name, first, parts }] }. A frame that
 * doesn't decode is damage the app reads as blank; one holding a NUL byte is a hole.
 */
export function scanFrames(db, { name = "output" } = {}) {
    const bad = [];
    const nul = new Map();
    if (!hasFrames(db)) return { bad, nul: [] };
    const where = name === null ? "" : "WHERE name = ?";
    const rows = db.prepare(`SELECT zoneid, name, first_part, part_count, codec, raw_len, check_hash, data FROM db_file_frame ${where} ORDER BY zoneid, name, first_part`);
    for (const f of name === null ? rows.iterate() : rows.iterate(name)) {
        let raw;
        try {
            raw = decodeFrame(f);
        } catch (e) {
            bad.push({ zone: f.zoneid, name: f.name, first: Number(f.first_part), error: String(e.message ?? e) });
            continue;
        }
        for (let i = 0; i < Number(f.part_count); i++) {
            if (!raw.subarray(i * PART, (i + 1) * PART).includes(0)) continue;
            const key = `${f.zoneid}\u0000${f.name}`;
            const had = nul.get(key);
            if (had) had.parts++;
            else nul.set(key, { zone: f.zoneid, name: f.name, first: Number(f.first_part) + i, parts: 1 });
        }
    }
    return { bad, nul: [...nul.values()] };
}

/** Delete a file's frames, as the app does when it deletes or replaces the file. */
export function deleteFrames(db, zone, name) {
    if (hasFrames(db)) db.prepare("DELETE FROM db_file_frame WHERE zoneid = ? AND name = ?").run(zone, name);
}
