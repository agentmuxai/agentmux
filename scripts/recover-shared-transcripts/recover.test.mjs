// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// recover.mjs on a synthetic store with injected damage: a part missing, pages
// corrupted on disk, and a transcript with no per-channel copy. Needs a sqlite3
// shell 3.51.3 or newer ($SQLITE3 or on PATH); the suite is skipped without one.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createChannel, createFilestore, putFile, PART, TRIGGERS, TABLES, transcript } from "./fixture.mjs";
import { findDamage } from "./lib/analyze.mjs";
import { atLeast, findSqlite3, MIN_SQLITE } from "./lib/sqlite.mjs";
import { main } from "./recover.mjs";

let cli = null;
try {
    cli = findSqlite3();
    if (!atLeast(cli.version, MIN_SQLITE)) cli = null;
} catch {
    cli = null;
}
const suite = cli ? describe : describe.skip;

const text = {
    aaa: transcript("aaa", 5 * PART + 100), // a part missing in the shared store; a whole copy in a channel
    bbb: transcript("bbb", 3 * PART + 7), // healthy
    ccc: transcript("ccc", 4 * PART + 5), // damaged, no per-channel copy
    ddd: transcript("ddd", 4 * PART + 9), // damaged; the only copy is shorter
    fff: transcript("fff", 6 * PART + 33), // pages corrupted on disk; a copy under the legacy meta key
    ggg: transcript("ggg", 3 * PART + 1), // right sizes, but a zero-filled hole in the middle; a whole copy in a channel
};
const holey = Buffer.from(text.ggg);
holey.fill(0, PART + 100, PART + 600);

const dir = mkdtempSync(join(tmpdir(), "recover-test-"));
const home = join(dir, "home");
const store = join(home, "shared", "agents", "transcripts", "filestore.db");
const sh = (db, sql) => spawnSync(cli.path, [db, sql], { encoding: "utf8" });

function reassemble(db, zone, name) {
    return Buffer.concat(db.prepare("SELECT data FROM db_file_data WHERE zoneid = ? AND name = ? ORDER BY partidx").all(zone, name).map((r) => Buffer.from(r.data)));
}

beforeAll(() => {
    if (!cli) return;
    const db = createFilestore(store);
    putFile(db, "agent:aaa:current", "output", text.aaa, { dropParts: [2] });
    putFile(db, "agent:aaa:current", "output.tsidx", Buffer.from("tsidx"));
    putFile(db, "agent:aaa:current", "output.idx", Buffer.from("idx"));
    putFile(db, "agent:aaa:current", "output.state.json", Buffer.from('{"old":true}'));
    putFile(db, "agent:bbb:current", "output", text.bbb);
    putFile(db, "agent:ccc:current", "output", text.ccc, { dropParts: [1] });
    putFile(db, "agent:ddd:current", "output", text.ddd, { dropParts: [3] });
    putFile(db, "agent:eee:current", "output", transcript("eee", PART + 10));
    putFile(db, "agent:eee:current", "output.idx", Buffer.alloc(3 * PART, 7), { dropParts: [1] });
    putFile(db, "agent:fff:current", "output", text.fff);
    putFile(db, "agent:ggg:current", "output", holey);
    db.exec("PRAGMA user_version = 2");
    db.exec("PRAGMA wal_checkpoint(TRUNCATE)");
    db.close();

    createChannel(home, "chA", "0.1.0", [
        { oid: "blk-aaa-old", meta: { agentId: "aaa" }, output: text.aaa.subarray(0, 2 * PART) },
        { oid: "blk-ddd", meta: { agentId: "ddd" }, output: text.ddd.subarray(0, 2 * PART) },
        { oid: "blk-unrelated", meta: { agentId: "zzz" }, output: transcript("zzz", 100) },
    ]);
    createChannel(home, "chB", "0.2.0", [
        { oid: "blk-aaa-broken", meta: { agentId: "aaa" }, output: Buffer.concat([text.aaa, transcript("more", PART)]), dropParts: [1] },
        { oid: "blk-aaa", meta: { agentId: "aaa" }, output: text.aaa },
        { oid: "blk-fff", meta: { "agent:id": "fff" }, output: text.fff },
        { oid: "blk-ggg", meta: { agentId: "ggg" }, output: text.ggg },
    ]);

    // Corrupt fff on disk: a whole page from the middle of its part 3 is lost (read back as zeros).
    const file = readFileSync(store);
    // A stretch that includes the zone's tag, so it can only be found in fff's own pages.
    const mark = text.fff.indexOf('"tag":"fff"', 3 * PART + 30000);
    const at = file.indexOf(text.fff.subarray(mark, mark + 64));
    expect(at).toBeGreaterThan(0);
    const page = Math.floor(at / 4096);
    file.fill(0, page * 4096, page * 4096 + 4096);
    writeFileSync(store, file);
});

