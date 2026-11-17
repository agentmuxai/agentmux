// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The tree column's width is saved at 100% of the tree's zoom: a drag of N screen pixels at 150% is N / 1.5. */

import { describe, expect, it, vi } from "vitest";
import { clampTreeWidth } from "./editor-zoom-parts";
import { treeResizeHandler } from "./editor-zoom-resize";
import type { EditorViewModel } from "./editor-model";

vi.mock("@/app/store/chrome-zoom", () => ({ chromeZoomAtom: () => 1 }));

function dragModel(stored: number) {
    let width = stored;
    const model = {
        treeWidthAtom: () => width,
        setTreeWidth: (w: number) => void (width = w),
        commitTreeWidth: vi.fn(async () => {}),
    };
    return { model: model as unknown as EditorViewModel, width: () => width, commit: model.commitTreeWidth };
}

function drag(handler: (e: MouseEvent) => void, from: number, to: number): void {
    const handle = document.createElement("div");
    document.body.append(handle);
    const down = new MouseEvent("mousedown", { clientX: from, bubbles: true, cancelable: true });
    Object.defineProperty(down, "currentTarget", { value: handle });
    handler(down);
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: to }));
    document.dispatchEvent(new MouseEvent("mouseup"));
}

describe("clampTreeWidth", () => {
    it("keeps the column at least 150 px wide as drawn: the stored least is smaller at a higher level", () => {
        expect(clampTreeWidth(10, 1)).toBe(150);
        expect(clampTreeWidth(10, 2)).toBe(75);
        expect(clampTreeWidth(10, 0.5)).toBe(150);
        // Never below the floor, whatever the level.
        expect(clampTreeWidth(10, 5)).toBe(50);
    });

    it("lets the divider move left at 200% in a pane too narrow for 150 stored pixels", () => {
        // The column is squeezed to 100 stored-equivalent pixels (200 drawn): a drag left of 50 screen px is 25 stored.
        expect(clampTreeWidth(100 - 25, 2)).toBe(75);
        expect(clampTreeWidth(100 - 25, 1)).toBe(150);
    });

    it("keeps a loaded width at or above the floor only, and the largest at 600", () => {
        expect(clampTreeWidth(80, Number.POSITIVE_INFINITY)).toBe(80);
        expect(clampTreeWidth(5, Number.POSITIVE_INFINITY)).toBe(50);
        expect(clampTreeWidth(900, 2)).toBe(600);
        expect(clampTreeWidth(Number.NaN, 2)).toBe(240);
    });
});

describe("treeResizeHandler", () => {
    it("passes the level the tree is at, so the model can let the column get narrow at a high level", () => {
        const set = vi.fn();
        const model = { treeWidthAtom: () => 100, setTreeWidth: set, commitTreeWidth: vi.fn(async () => {}) };
        drag(treeResizeHandler(model as unknown as EditorViewModel, () => 2), 300, 250);
        expect(set).toHaveBeenCalledWith(75, 2);
    });

    it("saves the width the tree has at 100%, whatever its zoom is while it is dragged", () => {
        const { model, width, commit } = dragModel(200);
        drag(treeResizeHandler(model, () => 1.5), 300, 360);
        // The column is not laid out in jsdom, so the drag starts from the stored width.
        expect(width()).toBe(240);
        expect(commit).toHaveBeenCalledTimes(1);
    });

    it("takes a screen pixel as a pixel at 100%", () => {
        const { model, width } = dragModel(200);
        drag(treeResizeHandler(model, () => 1), 300, 360);
        expect(width()).toBe(260);
    });
});
