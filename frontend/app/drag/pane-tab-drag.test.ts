// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane-tab (pill) drag on the drag session, from start to release to the
 * cross-window monitor's end. SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

import { afterEach, describe, expect, it } from "vitest";
import { beginDrag, endDrag, endReleasedSession, markEscaped, session } from "./drag-session";
import { isDraggedPaneTab, releasePaneTabDrag, startPaneTabDrag } from "./pane-tab-drag";

afterEach(() => endDrag("cancel"));

describe("pane-tab drag", () => {
    it("start begins a pane-tab session with its block, pane and window tab", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        expect(session()).toMatchObject({
            kind: "pane-tab",
            source: { blockId: "b1", nodeId: "pane-1", tabId: "tab-1" },
            escaped: false,
            released: false,
        });
        expect(isDraggedPaneTab("b1", "pane-1")).toBe(true);
    });

    it("only that pane's pill for the block counts as dragged", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        expect(isDraggedPaneTab("b1", "pane-2")).toBe(false);
        expect(isDraggedPaneTab("b2", "pane-1")).toBe(false);
    });

    it("the source's release keeps the session, escape and all, for the monitor", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        markEscaped();
        releasePaneTabDrag();
        expect(isDraggedPaneTab("b1", "pane-1")).toBe(false);
        expect(session()).toMatchObject({ kind: "pane-tab", released: true, escaped: true });
    });

    it("never releases another kind of drag", () => {
        beginDrag("tile", { nodeId: "n1" });
        releasePaneTabDrag();
        expect(session()?.released).toBe(false);
    });
});

describe("endReleasedSession (the cross-window monitor's dragend)", () => {
    it("ends a released session and hands it back, so escape can still be read", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        markEscaped();
        releasePaneTabDrag();
        const ended = endReleasedSession("dragend");
        expect(ended).toMatchObject({ kind: "pane-tab", escaped: true });
        expect(session()).toBeNull();
    });

    it("leaves a session its source hasn't released", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        expect(endReleasedSession("dragend")?.kind).toBe("pane-tab");
        expect(session()?.kind).toBe("pane-tab");
    });

    it("returns null when the tab bar already ended the drag", () => {
        expect(endReleasedSession("dragend")).toBeNull();
    });
});
