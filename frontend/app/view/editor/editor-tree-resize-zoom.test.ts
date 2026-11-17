// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The tree column's width is saved at 100% of the tree's zoom: a drag of N screen pixels at 150% is N / 1.5. */

import { describe, expect, it, vi } from "vitest";
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

describe("treeResizeHandler", () => {
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
