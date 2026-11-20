// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What is alive in an allocator, beside what the allocator holds from the system.
//
// A memory-infra dump gives an allocator two figures: `size`, the memory it holds, and
// `allocated_objects_size`, the objects alive in it. A heap that frees a page only when the whole
// page is empty can hold far more than is alive in it after a run that allocated a lot, so the two
// read apart: "more is alive" is something kept, "more is held, the same is alive" is churn.
// docs/reports/REPORT_NIGHTLY_UI_MEMORY_RISE_AND_FALL_2026_11_20.md is the rise that could not be
// read for want of the second figure.

import { median } from "./summary.mjs";

const MB = 1048576;

/**
 * What is alive in each allocator root, in bytes, from the allocators `dumpedProcesses` read
 * (`{ path: { size, live } }`). Summed over the highest nodes of a root that report it: a node
 * under one that reports is already counted in it. A root with no such node but with an
 * `<root>/allocated_objects` node (malloc, partition_alloc) gets that node's size. A root with
 * neither is left out: not known is not zero.
 */
export function allocatorLive(alloc) {
    const reports = (path) => alloc[path]?.live != null;
    const live = {};
    for (const path of Object.keys(alloc)) {
        if (!reports(path)) continue;
        const parts = path.split("/");
        if (parts.some((_, i) => i > 0 && reports(parts.slice(0, i).join("/")))) continue;
        live[parts[0]] = (live[parts[0]] ?? 0) + alloc[path].live;
    }
    for (const path of Object.keys(alloc)) {
        const parts = path.split("/");
        if (parts.length === 2 && parts[1] === "allocated_objects" && live[parts[0]] === undefined && alloc[path].size != null) live[parts[0]] = alloc[path].size;
    }
    return live;
}

/** The main window's renderer in a sample: the renderer with the most private memory that has a dump. Null without one. */
export function mainRenderer(sample) {
    const renderers = (sample?.procs ?? []).filter((p) => p.role === "renderer" && p.roots);
    return renderers.sort((a, b) => (b.private ?? 0) - (a.private ?? 0))[0] ?? null;
}

const mb = (bytes) => (bytes / MB).toFixed(1);
const signed = (bytes) => `${bytes >= 0 ? "+" : ""}${mb(bytes)}`;
const withLive = (held, live, fmt = mb) => (live == null ? fmt(held) : `${fmt(held)} (${fmt(live)})`);

/** The `summary.md` line for the main window renderer's allocators of 1 MB or more, largest first. Null without a dump. */
export function allocatorsLine(sample) {
    const renderer = mainRenderer(sample);
    if (!renderer) return null;
    const parts = Object.entries(renderer.roots)
        .filter(([, held]) => held >= MB)
        .sort((a, b) => b[1] - a[1])
        .map(([root, held]) => `${root} ${withLive(held, renderer.rootsLive?.[root])}`);
    if (parts.length === 0) return null;
    return `Main window renderer by allocator, MB held from the system (alive in it, where the dump says): ${parts.join(", ")}.`;
}

/**
 * The main window renderer's allocators of two sides, each the median of its runs' finals:
 * `[{ root, base: { held, live }, head: { held, live } }]`, largest first, for the roots that
 * reach 1 MB on a side. A side whose runs have no dump has null figures.
 */
export function compareAllocators(base, head) {
    const side = (runs) => {
        const renderers = runs.map((r) => mainRenderer(r.final)).filter(Boolean);
        return (root) => ({
            held: median(renderers.map((p) => p.roots[root])),
            live: median(renderers.map((p) => p.rootsLive?.[root])),
        });
    };
    const [a, b] = [side(base), side(head)];
    const roots = new Set([...base, ...head].flatMap((r) => Object.keys(mainRenderer(r.final)?.roots ?? {})));
    return [...roots]
        .map((root) => ({ root, base: a(root), head: b(root) }))
        .filter((row) => Math.max(row.base.held ?? 0, row.head.held ?? 0) >= MB)
        .sort((x, y) => Math.max(y.base.held ?? 0, y.head.held ?? 0) - Math.max(x.base.held ?? 0, x.head.held ?? 0));
}

/** `compareAllocators` as the comparison's Markdown lines; none when neither side has a dump. */
export function allocatorsTable(rows, baseLabel, headLabel) {
    if (rows.length === 0) return [];
    const cell = (s) => (s.held == null ? "n/a" : withLive(s.held, s.live));
    const change = (row) => {
        if (row.base.held == null || row.head.held == null) return "n/a";
        const live = row.base.live == null || row.head.live == null ? null : row.head.live - row.base.live;
        return withLive(row.head.held - row.base.held, live, signed);
    };
    return [
        "Main window renderer by allocator (report only), median MB held from the system, and in brackets what is alive in it, where the build's dump says. More alive is something kept; more held with the same alive is allocation churn.",
        "",
        `| allocator | ${baseLabel} | ${headLabel} | change |`,
        "|---|---:|---:|---:|",
        ...rows.map((row) => `| ${row.root} | ${cell(row.base)} | ${cell(row.head)} | ${change(row)} |`),
    ];
}
