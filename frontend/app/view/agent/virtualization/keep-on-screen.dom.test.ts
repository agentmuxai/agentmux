// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../../../agent-model/types";
import { captureDomAnchor, restoreOnScreen, successorIds } from "./keep-on-screen";

vi.mock("@/app/store/agent-pane-layout-store", () => ({
    dispatchIfRegistered: vi.fn(),
    snapshot: () => ({ scrollTop: 0, viewportPx: 300 }),
}));

/** A scroller whose top is at y = 100, holding rows drawn at the given tops (client px). */
function pane(rows: Record<string, { top: number; height: number }>) {
    const scroller = document.createElement("div");
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 100 } as DOMRect);
    for (const [id, r] of Object.entries(rows)) {
        const el = document.createElement("div");
        el.dataset.nodeId = id;
        vi.spyOn(el, "getBoundingClientRect").mockImplementation(() => ({ top: r.top, bottom: r.top + r.height, height: r.height }) as DOMRect);
        scroller.appendChild(el);
    }
    return scroller;
}

const run = (id: string, hiddenIds: string[]): DocumentNode => ({ type: "hidden_run", id, counts: [], failed: 0, open: false, hiddenIds }) as DocumentNode;

afterEach(() => vi.restoreAllMocks());

describe("keep-on-screen for rows outside the layout slice (SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.6)", () => {
    it("names where a row may be shown after a view change: itself, its run's first row, the run that holds that", () => {
        const nodes = [run("hidden-run:a", ["a", "b"])];
        expect(successorIds("b", nodes)).toEqual(["b", "hidden-run:a"]);
        expect(successorIds("hidden-run:a", nodes)).toEqual(["hidden-run:a", "a", "hidden-run:a"]);
        expect(successorIds("x", nodes)).toEqual(["x"]);
    });

    it("captures the first row whose bottom is below the top of the pane, and how far the pane's top is past it", () => {
        const scroller = pane({ a: { top: 20, height: 60 }, b: { top: 60, height: 80 }, c: { top: 140, height: 50 } });
        expect(captureDomAnchor(scroller, 1)).toEqual({ nodeId: "b", offsetPx: 40 });
        expect(captureDomAnchor(scroller, 2)).toEqual({ nodeId: "b", offsetPx: 20 });
        expect(captureDomAnchor(undefined, 1)).toBeNull();
    });

    it("scrolls by how far the anchor's row moved, and stops once it is back", () => {
        const scroller = pane({ b: { top: 160, height: 80 } }); // b was 40 px above the top; now 60 px below it
        scroller.scrollTop = 500;
        const o = { blockId: "p", rows: [], dom: { nodeId: "b", offsetPx: 40 }, latest: () => null, scroller };
        const frames: Array<() => void> = [];
        vi.stubGlobal("requestAnimationFrame", (fn: () => void) => frames.push(fn));
        vi.stubGlobal("cancelAnimationFrame", () => undefined);
        restoreOnScreen(o);
        frames.shift()!();
        expect(scroller.scrollTop).toBe(600); // b's top is at 60 past the pane's top; it belongs 40 above it: 100 px down
    });

    it("holds the run that took the row's place, with the offset kept within the run's height", () => {
        const scroller = pane({ "hidden-run:b": { top: 100, height: 24 } });
        scroller.scrollTop = 500;
        const frames: Array<() => void> = [];
        vi.stubGlobal("requestAnimationFrame", (fn: () => void) => frames.push(fn));
        vi.stubGlobal("cancelAnimationFrame", () => undefined);
        restoreOnScreen({ blockId: "p", rows: [], dom: { nodeId: "b", offsetPx: 90 }, latest: () => null, scroller, nodes: [run("hidden-run:b", ["b"])] });
        frames.shift()!();
        expect(scroller.scrollTop).toBe(524); // the run starts at the pane's top; 90 is kept to its 24 px
    });

    it("reads a drawn row a frame later, and stops when the pane follows the bottom", () => {
        const frames: Array<() => void> = [];
        vi.stubGlobal("requestAnimationFrame", (fn: () => void) => frames.push(fn));
        vi.stubGlobal("cancelAnimationFrame", () => undefined);
        const scroller = pane({ b: { top: 160, height: 80 } });
        scroller.scrollTop = 500;
        let following = false;
        restoreOnScreen({ blockId: "p", rows: [], dom: { nodeId: "b", offsetPx: 40 }, latest: () => null, scroller, following: () => following });
        // Nothing is read from the DOM in the task that changed the rows: the container's height is still the old one.
        expect(scroller.scrollTop).toBe(500);
        expect(frames).toHaveLength(1);
        frames.shift()!();
        expect(scroller.scrollTop).toBe(600);
        following = true;
        frames.shift()?.();
        expect(frames).toHaveLength(0);
    });
});
