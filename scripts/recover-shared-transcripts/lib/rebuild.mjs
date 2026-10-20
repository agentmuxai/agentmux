// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Writing a zone back into the recovered database, the way `replace_file` and
// `seed_global_zone` do (crates/srv/src/backend/storage/filestore/replace.rs,
// crates/srv/src/backend/transcript_backfill.rs): the new `output` replaces the
// old one and the files derived from it are dropped, in one transaction. The
// line counters (`gen`, `lines`, ...) are left NULL, which the app reads as "not
// counted yet" and recounts on first use.

import { PART } from "./analyze.mjs";

const SNAPSHOT = "output.state.json";
const DERIVED_OF_OUTPUT = ["output.tsidx", "output.idx", SNAPSHOT];

const nonBlankLines = (bytes) =>
    bytes
        .toString("utf8")
        .split("\n")
        .filter((l) => l.trim() !== "").length;

/** The overlay `seed_global_zone` writes beside a seeded transcript. */
export function overlayFor(bytes) {
    return JSON.stringify({
        schemaVersion: 2,
        savedAt: "",
        highWaterMark: nonBlankLines(bytes),
        sourceBlockId: "",
        documentState: {
            collapsedNodeIds: [],
            pinnedNodeIds: [],
            scrollPosition: 0,
            filter: { showThinking: false, showSuccessfulTools: true, showFailedTools: true, showIncoming: true, showOutgoing: true },
        },
        paneState: { detailsOpen: false },
    });
}

/** Whether this database has the line-counter columns (an old store may not). */
const counterCache = new WeakMap();
function counterColumns(db) {
    if (!counterCache.has(db)) {
        counterCache.set(db, db.prepare("PRAGMA table_info(db_wave_file)").all().some((c) => c.name === "lines_rev"));
    }
    return counterCache.get(db);
}

function writeFile(db, zone, name, bytes, now, template) {
    db.prepare("DELETE FROM db_file_data WHERE zoneid = ? AND name = ?").run(zone, name);
    const had = db.prepare("SELECT opts, meta, createdts FROM db_wave_file WHERE zoneid = ? AND name = ?").get(zone, name);
    if (had) {
        const counted = counterColumns(db) ? ", gen = NULL, lines = NULL, lines_size = NULL, lines_tail = NULL, lines_modts = NULL, lines_rev = NULL" : "";
        db.prepare(`UPDATE db_wave_file SET size = ?, modts = ?${counted} WHERE zoneid = ? AND name = ?`).run(bytes.length, now, zone, name);
    } else {
        db.prepare("INSERT INTO db_wave_file (zoneid, name, size, createdts, modts, opts, meta) VALUES (?, ?, ?, ?, ?, ?, ?)").run(
            zone,
            name,
            bytes.length,
            now,
            now,
            template.opts,
            template.meta,
        );
    }
    const insert = db.prepare("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?, ?, ?, ?)");
    for (let i = 0, at = 0; at < bytes.length; i++, at += PART) insert.run(zone, name, i, bytes.subarray(at, at + PART));
}

/** The options and metadata most `output` rows carry, for a row that has to be made. */
export function outputTemplate(db) {
    const row = db
        .prepare("SELECT opts, meta, COUNT(*) AS n FROM db_wave_file WHERE name = 'output' GROUP BY opts, meta ORDER BY n DESC LIMIT 1")
        .get();
    return row ? { opts: row.opts, meta: row.meta } : { opts: "{}", meta: "{}" };
}

/** Replace a zone's `output` with `bytes` and drop what was derived from it. */
export function rebuildZone(db, zone, bytes, now, template) {
    db.exec("BEGIN IMMEDIATE");
    try {
        writeFile(db, zone, "output", bytes, now, template);
        for (const name of DERIVED_OF_OUTPUT) {
            db.prepare("DELETE FROM db_file_data WHERE zoneid = ? AND name = ?").run(zone, name);
            db.prepare("DELETE FROM db_wave_file WHERE zoneid = ? AND name = ?").run(zone, name);
        }
        writeFile(db, zone, SNAPSHOT, Buffer.from(overlayFor(bytes)), now, template);
        db.exec("COMMIT");
    } catch (e) {
        db.exec("ROLLBACK");
        throw e;
    }
}

/** Drop files the app rebuilds by itself (indexes, the view overlay). */
export function dropFile(db, zone, name) {
    db.exec("BEGIN IMMEDIATE");
    try {
        db.prepare("DELETE FROM db_file_data WHERE zoneid = ? AND name = ?").run(zone, name);
        db.prepare("DELETE FROM db_wave_file WHERE zoneid = ? AND name = ?").run(zone, name);
        db.exec("COMMIT");
    } catch (e) {
        db.exec("ROLLBACK");
        throw e;
    }
}
