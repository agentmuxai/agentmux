// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What is wrong with a (recovered) filestore: for every file, its parts must run
// 0..N with no gap and add up to the recorded size. The parts are 64 KB each
// (`PART_DATA_SIZE` in crates/srv/src/backend/storage/filestore/core.rs).

export const PART = 64 * 1024;

const INVENTORY = `
WITH parts AS (
    SELECT zoneid, name, COUNT(*) AS n, MIN(partidx) AS lo, MAX(partidx) AS hi, SUM(LENGTH(data)) AS bytes
      FROM db_file_data GROUP BY zoneid, name)
SELECT f.zoneid AS zoneid, f.name AS name, f.size AS size, p.n AS n, p.lo AS lo, p.hi AS hi, p.bytes AS bytes
  FROM db_wave_file f LEFT JOIN parts p ON p.zoneid = f.zoneid AND p.name = f.name
UNION ALL
SELECT p.zoneid, p.name, NULL, p.n, p.lo, p.hi, p.bytes
  FROM parts p LEFT JOIN db_wave_file f ON f.zoneid = p.zoneid AND f.name = p.name
 WHERE f.zoneid IS NULL`;

/** Missing part-index ranges of one file, and the bytes in its unbroken prefix. */
function partMap(db, zone, name) {
    const rows = db.prepare("SELECT partidx, LENGTH(data) AS len FROM db_file_data WHERE zoneid = ? AND name = ? ORDER BY partidx").all(zone, name);
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
 * size-mismatch (more bytes than the recorded size).
 */
export function findDamage(db) {
    const damage = [];
    let files = 0;
    let zones = new Set();
    for (const r of db.prepare(INVENTORY).all()) {
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
 * Reads every `output` part; skip with --skip-content-scan.
 */
export function findNulHoles(db) {
    return db
        .prepare("SELECT zoneid, name, MIN(partidx) AS first, COUNT(*) AS n FROM db_file_data WHERE name = 'output' AND instr(data, x'00') > 0 GROUP BY zoneid, name")
        .all()
        .map((r) => ({ zone: r.zoneid, name: r.name, first: r.first, parts: r.n }));
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
