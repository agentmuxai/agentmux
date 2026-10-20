// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The two ways this tool talks to SQLite: the `sqlite3` command-line shell (for
// `PRAGMA integrity_check` and `.recover`, which exist only there) and Node's
// built-in `node:sqlite` (for the queries and the rebuild on a database that
// has already been recovered). Nothing here opens the original store.

import { spawn, spawnSync } from "node:child_process";
import { DatabaseSync } from "node:sqlite";

/** The first SQLite release without the WAL-reset corruption bug. */
export const MIN_SQLITE = [3, 51, 3];

export function parseVersion(text) {
    const m = /(\d+)\.(\d+)\.(\d+)/.exec(text ?? "");
    return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

export function atLeast(version, min) {
    for (let i = 0; i < 3; i++) {
        if (version[i] !== min[i]) return version[i] > min[i];
    }
    return true;
}

/** The sqlite3 shell to use: `explicit`, else $SQLITE3, else the one on PATH. */
export function findSqlite3(explicit, env = process.env) {
    const tried = [];
    for (const candidate of [explicit, env.SQLITE3, "sqlite3", "sqlite3.exe"]) {
        if (!candidate) continue;
        const run = spawnSync(candidate, ["--version"], { encoding: "utf8" });
        tried.push(candidate);
        if (run.status === 0) {
            const version = parseVersion(run.stdout);
            return { path: candidate, version, text: run.stdout.trim() };
        }
    }
    throw new Error(
        `no sqlite3 shell found (tried: ${tried.join(", ") || "nothing"}). Download the "sqlite-tools" zip for your platform from https://sqlite.org/download.html and pass --sqlite3 <path>; it needs to be ${MIN_SQLITE.join(".")} or newer.`,
    );
}

/** Run `sql` in the shell against `db`; returns { status, stdout, stderr }. */
export function shell(cli, db, sql, extra = []) {
    const run = spawnSync(cli.path, [...extra, db, sql], { encoding: "utf8", maxBuffer: 1 << 28 });
    return { status: run.status, stdout: run.stdout ?? "", stderr: run.stderr ?? "" };
}

/** `PRAGMA integrity_check(limit)`: { ok, lines }. */
export function integrityCheck(cli, db, limit = 1000) {
    const run = shell(cli, db, `PRAGMA integrity_check(${limit});`);
    const lines = run.stdout.split("\n").filter(Boolean);
    if (run.stderr.trim()) lines.push(...run.stderr.trim().split("\n").map((l) => `(stderr) ${l}`));
    return { ok: run.status === 0 && lines.length === 1 && lines[0] === "ok", lines };
}

/**
 * `sqlite3 src ".recover" | sqlite3 dest`: salvages every row it can read into a
 * new database. Resolves with { ms, stderr }; rejects if either side fails.
 */
export function recover(cli, src, dest) {
    return new Promise((resolve, reject) => {
        const started = Date.now();
        const reader = spawn(cli.path, [src, ".recover"], { stdio: ["ignore", "pipe", "pipe"] });
        const writer = spawn(cli.path, [dest], { stdio: ["pipe", "ignore", "pipe"] });
        let stderr = "";
        const keep = (side) => (chunk) => {
            if (stderr.length < 20000) stderr += `[${side}] ${chunk}`;
        };
        reader.stderr.on("data", keep("recover"));
        writer.stderr.on("data", keep("load"));
        reader.stdout.pipe(writer.stdin);
        writer.stdin.on("error", () => {});
        const codes = {};
        const done = (side) => (code) => {
            codes[side] = code;
            if (codes.reader === undefined || codes.writer === undefined) return;
            if (codes.reader === 0 && codes.writer === 0) resolve({ ms: Date.now() - started, stderr });
            else reject(new Error(`.recover failed (recover exit ${codes.reader}, load exit ${codes.writer}): ${stderr.slice(0, 2000)}`));
        };
        reader.on("close", done("reader"));
        writer.on("close", done("writer"));
        reader.on("error", reject);
        writer.on("error", reject);
    });
}

/** A database opened with node:sqlite; read-only unless `write` is set. */
export function open(path, { write = false } = {}) {
    return new DatabaseSync(path, { readOnly: !write });
}

export function sqliteVersionOfNode() {
    const db = new DatabaseSync(":memory:");
    try {
        return db.prepare("select sqlite_version() v").get().v;
    } finally {
        db.close();
    }
}
