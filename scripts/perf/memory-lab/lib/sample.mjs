// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One memory sample of a running AgentMux instance, the way
// docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md §2 takes it:
//
// - OS numbers for every process under the app (private working set and
//   private commit, by role), from `memtree` (crates/procstats), so Windows,
//   macOS and Linux each report their own metric;
// - Chromium's memory-infra dump over CDP (`Tracing.requestMemoryDump`,
//   detailed; with `gc`, deterministic: Chromium forces a garbage collection
//   first), reduced to each process's allocator roots (v8, blink_gc,
//   partition_alloc, malloc, cc, gpu, ...);
// - per page: the JS heap, the DOM counters and a few AgentMux counts.
//
// Compare samples taken with `gc`; one without includes garbage.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { connectBrowser, connectPage, listTargets } from "../../../ui-screenshots/lib/cdp-client.mjs";

const hex = (v) => (typeof v === "string" ? parseInt(v, 16) : Number(v ?? 0));

/**
 * The last sample in a `memtree --json` file: the last line with a `procs`
 * array. On macOS memtree appends a `{"footprint": ...}` cross-check record
 * after the samples, which isn't one.
 */
export function lastSample(text) {
    const records = text
        .split(/\r?\n/)
        .filter(Boolean)
        .map((line) => JSON.parse(line))
        .filter((rec) => Array.isArray(rec.procs));
    if (records.length === 0) throw new Error("memtree wrote no sample");
    return records[records.length - 1];
}

/**
 * `memtree` labels a process by its command line's `--type=`; on Linux a
 * renderer or the GPU process forked from the zygote keeps the zygote's, and
 * reads "zygote-forked". Chromium itself says what each process is
 * (`SystemInfo.getProcessInfo`: browser, renderer, gpu, utility, ...), so a
 * forked one takes its type from there.
 */
export function withChromiumRoles(sample, processInfo) {
    const type = new Map(processInfo.map((p) => [p.id, p.type]));
    const role = (t) => (t === "gpu" ? "gpu-process" : t === "browser" ? "main" : t);
    return { ...sample, procs: sample.procs.map((p) => (p.role === "zygote-forked" && type.has(p.pid) ? { ...p, role: role(type.get(p.pid)) } : p)) };
}

/**
 * OS numbers: one `memtree` sample of the tree under `rootPid`; with the CDP
 * `port`, a zygote-forked process gets its real role (`withChromiumRoles`).
 */
