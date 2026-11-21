// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The status bar's panels open upward from the bar. At a large chrome zoom a tall one (the instance panel with
 * several windows, the host panel) is taller than the window, and without `fitHeight` its top rows sit above the
 * window's edge, out of reach. `fitHeight` caps the panel at the room the window leaves, in screen pixels over the
 * zoom, and scrolls inside it (the cap itself is tested in element/anchored-popover.test.tsx). jsdom has no layout,
 * so this pins that every panel asks for it.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const PANELS = [
    "BackendStatus",
    "CpuCoresPopover",
    "DiskVolumesPopover",
    "GpuPanel",
    "HistoryUsagePopover",
    "HostPopover",
    "InstancePanel",
    "LowDiskNotice",
    "RestartToUpdatePopover",
    "SummaryPopover",
    "TokenBreakdownPopover",
];

/** The opening tag of the file's AnchoredPopover, props and all. */
function openingTag(file: string): string {
    const src = readFileSync(join(__dirname, `${file}.tsx`), "utf8");
    const start = src.indexOf("<AnchoredPopover");
    expect(start, `${file} renders an AnchoredPopover`).toBeGreaterThan(-1);
    return src.slice(start, src.indexOf(">\n", start));
}

describe("status bar panels fit the window at any chrome zoom", () => {
    it.each(PANELS)("%s caps its height at the room the window leaves", (file) => {
        expect(openingTag(file)).toMatch(/^\s*fitHeight$/m);
    });

    it("every panel that opens upward from the bar is listed", () => {
        const upward = readdirSync(__dirname)
            .filter((f) => f.endsWith(".tsx") && !f.includes(".test."))
            .map((f) => f.replace(/\.tsx$/, ""))
            .filter((f) => {
                const src = readFileSync(join(__dirname, `${f}.tsx`), "utf8");
                const start = src.indexOf("<AnchoredPopover");
                if (start < 0) return false;
                const tag = src.slice(start, src.indexOf(">\n", start));
                return /placement="top-(start|end)"/.test(tag);
            });
        expect(upward.sort()).toEqual([...PANELS].sort());
    });
});
