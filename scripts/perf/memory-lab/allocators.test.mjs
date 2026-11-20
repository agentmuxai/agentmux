// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { allocatorLive, allocatorsLine, allocatorsTable, compareAllocators, mainRenderer } from "./lib/allocators.mjs";
import { dumpedProcesses } from "./lib/sample.mjs";

const MB = 1048576;
const hexOf = (n) => n.toString(16);

describe("what is alive in an allocator", () => {
    it("is read from a memory-infra trace beside the size the allocator holds", () => {
        const [p] = dumpedProcesses([
            {
                ph: "v",
                pid: 7,
                args: { dumps: { allocators: { "blink_gc/main/heap": { attrs: { size: { value: hexOf(600) }, allocated_objects_size: { value: hexOf(400) } } }, cc: { attrs: { size: { value: hexOf(9) } } } } } },
            },
        ]);
        expect(p.alloc["blink_gc/main/heap"]).toEqual({ size: 600, live: 400 });
        expect(p.alloc.cc).toEqual({ size: 9, live: null });
    });

    it("sums a root's highest reporting nodes and does not count a node under one of them again", () => {
        const live = allocatorLive({
            "blink_gc/main/heap": { size: 600, live: 400 },
            "blink_gc/main/heap/normal_page_space": { size: 500, live: 350 },
            "blink_gc/workers/heap": { size: 50, live: 20 },
            "v8/main/heap": { size: 120, live: 100 },
        });
        expect(live).toEqual({ blink_gc: 420, v8: 100 });
    });

    it("takes a root's allocated_objects node when no node of the root reports", () => {
        const live = allocatorLive({ malloc: { size: 130, live: null }, "malloc/allocated_objects": { size: 90, live: null }, "malloc/metadata": { size: 4, live: null } });
        expect(live).toEqual({ malloc: 90 });
    });

    it("leaves out a root the dump says nothing about: not known is not zero", () => {
        expect(allocatorLive({ cc: { size: 9, live: null }, "cc/tiles": { size: 5, live: null } })).toEqual({});
        expect(allocatorLive({})).toEqual({});
    });

    it("keeps the reported figure over the allocated_objects node when a root has both", () => {
        expect(allocatorLive({ "malloc/partitions": { size: 100, live: 70 }, "malloc/allocated_objects": { size: 60, live: null } })).toEqual({ malloc: 70 });
    });
});

const renderer = (privateMB, roots, rootsLive) => ({ role: "renderer", private: privateMB * MB, roots, ...(rootsLive ? { rootsLive } : {}) });
const run = (label, procs) => ({ label, final: { procs } });

describe("the main window renderer's allocators", () => {
    it("is the renderer with the most private memory that has a dump", () => {
        const main = renderer(700, { blink_gc: 400 * MB });
        expect(mainRenderer({ procs: [renderer(30, { blink_gc: 4 * MB }), main, { role: "renderer", private: 900 * MB }, { role: "gpu", private: 999 * MB, roots: {} }] })).toBe(main);
        expect(mainRenderer({ procs: [{ role: "renderer", private: 5 }] })).toBeNull();
        expect(mainRenderer(null)).toBeNull();
    });

    it("lists the allocators of 1 MB or more, largest first, with what is alive where the dump says", () => {
        const line = allocatorsLine({ procs: [renderer(700, { v8: 115 * MB, blink_gc: 620 * MB, skia: 1000, cc: 7 * MB }, { blink_gc: 410 * MB, v8: 100.5 * MB })] });
        expect(line).toBe("Main window renderer by allocator, MB held from the system (alive in it, where the dump says): blink_gc 620.0 (410.0), v8 115.0 (100.5), cc 7.0.");
    });

    it("has no line without a dump, or for a build whose result has no live sizes and no large allocator", () => {
        expect(allocatorsLine({ procs: [{ role: "renderer", private: 5 }] })).toBeNull();
        expect(allocatorsLine({ procs: [renderer(1, { skia: 10 })] })).toBeNull();
    });
});

describe("two sides' allocators compared", () => {
    const a = (heldMB, liveMB) => run("a", [renderer(700, { blink_gc: heldMB * MB, v8: 114 * MB, skia: 1000 }, { blink_gc: liveMB * MB })]);
    const b = (heldMB, liveMB) => run("b", [renderer(700, { blink_gc: heldMB * MB, v8: 115 * MB, skia: 1000 }, { blink_gc: liveMB * MB })]);

    it("takes the median of each side's runs, largest allocator first, and drops those under 1 MB", () => {
        const rows = compareAllocators([a(600, 410), a(620, 400), a(640, 405)], [b(430, 402), b(420, 398)]);
        expect(rows.map((r) => r.root)).toEqual(["blink_gc", "v8"]);
        expect(rows[0].base).toEqual({ held: 620 * MB, live: 405 * MB });
        expect(rows[0].head).toEqual({ held: 425 * MB, live: 400 * MB });
        expect(rows[1].base).toEqual({ held: 114 * MB, live: null });
    });

    it("prints what each side holds and has alive, and the change in both", () => {
        const text = allocatorsTable(compareAllocators([a(620, 405)], [b(425, 400)]), "a1", "b1").join("\n");
        expect(text).toContain("| allocator | a1 | b1 | change |");
        expect(text).toContain("| blink_gc | 620.0 (405.0) | 425.0 (400.0) | -195.0 (-5.0) |");
        expect(text).toContain("| v8 | 114.0 | 115.0 | +1.0 |");
    });

    it("says n/a for a side with no dump, shows no live change when one side lacks it, and prints nothing when neither side has a dump", () => {
        const old = run("old", [renderer(700, { blink_gc: 620 * MB })]);
        expect(allocatorsTable(compareAllocators([old], [b(425, 400)]), "old", "b1").join("\n")).toContain("| blink_gc | 620.0 | 425.0 (400.0) | -195.0 |");
        const none = run("none", [{ role: "renderer", private: 5 }]);
        expect(allocatorsTable(compareAllocators([none], [b(425, 400)]), "none", "b1").join("\n")).toContain("| blink_gc | n/a | 425.0 (400.0) | n/a |");
        expect(allocatorsTable(compareAllocators([none], [none]), "x", "y")).toEqual([]);
    });
});
