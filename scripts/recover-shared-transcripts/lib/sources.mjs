// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Where a lost transcript can come from: every channel and version keeps its own
// copy of each pane's `output` in its own filestore (written before the shared
// mirror). The shared zone `agent:<definition id>:current` belongs to the blocks
// whose meta `agentId` (or the legacy `agent:id`) is that definition id. This is
// the selection `transcript_backfill.rs` makes, with one addition: a copy whose
// parts have a gap, or don't add up to its size, is not used.

import { existsSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { partIndex, readFileBytes, scanFrames } from "./frames.mjs";
import { open } from "./sqlite.mjs";

const subdirs = (dir) => {
    try {
        return readdirSync(dir)
            .map((n) => join(dir, n))
            .filter((p) => statSync(p).isDirectory());
    } catch {
        return [];
    }
};

// Every data dir under <home>/channels/<channel>/versions/<version> and <home>/dev/<branch>[/<sub>] that holds both stores.
export function collectDataDirs(home) {
    const dirs = [];
    const add = (data) => {
        if (existsSync(join(data, "db", "objects.db")) && existsSync(join(data, "db", "filestore.db"))) dirs.push(data);
    };
    for (const ch of subdirs(join(home, "channels"))) for (const v of subdirs(join(ch, "versions"))) add(join(v, "data"));
    for (const br of subdirs(join(home, "dev"))) {
        add(join(br, "data"));
        for (const sub of subdirs(br)) add(join(sub, "data"));
    }
    return dirs;
}

/** [{ defId, blockOid }] for blocks whose meta names one of `want` (a Set). */
function agentBlocks(objectsDb, want) {
    const db = open(objectsDb);
    try {
        const out = [];
        for (const row of db.prepare("SELECT data FROM db_block").all()) {
            let v;
            try {
                // Stored as a BLOB although the column is declared TEXT.
                v = JSON.parse(Buffer.from(row.data).toString("utf8"));
            } catch {
                continue;
            }
            const meta = v?.meta ?? {};
            const defId = typeof meta.agentId === "string" ? meta.agentId : meta["agent:id"];
            if (typeof defId === "string" && typeof v.oid === "string" && want.has(defId)) out.push({ defId, blockOid: v.oid });
        }
        return out;
    } finally {
        db.close();
    }
}

/**
 * The recorded size of a zone's `output` and whether its parts are whole: no gap, adding up to
 * the size and, with `contentScan`, no NUL bytes (a lost page read back as zeros). Null if none.
 */
function outputState(db, zone, contentScan) {
    const f = db.prepare("SELECT size FROM db_wave_file WHERE zoneid = ? AND name = 'output'").get(zone);
    if (!f || Number(f.size) <= 0) return null;
    const parts = partIndex(db, zone, "output");
    const bytes = parts.reduce((sum, p) => sum + p.len, 0);
    let whole = parts.length > 0 && parts.every((p, i) => p.partidx === i) && bytes === Number(f.size);
    if (whole && contentScan) {
        whole = db.prepare("SELECT COUNT(*) AS n FROM db_file_data WHERE zoneid = ? AND name = 'output' AND instr(data, x'00') > 0").get(zone).n === 0;
        if (whole) {
            // Frame by frame: a candidate is never assembled in memory just to be checked.
            const frames = scanFrames(db, { zone, name: "output" });
            whole = frames.bad.length === 0 && frames.nul.length === 0;
        }
    }
    return { size: Number(f.size), whole };
}

/**
 * For each definition id in `want`, every candidate copy found and the one to use:
 * the largest whole copy (no gap, the right size, no NUL bytes). -> Map(defId -> { best: {dataDir, blockOid, size} | null, candidates: [...] })
 */
export function findSources(home, want, { contentScan = true } = {}) {
    const found = new Map([...want].map((id) => [id, { best: null, candidates: [] }]));
    const dataDirs = collectDataDirs(home);
    for (const data of dataDirs) {
        let blocks;
        try {
            blocks = agentBlocks(join(data, "db", "objects.db"), want);
        } catch {
            continue;
        }
        if (!blocks.length) continue;
        let fdb;
        try {
            fdb = open(join(data, "db", "filestore.db"));
        } catch {
            continue;
        }
        try {
            for (const { defId, blockOid } of blocks) {
                let state = null;
                try {
                    state = outputState(fdb, blockOid, contentScan);
                } catch {
                    continue;
                }
                if (!state) continue;
                const entry = found.get(defId);
                const cand = { dataDir: data, blockOid, size: state.size, whole: state.whole };
                entry.candidates.push(cand);
                if (state.whole && (!entry.best || cand.size > entry.best.size)) entry.best = cand;
            }
        } finally {
            fdb.close();
        }
    }
    return { dataDirsScanned: dataDirs.length, byAgent: found };
}

/** The bytes of a chosen copy (its parts in order, frames decoded). */
export function readSource(cand) {
    const db = open(join(cand.dataDir, "db", "filestore.db"));
    try {
        return readFileBytes(db, cand.blockOid, "output");
    } finally {
        db.close();
    }
}
