// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A small shared transcript store and the per-channel stores around it, built the
// way srv builds them (the DDL below is copied from `run_filestore_migrations` in
// crates/srv/src/backend/storage/migrations.rs; recover.test.mjs checks it still
// matches), for testing recover.mjs. Not used by the tool itself.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

export const PART = 64 * 1024;

export const TABLES = `
CREATE TABLE IF NOT EXISTS db_wave_file (
    zoneid TEXT NOT NULL,
    name TEXT NOT NULL,
    size INTEGER NOT NULL DEFAULT 0,
    createdts INTEGER NOT NULL DEFAULT 0,
    modts INTEGER NOT NULL DEFAULT 0,
    opts TEXT NOT NULL DEFAULT '{}',
    meta TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (zoneid, name)
);
CREATE TABLE IF NOT EXISTS db_file_data (
    zoneid TEXT NOT NULL,
    name TEXT NOT NULL,
    partidx INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (zoneid, name, partidx)
);
ALTER TABLE db_wave_file ADD COLUMN gen TEXT;
ALTER TABLE db_wave_file ADD COLUMN lines INTEGER;
ALTER TABLE db_wave_file ADD COLUMN lines_size INTEGER;
ALTER TABLE db_wave_file ADD COLUMN lines_tail INTEGER;
ALTER TABLE db_wave_file ADD COLUMN lines_modts INTEGER;
ALTER TABLE db_wave_file ADD COLUMN rev INTEGER;
ALTER TABLE db_wave_file ADD COLUMN lines_rev INTEGER;
ALTER TABLE db_wave_file ADD COLUMN incarnation BLOB;`;

export const TRIGGERS = [
    `CREATE TRIGGER IF NOT EXISTS db_file_data_rev_delete
         AFTER DELETE ON db_file_data
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = OLD.zoneid AND name = OLD.name;
         END;`,
    `CREATE TRIGGER IF NOT EXISTS db_file_data_rev_update
         BEFORE UPDATE OF data ON db_file_data
         WHEN substr(NEW.data, 1, length(OLD.data)) IS NOT OLD.data
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = OLD.zoneid AND name = OLD.name;
         END;`,
    `CREATE TRIGGER IF NOT EXISTS db_wave_file_incarnation
         AFTER INSERT ON db_wave_file
         BEGIN
             UPDATE db_wave_file SET incarnation = randomblob(8)
             WHERE zoneid = NEW.zoneid AND name = NEW.name;
         END;`,
];

/** Lines of `{"n":i,"t":"..."}` totalling at least `bytes`. */
export function transcript(tag, bytes) {
    const lines = [];
    let n = 0;
    let size = 0;
    while (size < bytes) {
        const line = `{"n":${n++},"tag":"${tag}","t":"${"x".repeat(60 + (n % 37))}"}\n`;
        lines.push(line);
        size += line.length;
    }
    return Buffer.from(lines.join(""));
}

export function createFilestore(path) {
    mkdirSync(join(path, ".."), { recursive: true });
    const db = new DatabaseSync(path);
    db.exec("PRAGMA journal_mode=WAL; PRAGMA page_size=4096;");
    for (const stmt of TABLES.split(";").map((s) => s.trim()).filter(Boolean)) db.exec(stmt);
    for (const t of TRIGGERS) db.exec(t);
    return db;
}

export function putFile(db, zone, name, bytes, { dropParts = [] } = {}) {
    db.prepare("INSERT INTO db_wave_file (zoneid, name, size, createdts, modts) VALUES (?, ?, ?, 1, 1)").run(zone, name, bytes.length);
    const ins = db.prepare("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?, ?, ?, ?)");
    for (let i = 0, at = 0; at < bytes.length; i++, at += PART) {
        if (!dropParts.includes(i)) ins.run(zone, name, i, bytes.subarray(at, at + PART));
    }
}

/** A channel's data dir with one agent block per entry of `blocks` { oid, meta, output }. */
export function createChannel(home, channel, version, blocks) {
    const data = join(home, "channels", channel, "versions", version, "data");
    mkdirSync(join(data, "db"), { recursive: true });
    const objects = new DatabaseSync(join(data, "db", "objects.db"));
    objects.exec("CREATE TABLE db_block (oid TEXT PRIMARY KEY, data TEXT NOT NULL)");
    const fs = createFilestore(join(data, "db", "filestore.db"));
    for (const b of blocks) {
        // The app stores the JSON as a BLOB in a column declared TEXT.
        objects.prepare("INSERT INTO db_block (oid, data) VALUES (?, ?)").run(b.oid, Buffer.from(JSON.stringify({ oid: b.oid, meta: b.meta })));
        putFile(fs, b.oid, "output", b.output, { dropParts: b.dropParts });
    }
    objects.close();
    fs.exec("PRAGMA wal_checkpoint(TRUNCATE)");
    fs.close();
    return data;
}
