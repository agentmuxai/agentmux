// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The writer behind `--disk-load` (see diskload.mjs): node diskload-worker.mjs <dir> <MB/s|Infinity> <parent pid>.
// Writes 8 MiB chunks of incompressible data to 512 MiB files in <dir>, keeping at most two on disk, and syncs each
// chunk so the volume sees the writes rather than the page cache. Prints "written <bytes>" about once a second.
// Stops writing (and idles) once it has written <cap> bytes. Exits when its parent is gone.

import { closeSync, fsyncSync, openSync, rmSync, writeSync } from "node:fs";
import { randomFillSync } from "node:crypto";
import { join } from "node:path";

import { sleepNeededMs } from "./diskload.mjs";

const [dir, rate, parentPid, cap] = process.argv.slice(2);
const capBytes = Number(cap);
const mbps = rate === "Infinity" ? Infinity : Number(rate);
const CHUNK = 8 * 1048576;
const FILE_BYTES = 512 * 1048576;
const chunk = randomFillSync(Buffer.alloc(CHUNK));

const started = Date.now();
let written = 0;
let lastReport = 0;
let fileNo = 0;
let fd = null;
let inFile = 0;

// A lab that died without stopping the writer (killed, crashed) leaves its files: the writer removes them as it goes.
const leave = () => {
    if (fd !== null) closeSync(fd);
    rmSync(dir, { recursive: true, force: true });
    process.exit(0);
};

const parentAlive = () => {
    try {
        process.kill(Number(parentPid), 0);
        return true;
    } catch {
        return false;
    }
};

for (;;) {
    if (written >= capBytes) {
        if (fd !== null) {
            closeSync(fd);
            fd = null;
        }
        console.log(`written ${written}`);
        console.log("capped");
        if (!parentAlive()) leave();
        await new Promise((r) => setTimeout(r, 1000));
        continue;
    }
    if (fd === null) {
        fd = openSync(join(dir, `load-${fileNo}.bin`), "w");
        inFile = 0;
    }
    writeSync(fd, chunk);
    fsyncSync(fd);
    written += CHUNK;
    inFile += CHUNK;
    if (inFile >= FILE_BYTES) {
        closeSync(fd);
        fd = null;
        if (fileNo >= 1) rmSync(join(dir, `load-${fileNo - 1}.bin`), { force: true });
        fileNo++;
    }
    const now = Date.now();
    if (now - lastReport >= 1000) {
        console.log(`written ${written}`);
        lastReport = now;
        if (!parentAlive()) leave();
    }
    const wait = sleepNeededMs(written, now - started, mbps);
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
}
