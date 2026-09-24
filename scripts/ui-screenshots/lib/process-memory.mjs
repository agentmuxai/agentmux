// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Resident memory of a set of processes, on Windows, macOS and Linux, for the
// Phase 0 bench's memory curve (renderer + browser/main process). CDP gives the
// process ids (SystemInfo.getProcessInfo) but not their memory; the OS does.
//
// Windows: one PowerShell call for all ids (WorkingSet64 = resident,
// PrivateMemorySize64 = private commit). macOS/Linux: one `ps` call (rss, KiB).

import { execFileSync } from "node:child_process";

/**
 * @param {number[]} pids
 * @returns {Map<number, { rssBytes: number, privateBytes: number | null }>}
 *          ids that no longer exist are simply absent.
 */
export function processMemory(pids, { platform = process.platform, exec = execFileSync } = {}) {
    const ids = [...new Set(pids.filter((p) => Number.isInteger(p) && p > 0))];
    if (ids.length === 0) return new Map();
    if (platform === "win32") {
        const script =
            `Get-Process -Id ${ids.join(",")} -ErrorAction SilentlyContinue | ` +
            `ForEach-Object { "$($_.Id) $($_.WorkingSet64) $($_.PrivateMemorySize64)" }`;
        const out = exec("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
            encoding: "utf8",
            stdio: ["ignore", "pipe", "ignore"],
        });
        return parseWindows(out);
    }
    let out = "";
    try {
        out = exec("ps", ["-o", "pid=,rss=", "-p", ids.join(",")], {
            encoding: "utf8",
            stdio: ["ignore", "pipe", "ignore"],
        });
    } catch (e) {
        // ps exits 1 when some ids are gone but still prints the rest.
        out = e.stdout ?? "";
    }
    return parsePs(out);
}

export function parseWindows(out) {
    const m = new Map();
    for (const line of String(out).split(/\r?\n/)) {
        const [pid, ws, priv] = line.trim().split(/\s+/).map(Number);
        if (Number.isInteger(pid) && Number.isFinite(ws))
            m.set(pid, { rssBytes: ws, privateBytes: Number.isFinite(priv) ? priv : null });
    }
    return m;
}

export function parsePs(out) {
    const m = new Map();
    for (const line of String(out).split(/\r?\n/)) {
        const [pid, rssKib] = line.trim().split(/\s+/).map(Number);
        if (Number.isInteger(pid) && Number.isFinite(rssKib))
            m.set(pid, { rssBytes: rssKib * 1024, privateBytes: null });
    }
    return m;
}
