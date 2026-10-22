// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The tool's frame decoder (lib/frames.mjs) on the test vector the Rust encoder
// wrote (testdata/frames.json, checked on the Rust side by tests_frame_vector.rs),
// and the tool's damage scan, rebuild and source reads on a store holding frames.
// Needs no sqlite3 shell: everything here runs in node:sqlite.

import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { zstdCompressSync, zstdDecompressSync } from "node:zlib";
import { afterAll, describe, expect, it } from "vitest";
import { createChannel, createFilestore, FRAME_TABLE, PART, putFile, transcript } from "./fixture.mjs";
import { allDamage, findDamage } from "./lib/analyze.mjs";
import { decodeDeltas, decodeFrame, partIndex, readFileBytes } from "./lib/frames.mjs";
import { rebuildZone } from "./lib/rebuild.mjs";
import { findSources, readSource } from "./lib/sources.mjs";
import { DatabaseSync } from "node:sqlite";

const vector = JSON.parse(readFileSync(new URL("./testdata/frames.json", import.meta.url), "utf8"));
const asRow = (f) => ({ ...f, check_hash: Buffer.from(f.check_hash, "hex"), data: Buffer.from(f.data, "base64") });
const [codec1, codec2] = vector.frames.map(asRow);

describe("the frame decoder", () => {
    it("decodes both codecs of the Rust test vector to the same bytes", () => {
        expect([codec1.codec, codec2.codec]).toEqual([1, 2]);
        const a = decodeFrame(codec1);
        const b = decodeFrame(codec2);
        expect(a.length).toBe(PART);
        expect(b.equals(a)).toBe(true);
        expect(createHash("sha256").update(b).digest("hex")).toBe(vector.frames[1].check_hash);
        const text = b.toString("utf8");
        expect(text).toContain('"type":"input_json_delta","partial_json":"{\\"path\\": \\"C:\\\\\\\\src\\\\\\\\0.rs\\", "');
        expect(text).toContain('"text":"word 1 naïve café — \\"q\\"\\n"');
        expect(text).toContain('"parent_tool_use_id":"toolu_01ABCdef"');
        expect(codec2.data.length).toBeLessThan(codec1.data.length);
    });

    it("refuses a frame that is damaged, the wrong shape or of an unknown codec", () => {
        const flipped = { ...codec1, data: Buffer.from(codec1.data) };
        flipped.data[flipped.data.length >> 1] ^= 0x40;
        expect(() => decodeFrame(flipped)).toThrow();
        const wrongHash = { ...codec2, check_hash: Buffer.alloc(32) };
        expect(() => decodeFrame(wrongHash)).toThrow(/hash/);
        expect(() => decodeFrame({ ...codec1, raw_len: PART - 1 })).toThrow(/shape/);
        expect(() => decodeFrame({ ...codec1, part_count: 65, raw_len: 65 * PART })).toThrow(/shape/);
        expect(() => decodeFrame({ ...codec1, codec: 3 })).toThrow(/codec 3/);
    });

    it("refuses a codec 2 payload cut short anywhere", () => {
        const payload = zstdDecompressSync(codec2.data);
        expect(decodeDeltas(payload, PART).equals(decodeFrame(codec1))).toBe(true);
        const cuts = [...Array(80).keys(), ...Array.from({ length: 40 }, (_, i) => Math.floor((payload.length * (i + 1)) / 41)), payload.length - 1];
        for (const cut of cuts) expect(() => decodeDeltas(payload.subarray(0, cut), PART), `cut at ${cut}`).toThrow();
    });
});

