// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The guard of task #23943: a floating panel of the app's chrome (a menu, a popover, a tooltip, a dialog) follows
 * chrome zoom, and the one way to do that is `AnchoredPopover` (anchored-popover.tsx): an unzoomed shell placed in
 * real pixels with a zoomed body (docs/reports/REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md §4 measured why no other
 * shape works). A raw `<Portal>` or `createPortal` into the page skips all of that, so the ones that remain are
 * listed in `chrome-zoom-portals.json` with the reason (pane content that has its own zoom, or the surface a later
 * PR moves). A file may not hold more of them than the list says, and a file that no longer has one comes off it.
 * A new panel uses `AnchoredPopover`; a new entry needs a reason a reviewer accepts.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = join(__dirname, "..");
const LIST: Record<string, { max: number; why: string }> = JSON.parse(readFileSync(join(__dirname, "chrome-zoom-portals.json"), "utf8"));
/** The primitive itself is the one place a portal is the point. */
const PRIMITIVE = "element/anchored-popover.tsx";

function sources(dir: string, out: string[] = []): string[] {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) {
            if (name !== "node_modules") sources(p, out);
        } else if (/\.tsx$/.test(name) && !/\.(test|spec)\.tsx$/.test(name)) {
            out.push(p);
        }
    }
    return out;
}

/** The portals a file opens, not counting its comments. */
function portalsIn(src: string): number {
    const code = src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
    return (code.match(/<Portal\b/g) ?? []).length + (code.match(/\bcreatePortal\(/g) ?? []).length;
}

describe("raw portals bypass chrome zoom", () => {
    const found: Record<string, number> = {};
    for (const file of sources(ROOT)) {
        const rel = relative(ROOT, file).split("\\").join("/");
        if (rel === PRIMITIVE) continue;
        const n = portalsIn(readFileSync(file, "utf8"));
        if (n > 0) found[rel] = n;
    }

    it("has no portal outside AnchoredPopover that the list does not name", () => {
        const unlisted = Object.keys(found).filter((f) => !LIST[f]);
        expect(unlisted, "Use AnchoredPopover (it follows chrome zoom), or add the file to chrome-zoom-portals.json with the reason.").toEqual([]);
    });

    it("holds no file to more portals than the list allows", () => {
        const over = Object.entries(found).filter(([f, n]) => LIST[f] && n > LIST[f].max);
        expect(over).toEqual([]);
    });

    it("lists nothing a file no longer has, and gives every entry a reason", () => {
        const stale = Object.keys(LIST).filter((f) => !found[f]);
        expect(stale, "Remove the entry: the file no longer opens a raw portal.").toEqual([]);
        expect(Object.entries(LIST).filter(([, v]) => !v.why.trim())).toEqual([]);
    });
});
