// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a pending `createResource` read in a pane's view may suspend to.
 *
 * Every view reads resources (a refetch, a part that mounts later), and a read that is pending suspends
 * the nearest `<Suspense>`. A plain one swaps its children for its fallback, so a view under one loses its
 * size and every scroller in it loses its place (the Kanban drawer #47462, the transcript #47406, and the
 * fourteen views of #47479 that read resources with no boundary of their own).
 *
 * `PaneViewBoundary` is the one boundary: it leaves the view in the document. This test keeps it the only one:
 * no source file uses `Suspense` but it, and the block renders its one `PaneView` inside it. A new `Suspense`
 * over a view, or a `PaneView` rendered outside the boundary, fails here and says what to do.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND_ROOT = join(__dirname, "..", "..");
const BOUNDARY = "app/block/PaneViewBoundary.tsx";
const BLOCK = "app/block/block.tsx";

function collectSourceFiles(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) {
            if (entry === "node_modules" || entry === "test") continue;
            collectSourceFiles(full, out);
        } else if (/\.(ts|tsx)$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
            out.push(full);
        }
    }
    return out;
}

const rel = (file: string) => relative(FRONTEND_ROOT, file).split(sep).join("/");
const withoutComments = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");

/** Sources that import `Suspense` from solid-js or render a `<Suspense` element. */
function filesUsingSuspense(): string[] {
    const importsSuspense = /import\s*\{[^}]*\bSuspense\b[^}]*\}\s*from\s*["']solid-js["']/;
    const rendersSuspense = /<Suspense[\s>]/;
    return collectSourceFiles(FRONTEND_ROOT)
        .filter((file) => {
            const text = withoutComments(readFileSync(file, "utf8"));
            return importsSuspense.test(text) || rendersSuspense.test(text);
        })
        .map(rel)
        .sort();
}

describe("a pane view's Suspense boundary", () => {
    it("is the only Suspense in the frontend", () => {
        expect(
            filesUsingSuspense(),
            "A source file uses Suspense. A plain Suspense takes its children out of the document while a read in them " +
                "is pending, which blanks the view and resets its scrollers. Read the resource under PaneViewBoundary " +
                "(app/block/PaneViewBoundary.tsx), which every pane view already is, or read it as a signal."
        ).toEqual([BOUNDARY]);
    });

    it("holds the block's one PaneView", () => {
        const block = withoutComments(readFileSync(join(FRONTEND_ROOT, BLOCK), "utf8"));
        const views = [...block.matchAll(/<PaneView[\s/]/g)];
        expect(views, "the block renders its view in one place").toHaveLength(1);
        const open = block.indexOf("<PaneViewBoundary");
        const close = block.indexOf("</PaneViewBoundary>");
        expect(open, "the block uses PaneViewBoundary").toBeGreaterThan(-1);
        expect(close).toBeGreaterThan(open);
        const at = views[0].index ?? -1;
        expect(at > open && at < close, "PaneView is rendered inside PaneViewBoundary").toBe(true);
    });
});
