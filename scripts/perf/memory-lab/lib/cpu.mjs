// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CPU time, and bytes written, of the app's processes over a lab run, per group. A process's CPU
// time is cumulative and dies with it, and a recycled renderer is a new
// process, so the total is built from the highest CPU time seen for every
// process (the ledger), not from the last sample alone.
//
// Limit: a process that starts and exits between two samples is never seen,
// and its CPU is missing. The lab samples at every mark, the recycle and the
// final, which covers the app's long-lived processes; short-lived helpers are
// not what these numbers are about.

import { GROUPS, groupOf } from "./summary.mjs";

export class CpuLedger {
    #seen = new Map();

    reset() {
        this.#seen.clear();
    }

    /** Takes the `cpu_ns` of every app process of one memtree sample. */
    note(procs) {
        for (const p of procs ?? []) {
            if (!p.app || (p.cpu_ns == null && p.write_bytes == null)) continue;
            const key = `${p.pid}|${p.name}`;
            const before = this.#seen.get(key) ?? { ns: 0, written: 0, group: groupOf(p) };
            this.#seen.set(key, {
                ns: Math.max(before.ns, p.cpu_ns ?? 0),
                written: Math.max(before.written, p.write_bytes ?? 0),
                group: before.group,
            });
        }
    }

    /** Bytes written so far, per group and `app`, counting every process seen. */
    writtenTotals() {
        const out = Object.fromEntries([...GROUPS, "app"].map((g) => [g, 0]));
        for (const { written, group } of this.#seen.values()) {
            out[group] += written;
            out.app += written;
        }
        return out;
    }

    /** CPU seconds so far, per group and `app` (all of them), counting every process seen. */
    totals() {
        const out = Object.fromEntries([...GROUPS, "app"].map((g) => [g, 0]));
        for (const { ns, group } of this.#seen.values()) {
            out[group] += ns / 1e9;
            out.app += ns / 1e9;
        }
        return out;
    }
}

/** What was written between two `writtenTotals()`, in megabytes (MiB) to a tenth. */
export function writtenBetween(start, end) {
    return Object.fromEntries(Object.keys(end).map((g) => [g, Math.round((Math.max(0, end[g] - (start[g] ?? 0)) / 1048576) * 10) / 10]));
}

/** What was spent between two `totals()`, in seconds to a hundredth. */
export function cpuBetween(start, end) {
    return Object.fromEntries(Object.keys(end).map((g) => [g, Math.round(Math.max(0, end[g] - (start[g] ?? 0)) * 100) / 100]));
}
