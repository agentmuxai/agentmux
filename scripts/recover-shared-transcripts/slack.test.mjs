// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The tool and a line-oriented file's slack: srv keeps the tail row of a transcript (and its stamps and
// jsonl logs) longer than the content, filled with spaces, so a line is appended in place
// (`tail_capacity` in crates/srv/src/backend/storage/filestore/parts.rs). The tool must not call that
// damage, and must still call real damage: extra bytes in a part that is not the last, NUL bytes or text
// in the slack, a whole part past the size.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { createFilestore, PART } from "./fixture.mjs";
import { allDamage, findDamage } from "./lib/analyze.mjs";
import { slackOnly } from "./lib/frames.mjs";

const dir = mkdtempSync(join(tmpdir(), "recover-slack-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

const lines = (n, from = 0) => Buffer.from(Array.from({ length: n }, (_, i) => `{"n":${from + i}}\n`).join(""));

/** A store with one `output` of `size` bytes whose parts are `rows` (the stored bytes of each part, in order). */
function store(label, size, rows, name = "output") {
    const db = createFilestore(join(dir, `${label}.db`));
    db.prepare("INSERT INTO db_wave_file (zoneid, name, size, createdts, modts) VALUES ('z', ?, ?, 1, 1)").run(name, size);
    const ins = db.prepare("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES ('z', ?, ?, ?)");
    rows.forEach((r, i) => ins.run(name, i, r));
    return db;
}
const pad = (b, to) => Buffer.concat([b, Buffer.alloc(to - b.length, 0x20)]);
const kinds = (db) => findDamage(db).damage.map((d) => d.kind);

describe("slack after the content of the last part", () => {
    const part0 = lines(PART / 4).subarray(0, PART); // a whole part (cut anywhere: the tool does not parse lines here)
    const tail = lines(40, 9000);

    it("is not damage: a short file, a file of several parts, a whole last part", () => {
        const small = store("small", tail.length, [pad(tail, 4096)]);
        expect(kinds(small)).toEqual([]);
        expect(slackOnly(small, "z", "output", tail.length)).toBe(true);
        const several = store("several", PART + tail.length, [part0, pad(tail, 8192)]);
        expect(kinds(several)).toEqual([]);
        expect(allDamage(several).damage).toEqual([]);
        const whole = store("whole", PART, [part0]);
        expect(kinds(whole), "no slack, exact size").toEqual([]);
        expect(slackOnly(whole, "z", "output", PART)).toBe(false);
    });

    it("is not damage for the stamp file either", () => {
        const stamps = Buffer.from('{"off":0,"ms":1}\n{"off":120,"ms":2}\n');
        expect(kinds(store("tsidx", stamps.length, [pad(stamps, 4096)], "output.tsidx"))).toEqual([]);
    });

    it("is damage: NUL bytes inside the slack", () => {
        const row = pad(tail, 4096);
        row[tail.length + 100] = 0;
        const db = store("nul-slack", tail.length, [row]);
        expect(kinds(db)).toEqual(["size-mismatch"]);
        expect(allDamage(db).damage.map((d) => d.zone)).toEqual(["z"]);
    });

    it("is damage: text inside the slack", () => {
        const row = pad(tail, 4096);
        row.write("x", tail.length + 5);
        expect(kinds(store("text-slack", tail.length, [row]))).toEqual(["size-mismatch"]);
    });

    it("is damage: extra bytes in a part that is not the last", () => {
        const long0 = Buffer.concat([part0, Buffer.from("extra\n")]);
        expect(kinds(store("long-first", PART + tail.length, [long0, tail]))).toEqual(["size-mismatch"]);
        // A short part before the last is a gap in the content, not slack.
        expect(kinds(store("short-first", PART + tail.length, [part0.subarray(0, PART - 10), tail]))).not.toEqual([]);
    });

    it("is damage: a whole extra part past the size, or a last part with no content in it", () => {
        expect(kinds(store("extra-part", PART, [part0, pad(tail, 4096)]))).toEqual(["size-mismatch"]);
    });

    it("is damage: a short last part is still tail-missing", () => {
        expect(kinds(store("short", tail.length + 10, [tail]))).toEqual(["tail-missing"]);
    });
});
