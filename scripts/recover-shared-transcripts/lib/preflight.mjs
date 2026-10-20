// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Checks before anything is copied: the machine is stopped, there is room, and
// the work folder can't be mistaken for the store.

import { spawnSync } from "node:child_process";
import { existsSync, statSync, statfsSync } from "node:fs";
import { basename, dirname, join, resolve, sep } from "node:path";

/** The store and its sidecars, with their sizes; throws if the database is missing. */
export function locateStore(dbPath) {
    const db = resolve(dbPath);
    if (!existsSync(db)) throw new Error(`${db} does not exist`);
    const files = [db, `${db}-wal`, `${db}-shm`].map((path) => {
        if (!existsSync(path)) return { path, exists: false, size: 0, mtimeMs: 0 };
        const s = statSync(path);
        return { path, exists: true, size: s.size, mtimeMs: s.mtimeMs };
    });
    return { db, files };
}

/** `~/.agentmux` for a store at `<home>/shared/agents/transcripts/filestore.db`. */
export function homeOf(dbPath) {
    return resolve(dirname(dbPath), "..", "..", "..");
}

/** Lines of the process list that look like a running AgentMux server. */
export function runningInstances() {
    const run =
        process.platform === "win32"
            ? spawnSync("tasklist", ["/FO", "CSV", "/NH"], { encoding: "utf8" })
            : spawnSync("ps", ["-A", "-o", "pid=,args="], { encoding: "utf8" });
    if (run.status !== 0) return { checked: false, lines: [] };
    const lines = run.stdout.split("\n").filter((l) => /agentmux-srv|agentmux-launcher/i.test(l));
    return { checked: true, lines };
}

/** Bytes free where `dir` is (or would be). */
export function freeBytes(dir) {
    let at = resolve(dir);
    while (!existsSync(at) && dirname(at) !== at) at = dirname(at);
    const s = statfsSync(at);
    return Number(s.bavail) * Number(s.bsize);
}

/** A copy and a recovered file each need about the store's size, plus slack. */
export function requiredBytes(store) {
    const total = store.files.reduce((n, f) => n + f.size, 0);
    return Math.ceil(total * 2.05) + 256 * 1024 * 1024;
}

/** Refuses a work folder that is, or is inside, the store's folder (or the reverse). */
export function checkWorkDir(work, store) {
    const w = resolve(work) + sep;
    const s = dirname(store.db) + sep;
    if (w.startsWith(s) || s.startsWith(w)) {
        throw new Error(`--work ${resolve(work)} overlaps the store's folder ${dirname(store.db)}; pick a folder elsewhere`);
    }
}

export const copyNames = (store) => store.files.filter((f) => f.exists).map((f) => ({ from: f.path, name: basename(f.path) }));
export const workPath = (work, name) => join(resolve(work), name);
