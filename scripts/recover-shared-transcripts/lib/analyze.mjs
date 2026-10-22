// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What is wrong with a (recovered) filestore: for every file, its parts must run
// 0..N with no gap and add up to the recorded size. The parts are 64 KB each
// (`PART_DATA_SIZE` in crates/srv/src/backend/storage/filestore/core.rs). A part
// is a row of `db_file_data` or a part of a compressed frame (lib/frames.mjs).

import { hasFrames, partIndex, scanFrames } from "./frames.mjs";

export const PART = 64 * 1024;

const ROW_PARTS = "SELECT zoneid, name, COUNT(*) AS n, MIN(partidx) AS lo, MAX(partidx) AS hi, SUM(LENGTH(data)) AS bytes FROM db_file_data GROUP BY zoneid, name";
// A frame's parts count as parts, each once: a part that has a row as well is the row, as
// the app reads it (frames.rs: a row always wins; a build that knows nothing of frames may
// write one into a framed part). This is `partIndex` in SQL, for every file at once.
const FRAMED_PARTS = `frame_parts(zoneid, name, partidx, k, cnt) AS (
    SELECT zoneid, name, first_part, 1, part_count FROM db_file_frame
    UNION ALL
    SELECT zoneid, name, partidx + 1, k + 1, cnt FROM frame_parts WHERE k < cnt),
all_parts AS (
    SELECT zoneid, name, partidx, LENGTH(data) AS len FROM db_file_data
    UNION ALL
    SELECT zoneid, name, partidx, ${64 * 1024} FROM frame_parts fp
     WHERE NOT EXISTS (SELECT 1 FROM db_file_data d WHERE d.zoneid = fp.zoneid AND d.name = fp.name AND d.partidx = fp.partidx)),
parts AS (SELECT zoneid, name, COUNT(*) AS n, MIN(partidx) AS lo, MAX(partidx) AS hi, SUM(len) AS bytes FROM all_parts GROUP BY zoneid, name)`;

const inventory = (framed) => `
${framed ? `WITH RECURSIVE ${FRAMED_PARTS}` : `WITH parts AS (${ROW_PARTS})`}
SELECT f.zoneid AS zoneid, f.name AS name, f.size AS size, p.n AS n, p.lo AS lo, p.hi AS hi, p.bytes AS bytes
  FROM db_wave_file f LEFT JOIN parts p ON p.zoneid = f.zoneid AND p.name = f.name
UNION ALL
SELECT p.zoneid, p.name, NULL, p.n, p.lo, p.hi, p.bytes
  FROM parts p LEFT JOIN db_wave_file f ON f.zoneid = p.zoneid AND f.name = p.name
 WHERE f.zoneid IS NULL`;

/** The bytes a file's parts hold, rows and frames. */
function storedBytes(db, zone, name) {
    return partIndex(db, zone, name).reduce((sum, p) => sum + p.len, 0);
}

/** Missing part-index ranges of one file, and the bytes in its unbroken prefix. */
function partMap(db, zone, name) {
    const rows = partIndex(db, zone, name);
    const missing = [];
    let next = 0;
    let prefixBytes = 0;
    let inPrefix = true;
    for (const r of rows) {
        if (r.partidx > next) {
            missing.push(r.partidx - 1 === next ? `${next}` : `${next}-${r.partidx - 1}`);
            inPrefix = false;
        }
        if (inPrefix) prefixBytes += Number(r.len);
        next = r.partidx + 1;
    }
    return { missing, prefixBytes };
}

/**
 * Every damaged file: { zone, name, kind, detail, size, partsBytes, prefixBytes }.
 * Kinds: orphan-parts (no file row), no-parts (a size but no parts), gap (a part
 * is missing), tail-missing (the parts end before the recorded size) and
 * size-mismatch (more bytes than the recorded size). A compressed frame that doesn't decode
 * is found by [`allDamage`]'s content scan.
 */