describe("a store holding frames", () => {
    const dir = mkdtempSync(join(tmpdir(), "recover-frames-"));
    afterAll(() => rmSync(dir, { recursive: true, force: true }));
    const raw = decodeFrame(codec2);
    const tail = transcript("vvv", PART + 50);

    // `agent:vvv:current` output: part 0 a codec 2 frame (the vector), then rows.
    function store(name, frame = codec2) {
        const db = createFilestore(join(dir, name, "filestore.db"));
        db.exec(FRAME_TABLE);
        putFile(db, "agent:vvv:current", "output", Buffer.concat([raw, tail]), { dropParts: [0] });
        db.prepare("INSERT INTO db_file_frame (zoneid, name, first_part, part_count, codec, raw_len, check_hash, data) VALUES (?, ?, 0, 1, ?, ?, ?, ?)").run(
            "agent:vvv:current",
            "output",
            frame.codec,
            frame.raw_len,
            frame.check_hash,
            frame.data,
        );
        return db;
    }

    it("reads framed parts as parts: no damage, and the bytes the app reads", () => {
        const db = store("whole");
        expect(findDamage(db).damage).toEqual([]);
        expect(allDamage(db).damage).toEqual([]);
        expect(partIndex(db, "agent:vvv:current", "output").map((p) => [p.partidx, p.framed])).toEqual([
            [0, true],
            [1, false],
            [2, false],
        ]);
        expect(readFileBytes(db, "agent:vvv:current", "output").equals(Buffer.concat([raw, tail]))).toBe(true);
        db.close();
    });

    it("finds a frame that doesn't decode", () => {
        const bad = { ...codec2, data: Buffer.from(codec2.data) };
        bad.data[10] ^= 0xff;
        const db = store("bad-frame", bad);
        expect(findDamage(db).damage).toEqual([]);
        const found = allDamage(db).damage;
        expect(found.map((d) => [d.zone, d.kind])).toEqual([["agent:vvv:current", "bad-frame"]]);
        expect(found[0].partsBytes).toBe(raw.length + tail.length);
        expect(allDamage(db, { contentScan: false }).damage).toEqual([]);
        db.close();
    });

    it("finds a NUL hole inside a frame", () => {
        const holey = Buffer.from(raw);
        holey.fill(0, 100, 200);
        const db = store("nul", { ...codec1, check_hash: createHash("sha256").update(holey).digest(), data: zstdCompressSync(holey) });
        expect(allDamage(db).damage.map((d) => [d.kind, d.detail])).toEqual([["nul-bytes", "1 part(s) hold NUL bytes, from part 0"]]);
        db.close();
    });

    it("finds a gap when a frame is missing", () => {
        const db = store("gap");
        db.exec("DELETE FROM db_file_frame");
        expect(findDamage(db).damage.map((d) => d.kind)).toEqual(["gap"]);
        db.close();
    });

    it("a frame with a part count srv never writes is damage, found without decoding it", () => {
        const db = store("wild-count");
        db.exec("UPDATE db_file_frame SET part_count = 2000000000, raw_len = 2000000000 * 65536");
        const found = findDamage(db).damage;
        expect(found.map((d) => d.kind)).toEqual(["gap"]);
        expect(found[0].detail).toMatch(/the frame from part 0 has a shape srv never writes \(2000000000 parts/);
        expect(partIndex(db, "agent:vvv:current", "output").map((p) => p.partidx)).toEqual([1, 2]);
        db.exec("UPDATE db_file_frame SET part_count = -5");
        expect(findDamage(db).damage[0].detail).toMatch(/-5 parts/);
        db.close();
    });

    it("a frame's damage or NULs count only for parts that have no row", () => {
        const bad = { ...codec2, data: Buffer.from(codec2.data) };
        bad.data[10] ^= 0xff;
        const db = store("shadowed-bad", bad);
        expect(allDamage(db).damage.map((d) => d.kind)).toEqual(["bad-frame"]);
        // Rows written over every part of the damaged frame: the app reads those.
        db.prepare("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?, ?, 0, ?)").run("agent:vvv:current", "output", Buffer.alloc(PART, "r"));
        expect(allDamage(db).damage).toEqual([]);
        db.close();

        const holey = Buffer.from(raw);
        holey.fill(0, 100, 200);
        const db2 = store("shadowed-nul", { ...codec1, check_hash: createHash("sha256").update(holey).digest(), data: zstdCompressSync(holey) });
        expect(allDamage(db2).damage.map((d) => d.kind)).toEqual(["nul-bytes"]);
        db2.prepare("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?, ?, 0, ?)").run("agent:vvv:current", "output", Buffer.alloc(PART, "r"));
        expect(allDamage(db2).damage).toEqual([]);
        db2.close();
    });

    it("a part with a row and a frame is the row, and the file is whole", () => {
        const db = store("row-and-frame");
        // A build that knows nothing of frames wrote part 0 again: the row wins.
        const newer = Buffer.alloc(PART, "n");
        db.prepare("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?, ?, 0, ?)").run("agent:vvv:current", "output", newer);
        expect(findDamage(db).damage).toEqual([]);
        expect(allDamage(db).damage).toEqual([]);
        expect(partIndex(db, "agent:vvv:current", "output").map((p) => p.framed)).toEqual([false, false, false]);
        expect(readFileBytes(db, "agent:vvv:current", "output").equals(Buffer.concat([newer, tail]))).toBe(true);
        db.close();
    });

    it("a rebuild takes the file's frames with its rows", () => {
        const db = store("rebuild");
        const fresh = transcript("new", 2 * PART + 3);
        rebuildZone(db, "agent:vvv:current", fresh, 5, { opts: "{}", meta: "{}" });
        expect(db.prepare("SELECT COUNT(*) AS n FROM db_file_frame").get().n).toBe(0);
        expect(readFileBytes(db, "agent:vvv:current", "output").equals(fresh)).toBe(true);
        expect(findDamage(db).damage).toEqual([]);
        db.close();
    });

    it("a per-channel copy holding frames is checked frame by frame and read decoded", () => {
        const home = join(dir, "home");
        const frameIn = (data, oid, frame) => {
            const db = new DatabaseSync(join(data, "db", "filestore.db"));
            db.exec(FRAME_TABLE);
            db.prepare("INSERT INTO db_file_frame (zoneid, name, first_part, part_count, codec, raw_len, check_hash, data) VALUES (?, 'output', 0, 1, ?, ?, ?, ?)").run(
                oid, frame.codec, frame.raw_len, frame.check_hash, frame.data);
            db.close();
        };
        const output = Buffer.concat([raw, tail]);
        const good = createChannel(home, "stable", "1.0.0", [{ oid: "blk-good", meta: { agentId: "ok" }, output, dropParts: [0] }]);
        frameIn(good, "blk-good", codec2);
        const bad = { ...codec2, data: Buffer.from(codec2.data) };
        bad.data[10] ^= 0xff;
        const broken = createChannel(home, "stable", "2.0.0", [{ oid: "blk-bad", meta: { agentId: "ok" }, output: Buffer.concat([output, Buffer.from("more\n")]), dropParts: [0] }]);
        frameIn(broken, "blk-bad", bad);
        const { byAgent } = findSources(home, new Set(["ok"]));
        const found = byAgent.get("ok");
        expect(found.candidates.map((c) => [c.blockOid, c.whole]).sort()).toEqual([["blk-bad", false], ["blk-good", true]]);
        expect(found.best.blockOid).toBe("blk-good");
        expect(readSource(found.best).equals(output)).toBe(true);
    });

    it("the fixture's frame table is the one srv creates", () => {
        const rust = readFileSync(new URL("../../crates/srv/src/backend/storage/filestore/frames.rs", import.meta.url), "utf8").replace(/\s+/g, " ");
        expect(rust).toContain(FRAME_TABLE.replace(/\s+/g, " "));
    });
});