afterAll(() => {
    if (process.env.KEEP_RECOVER_TEST_DIR) console.log(`kept ${dir}`);
    else rmSync(dir, { recursive: true, force: true });
});

const run = (...extra) => main(["--work", join(dir, "work"), "--source", store, "--sqlite3", cli.path, "--allow-running", ...extra]);
const report = () => JSON.parse(readFileSync(join(dir, "work", "report.json"), "utf8"));

suite("the offline recovery tool", () => {
    it("starts from a store that is really damaged", () => {
        const check = sh(store, "PRAGMA integrity_check(5);");
        expect(check.stdout.trim()).not.toBe("ok");
    });

    it("a dry run finds the damage, plans the rebuilds and changes nothing", async () => {
        const originals = [store, `${store}-wal`, `${store}-shm`].filter(existsSync).map((p) => [p, statSync(p).size, statSync(p).mtimeMs]);
        expect(await run()).toBe(0);
        const r = report();
        expect(r.mode).toBe("dry run");
        expect(r.integrity.ok).toBe(false);
        expect(r.originalsUnchanged).toBe(true);
        expect(r.userVersion.original).toBe(2);
        for (const [p, size, mtime] of originals) expect([statSync(p).size, statSync(p).mtimeMs]).toEqual([size, mtime]);

        const byZone = Object.fromEntries(r.plan.filter((p) => p.name === "output").map((p) => [p.zone, p]));
        expect(Object.keys(byZone).sort()).toEqual(["agent:aaa:current", "agent:ccc:current", "agent:ddd:current", "agent:fff:current", "agent:ggg:current"]);
        expect(r.damage.find((d) => d.zone === "agent:ggg:current").kind, "sizes add up, only the zeros give it away").toBe("nul-bytes");
        expect(byZone["agent:ggg:current"].action).toBe("rebuild");
        expect(byZone["agent:aaa:current"].action).toBe("rebuild");
        expect(byZone["agent:aaa:current"].source.blockOid, "the largest whole copy, not the larger one with a gap").toBe("blk-aaa");
        expect(byZone["agent:ccc:current"].action).toBe("no-source");
        expect(byZone["agent:ddd:current"].action).toBe("source-shorter");
        expect(byZone["agent:fff:current"].action).toBe("rebuild");
        expect(r.damage.find((d) => d.zone === "agent:fff:current").kind, "a lost page is found whether it came back short or as zeros").toMatch(/nul-bytes|tail-missing|gap|size-mismatch/);
        expect(byZone["agent:fff:current"].source.blockOid, "found through the legacy agent:id meta key").toBe("blk-fff");
        expect(r.plan.find((p) => p.zone === "agent:eee:current" && p.name === "output.idx").action).toBe("drop-derived");
        expect(r.damage.map((d) => d.zone)).not.toContain("agent:bbb:current");
        expect(r.plan.every((p) => p.applied === undefined)).toBe(true);
        expect(existsSync(join(dir, "work", "recovered.db"))).toBe(true);
    }, 120_000);

    it("--apply rebuilds the damaged zones from the per-channel copies and leaves the rest alone", async () => {
        expect(await run("--recovered", join(dir, "work", "recovered.db"), "--apply")).toBe(0);
        const r = report();
        expect(r.mode).toBe("apply");
        expect(r.final.integrity.ok).toBe(true);
        expect(r.userVersion.recovered).toBe(2);
        expect(r.originalsUnchanged).toBe(true);

        const db = new DatabaseSync(join(dir, "work", "recovered.db"), { readOnly: true });
        try {
            expect(reassemble(db, "agent:aaa:current", "output").equals(text.aaa)).toBe(true);
            expect(reassemble(db, "agent:fff:current", "output").equals(text.fff)).toBe(true);
            expect(reassemble(db, "agent:ggg:current", "output").equals(text.ggg), "the hole is filled from the copy").toBe(true);
            expect(reassemble(db, "agent:bbb:current", "output").equals(text.bbb), "a healthy zone is untouched").toBe(true);
            expect(db.prepare("SELECT COUNT(*) n FROM db_wave_file WHERE zoneid = 'agent:aaa:current' AND name IN ('output.tsidx','output.idx')").get().n).toBe(0);
            const overlay = JSON.parse(reassemble(db, "agent:aaa:current", "output.state.json").toString());
            expect(overlay.highWaterMark).toBe(text.aaa.toString().split("\n").filter((l) => l.trim()).length);
            const row = db.prepare("SELECT size, gen, lines FROM db_wave_file WHERE zoneid = 'agent:aaa:current' AND name = 'output'").get();
            expect(row).toEqual({ size: text.aaa.length, gen: null, lines: null });
            expect(db.prepare("SELECT COUNT(*) n FROM db_wave_file WHERE zoneid = 'agent:eee:current' AND name = 'output.idx'").get().n, "a damaged derived file is dropped").toBe(0);
            // What cannot be rebuilt is still reported, not hidden.
            const left = findDamage(db).damage.map((d) => `${d.zone}/${d.name}`).sort();
            expect(left).toEqual(["agent:ccc:current/output", "agent:ddd:current/output"]);
        } finally {
            db.close();
        }
        const actions = Object.fromEntries(r.plan.filter((p) => p.name === "output").map((p) => [p.zone, [p.action, p.applied]]));
        expect(actions["agent:aaa:current"]).toEqual(["rebuild", true]);
        expect(actions["agent:ccc:current"][1]).toBeUndefined();
    }, 120_000);

    it("--allow-shorter rebuilds a zone from a shorter copy", async () => {
        expect(await run("--recovered", join(dir, "work", "recovered.db"), "--apply", "--allow-shorter")).toBe(0);
        const db = new DatabaseSync(join(dir, "work", "recovered.db"), { readOnly: true });
        try {
            expect(reassemble(db, "agent:ddd:current", "output").equals(text.ddd.subarray(0, 2 * PART))).toBe(true);
        } finally {
            db.close();
        }
    }, 120_000);

    it("--skip-content-scan leaves zero-filled holes unreported", async () => {
        const work = join(dir, "work2");
        expect(await main(["--work", work, "--source", store, "--sqlite3", cli.path, "--allow-running", "--skip-content-scan"])).toBe(0);
        const r = JSON.parse(readFileSync(join(work, "report.json"), "utf8"));
        expect(r.damage.map((d) => d.zone)).not.toContain("agent:ggg:current");
    }, 120_000);

    it("refuses a work folder inside the store's folder", async () => {
        await expect(main(["--work", join(home, "shared", "agents", "transcripts", "w"), "--source", store, "--sqlite3", cli.path, "--allow-running"])).rejects.toThrow(/overlaps/);
    });
});

describe("the fixture", () => {
    it("builds its tables and triggers from the same SQL as srv's migrations", () => {
        const rust = readFileSync(new URL("../../crates/srv/src/backend/storage/migrations.rs", import.meta.url), "utf8").replace(/\s+/g, " ");
        const norm = (s) => s.replace(/\s+/g, " ").trim();
        for (const t of TRIGGERS) expect(rust, norm(t).slice(0, 60)).toContain(norm(t).replace(/;$/, ""));
        for (const col of TABLES.match(/ALTER TABLE db_wave_file ADD COLUMN [^;]+/g)) expect(rust).toContain(col.replace("ALTER TABLE db_wave_file ADD COLUMN ", '"ALTER TABLE db_wave_file ADD COLUMN ') + '"');
        expect(rust).toContain("partidx INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (zoneid, name, partidx)");
    });
});
