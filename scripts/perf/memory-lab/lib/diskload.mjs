// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Background disk load for a lab run. The operator's real instances saw srv's transcript store held for
// seconds because other work (agents' builds) saturated the disk; a lab run on an idle disk never does
// (issue #5141). `--disk-load <MB/s>` (or `max`) runs a writer process that streams large files to the same
// volume as the lab's data folder while the agents work, so both sides of an A/B see the same load.

import { spawn } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The requested load as a rate in MiB/s: a positive number, or Infinity for `max`; null for none. Throws on anything else. */
export function parseDiskLoad(value) {
    if (value == null || value === "" || value === "0" || value === 0) return null;
    if (String(value).toLowerCase() === "max") return Infinity;
    const n = Number(value);
    if (!Number.isFinite(n) || n <= 0) throw new Error(`--disk-load needs a positive number of MB per second, or "max" (got "${value}")`);
    return n;
}

/** How long the writer should sleep to hold `mbps`, given what it has written and how long it has been going. 0 when it is behind. */
export function sleepNeededMs(bytesWritten, elapsedMs, mbps) {
    if (!Number.isFinite(mbps)) return 0;
    const dueAtMs = (bytesWritten / (mbps * 1048576)) * 1000;
    return Math.max(0, Math.round(dueAtMs - elapsedMs));
}

/** The most the writer will write in one run, so `max` on a fast disk does not wear it out: 100 s at 600 MB/s is 60 GB. */
export const DEFAULT_CAP_GB = 20;

const WORKER = join(dirname(fileURLToPath(import.meta.url)), "diskload-worker.mjs");

/**
 * Starts the writer in `dir`/diskload. Returns `stop()`, which ends it, deletes its files and reports what it
 * wrote: `{ rateMBps, writtenMB, seconds }`.
 */
export function startDiskLoad({ dir, mbps, capGB = DEFAULT_CAP_GB }) {
    const target = join(dir, "diskload");
    rmSync(target, { recursive: true, force: true });
    mkdirSync(target, { recursive: true });
    const child = spawn(process.execPath, [WORKER, target, String(mbps), String(process.pid), String(Math.round(capGB * 1073741824))], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
    const started = Date.now();
    let written = 0;
    let buffer = "";
    let capReached = false;
    // How the writer ended, if it ended on its own: a disk that fills or a write that fails kills it while the workload goes on.
    let exited = null;
    child.once("exit", (code, signal) => {
        exited = { code, signal, at: Date.now() };
    });
    child.once("error", (e) => {
        exited ??= { code: null, signal: null, error: String(e.message), at: Date.now() };
    });
    child.stdout.on("data", (d) => {
        buffer += d;
        for (let i; (i = buffer.indexOf("\n")) >= 0; ) {
            const line = buffer.slice(0, i);
            buffer = buffer.slice(i + 1);
            const m = /^written (\d+)$/.exec(line);
            if (m) written = Number(m[1]);
            if (line === "capped") capReached = true;
        }
    });
    let stopped = null;
    return {
        pid: child.pid,
        /** Why the writer died before it was stopped, or null while it runs (or if it was only stopped). */
        diedEarly() {
            return exited && !stopped ? `the disk-load writer exited on its own (code ${exited.code}, signal ${exited.signal}${exited.error ? `, ${exited.error}` : ""}) ${Math.round((exited.at - started) / 1000)} s in: the volume may be full, or a write failed` : null;
        },
        async stop() {
            if (stopped) return stopped;
            const diedEarly = this.diedEarly();
            if (!exited) {
                await new Promise((resolve) => {
                    child.once("exit", resolve);
                    child.kill();
                    setTimeout(resolve, 5000);
                });
            }
            rmSync(target, { recursive: true, force: true });
            stopped = { capGB, rateMBps: Number.isFinite(mbps) ? mbps : "max", writtenMB: Math.round((written / 1048576) * 10) / 10, seconds: Math.round((Date.now() - started) / 100) / 10, capReached, ...(diedEarly ? { diedEarly } : {}) };
            return stopped;
        },
    };
}