export async function osSample(memtree, rootPid, port = null) {
    const dir = mkdtempSync(join(tmpdir(), "memory-lab-"));
    const file = join(dir, "memtree.jsonl");
    try {
        execFileSync(memtree, ["--pid", String(rootPid), "--samples", "1", "--json", file], { stdio: ["ignore", "ignore", "pipe"] });
        const sample = lastSample(readFileSync(file, "utf8"));
        if (port == null || !sample.procs.some((p) => p.role === "zygote-forked")) return sample;
        const browser = await connectBrowser(port, { timeoutMs: 15_000 });
        try {
            return withChromiumRoles(sample, (await browser.send("SystemInfo.getProcessInfo")).processInfo);
        } finally {
            await browser.close();
        }
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
}

/** Chromium's memory-infra dump of every process, each as `{ pid, name, labels, totals, alloc }`. */
export async function memoryInfraDump(browser, deterministic) {
    const events = [];
    const off = browser.on("Tracing.dataCollected", (p) => events.push(...p.value));
    const done = new Promise((r) => browser.on("Tracing.tracingComplete", r));
    await browser.send("Tracing.start", {
        transferMode: "ReportEvents",
        traceConfig: {
            recordMode: "recordAsMuchAsPossible",
            includedCategories: ["disabled-by-default-memory-infra", "__metadata"],
            excludedCategories: ["*"],
            memoryDumpConfig: { triggers: [] },
        },
    });
    await new Promise((r) => setTimeout(r, 500));
    const res = await browser.send("Tracing.requestMemoryDump", { deterministic, levelOfDetail: "detailed" }, { timeout: 120_000 });
    await new Promise((r) => setTimeout(r, 500));
    await browser.send("Tracing.end");
    await done;
    off();
    if (!res.success) console.warn("memory dump reported failure");
    return dumpedProcesses(events);
}

/** The processes in a memory-infra trace's events. */
export function dumpedProcesses(events) {
    const names = new Map();
    const labels = new Map();
    for (const e of events) {
        if (e.ph === "M" && e.name === "process_name") names.set(e.pid, e.args?.name);
        if (e.ph === "M" && e.name === "process_labels") labels.set(e.pid, e.args?.labels);
    }
    const procs = new Map();
    for (const e of events) {
        if (e.ph !== "v" || !e.args?.dumps) continue;
        const d = e.args.dumps;
        const alloc = {};
        for (const [path, node] of Object.entries(d.allocators ?? {})) {
            alloc[path] = { size: node.attrs?.size ? hex(node.attrs.size.value) : null };
        }
        const totals = {};
        for (const [k, v] of Object.entries(d.process_totals ?? {})) totals[k] = hex(v);
        procs.set(e.pid, { pid: e.pid, name: names.get(e.pid), labels: labels.get(e.pid), totals, alloc });
    }
    return [...procs.values()];
}

/**
 * A process's allocators reduced to the top-level roots (size of the root, else
 * the sum of its direct children), in bytes.
 */
export function allocatorRoots(alloc) {
    const paths = Object.keys(alloc);
    const roots = {};
    for (const r of new Set(paths.map((p) => p.split("/")[0]))) {
        let size = alloc[r]?.size;
        if (size == null) {
            size = 0;
            for (const p of paths) {
                const parts = p.split("/");
                if (parts.length === 2 && parts[0] === r && alloc[p].size != null) size += alloc[p].size;
            }
        }
        roots[r] = size;
    }
    return roots;
}

// Counts of what the page holds, for a sense of how much is mounted.
const PAGE_PROBE = `(() => {
  const q = (s) => document.querySelectorAll(s).length;
  const canv = [...document.querySelectorAll("canvas")];
  let canvasPx = 0;
  for (const c of canv) canvasPx += c.width * c.height;
  return {
    blocks: q("[data-blockid]"),
    agentPanes: q(".agent-input"),
    xterms: q(".xterm"),
    canvases: canv.length,
    canvasMPx: Math.round(canvasPx / 1e5) / 10,
    tabs: q(".tab"),
    elements: document.getElementsByTagName("*").length,
    imgs: q("img"),
  };
})()`;

async function pageStats(port, target) {
    const { session } = await connectPage(port, target.id);
    try {
        await session.send("Performance.enable");
        const heap = await session.send("Runtime.getHeapUsage");
        const dom = await session.send("Memory.getDOMCounters").catch(() => ({}));
        const perf = await session.send("Performance.getMetrics");
        const metrics = Object.fromEntries(perf.metrics.map((m) => [m.name, m.value]));
        let probe = null;
        try {
            probe = await session.evaluate(PAGE_PROBE, { timeout: 10_000 });
        } catch (e) {
            probe = { error: String(e.message) };
        }
        return {
            id: target.id,
            title: target.title,
            url: target.url.slice(0, 80),
            jsHeapUsed: heap.usedSize,
            jsHeapTotal: heap.totalSize,
            dom,
            nodes: metrics.Nodes,
            layoutObjects: metrics.LayoutObjects,
            listeners: metrics.JSEventListeners,
            probe,
        };
    } finally {
        await session.close();
    }
}

/**
 * Whether the browser process on CDP `port` is one of the tree under `rootPid`,
 * so the lab drives the instance it started and not another that answers there.
 */
export async function portBelongsTo(port, memtree, rootPid) {
    const browser = await connectBrowser(port, { timeoutMs: 15_000 });
    try {
        const { processInfo } = await browser.send("SystemInfo.getProcessInfo");
        const mine = new Set((await osSample(memtree, rootPid)).procs.map((p) => p.pid));
        return processInfo.some((p) => p.type === "browser" && mine.has(p.id));
    } finally {
        await browser.close();
    }
}

/**
 * The main window's page among a sample's pages: the one the app itself serves
 * without a `windowLabel` (pool and floating-pool windows have one; a browser
 * pane's page is on an outside URL).
 */
export function mainPage(pages) {
    const isApp = (p) => /^https?:\/\/(127\.0\.0\.1|localhost)[:/]/.test(p.url ?? "");
    return pages.find((p) => isApp(p) && !String(p.url).includes("windowLabel=")) ?? null;
}

/**
 * One sample: the OS numbers (`procs`, as `memtree` gives them, each CEF
 * process also with its allocator roots) and the pages.
 * @param {{ port: number, memtree: string, rootPid: number, label: string, gc?: boolean, pages?: boolean }} o
 */
export async function takeSample({ port, memtree, rootPid, label, gc = false, pages = true }) {
    const browser = await connectBrowser(port, { timeoutMs: 15_000 });
    let dumps = [];
    let pageList = [];
    try {
        // The dump first: with `gc` it collects garbage, and the pages' JS heap and DOM
        // counters are then read after that, not before.
        dumps = await memoryInfraDump(browser, gc);
        if (pages) {
            for (const t of (await listTargets(port)).filter((x) => x.type === "page")) {
                try {
                    pageList.push(await pageStats(port, t));
                } catch (e) {
                    pageList.push({ id: t.id, title: t.title, error: String(e.message) });
                }
            }
        }
    } finally {
        await browser.close();
    }
    // The OS numbers right after the dump, so both describe the same moment.
    const os = await osSample(memtree, rootPid, port);
    const byPid = new Map(dumps.map((d) => [d.pid, d]));
    const procs = os.procs.map((p) => {
        const d = byPid.get(p.pid);
        return d ? { ...p, footprint: d.totals?.private_footprint_bytes ?? null, roots: allocatorRoots(d.alloc) } : p;
    });
    return { label, time: new Date().toISOString(), gc, procs, pages: pageList };
}