export function findDamage(db) {
    const damage = [];
    let files = 0;
    let zones = new Set();
    for (const r of db.prepare(inventory(hasFrames(db))).all()) {
        files++;
        zones.add(r.zoneid);
        const size = r.size === null ? null : Number(r.size);
        const partsBytes = r.bytes === null ? 0 : Number(r.bytes);
        const base = { zone: r.zoneid, name: r.name, size, partsBytes };
        if (size === null) {
            damage.push({ ...base, kind: "orphan-parts", detail: `${r.n} part(s) and no file row`, prefixBytes: 0 });
        } else if (r.n === null) {
            if (size > 0) damage.push({ ...base, kind: "no-parts", detail: `size ${size} and no parts`, prefixBytes: 0 });
        } else if (r.lo !== 0 || r.hi + 1 !== r.n) {
            const { missing, prefixBytes } = partMap(db, r.zoneid, r.name);
            const shown = missing.length > 8 ? `${missing.slice(0, 8).join(", ")} and ${missing.length - 8} more` : missing.join(", ");
            damage.push({ ...base, kind: "gap", detail: `part(s) ${shown || `0-${r.lo - 1}`} missing of ${r.hi + 1}`, prefixBytes });
        } else if (partsBytes < size) {
            damage.push({ ...base, kind: "tail-missing", detail: `parts hold ${partsBytes} of ${size} bytes`, prefixBytes: partsBytes });
        } else if (partsBytes > size) {
            damage.push({ ...base, kind: "size-mismatch", detail: `parts hold ${partsBytes} bytes, size says ${size}`, prefixBytes: partsBytes });
        }
    }
    return { damage, files, zones: zones.size };
}

/**
 * `output` files with NUL bytes in a part. A transcript is lines of JSON text, so a NUL is a
 * zero-filled hole where a lost page was read back as zeros, which a size check cannot see.
 * Reads every `output` part, and decodes every `output` frame (pass `frames` from
 * [`scanFrames`] to reuse a scan); skip with --skip-content-scan.
 */
export function findNulHoles(db, frames = scanFrames(db)) {
    const holes = new Map();
    for (const r of db
        .prepare("SELECT zoneid, name, MIN(partidx) AS first, COUNT(*) AS n FROM db_file_data WHERE name = 'output' AND instr(data, x'00') > 0 GROUP BY zoneid, name")
        .all()) {
        holes.set(`${r.zoneid}\u0000${r.name}`, { zone: r.zoneid, name: r.name, first: Number(r.first), parts: Number(r.n) });
    }
    for (const h of frames.nul) {
        const key = `${h.zone}\u0000${h.name}`;
        const had = holes.get(key);
        if (had) {
            had.parts += h.parts;
            had.first = Math.min(had.first, h.first);
        } else holes.set(key, { ...h });
    }
    return [...holes.values()];
}

/** Rows `.recover` could not place: its `lost_and_found` table, if it made one. */
export function lostAndFound(db) {
    const has = db.prepare("SELECT 1 FROM sqlite_schema WHERE name = 'lost_and_found'").get();
    if (!has) return { table: false, rows: 0 };
    return { table: true, rows: Number(db.prepare("SELECT COUNT(*) AS n FROM lost_and_found").get().n) };
}

/** `agent:<definition id>:current` -> the definition id; null for any other zone. */
export function agentOfZone(zone) {
    const m = /^agent:([A-Za-z0-9_-]+):current$/.exec(zone);
    return m ? m[1] : null;
}

/** Files the app derives from `output` and rebuilds on demand. */
export const DERIVED = new Set(["output.tsidx", "output.idx", "output.state.json"]);

/** Add `detail` to the file's damage entry, or make one of `kind`. */
function note(db, found, zone, name, kind, detail) {
    const known = found.damage.find((d) => d.zone === zone && d.name === name);
    if (known) {
        known.detail += `; ${detail}`;
        return;
    }
    const f = db.prepare("SELECT size FROM db_wave_file WHERE zoneid = ? AND name = ?").get(zone, name);
    found.damage.push({ zone, name, kind, detail, size: f ? Number(f.size) : null, partsBytes: storedBytes(db, zone, name), prefixBytes: 0 });
}

/**
 * Everything wrong with a database: the structural damage of [`findDamage`] and, unless
 * `contentScan` is false, compressed frames that don't decode and the NUL holes of
 * [`findNulHoles`] (added to the file's entry when it already has one).
 */
export function allDamage(db, { contentScan = true } = {}) {
    const found = findDamage(db);
    if (contentScan) {
        const frames = scanFrames(db, { name: null });
        for (const b of frames.bad) note(db, found, b.zone, b.name, "bad-frame", `the frame from part ${b.first} doesn't decode (${b.error})`);
        for (const hole of findNulHoles(db, { nul: frames.nul.filter((h) => h.name === "output") })) {
            note(db, found, hole.zone, hole.name, "nul-bytes", `${hole.parts} part(s) hold NUL bytes, from part ${hole.first}`);
        }
    }
    return found;
}
