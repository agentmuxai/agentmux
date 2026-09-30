// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One answer to "what is being dragged" per renderer, and who may end it.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { beginDrag, endDrag, isUnderway, markEscaped, markReleased, onSessionEnded, session } from "./drag-session";
import { isPaneTabSource, isTabSource, isTileSource, paneTabItemType, tabItemType, tileItemType } from "./drag-types";

afterEach(() => {
    endDrag("cancel");
});

describe("drag session", () => {
    it("begins with a fresh id, its source and payload, and ends with a reason", () => {
        const ended = vi.fn();
        const off = onSessionEnded(ended);
        const s = beginDrag("pane-tab", { blockId: "b1", tabId: "t1" }, { paneSize: { width: 640, height: 400 } });
        expect(session()).toBe(s);
        expect(s.kind).toBe("pane-tab");
        expect(s.dragId).toMatch(/\S{8,}/);
        expect(s.source).toEqual({ blockId: "b1", tabId: "t1" });
        expect(s.payload?.paneSize).toEqual({ width: 640, height: 400 });
        expect(s.escaped).toBe(false);
        expect(s.released).toBe(false);
        endDrag("drop");
        expect(session()).toBeNull();
        expect(ended).toHaveBeenCalledWith({ session: s, reason: "drop" });
        off();
    });

    it("mints a new id per drag", () => {
        const a = beginDrag("tile", { nodeId: "n1" });
        endDrag("drop");
        const b = beginDrag("tile", { nodeId: "n1" });
        expect(b.dragId).not.toBe(a.dragId);
    });

    it("a source-side release only marks the session; the cross-window monitor still sees it", () => {
        const s = beginDrag("tile", { nodeId: "n1" });
        markReleased();
        expect(session()?.dragId).toBe(s.dragId);
        expect(session()?.released).toBe(true);
    });

    it("a drag is underway from its start until its source releases it, and only for its own kind", () => {
        expect(isUnderway("tile")).toBe(false);
        beginDrag("tile", { nodeId: "n1" });
        expect(isUnderway("tile")).toBe(true);
        expect(isUnderway("pane-tab")).toBe(false);
        markReleased();
        expect(isUnderway("tile")).toBe(false);
    });

    it("escape is recorded on the session", () => {
        beginDrag("window-tab", { tabId: "t1" });
        markEscaped();
        expect(session()?.escaped).toBe(true);
    });

    it("ending a drag that is no longer current does nothing", () => {
        const ended = vi.fn();
        const off = onSessionEnded(ended);
        const old = beginDrag("tile", { nodeId: "n1" });
        endDrag("drop");
        const current = beginDrag("tile", { nodeId: "n2" });
        endDrag("dragend", old.dragId);
        expect(session()).toBe(current);
        expect(ended).toHaveBeenCalledTimes(1);
        off();
    });

    it("a new drag replaces a stranded one, which ends as cancelled", () => {
        const ended = vi.fn();
        const off = onSessionEnded(ended);
        const stranded = beginDrag("tile", { nodeId: "n1" });
        beginDrag("pane-tab", { blockId: "b1" });
        expect(ended).toHaveBeenCalledWith({ session: stranded, reason: "cancel" });
        off();
    });

    it("ending with no session is a no-op", () => {
        const ended = vi.fn();
        const off = onSessionEnded(ended);
        endDrag("dragend");
        expect(ended).not.toHaveBeenCalled();
        off();
    });

    it("a listener can unsubscribe", () => {
        const ended = vi.fn();
        onSessionEnded(ended)();
        beginDrag("tile", { nodeId: "n1" });
        endDrag("drop");
        expect(ended).not.toHaveBeenCalled();
    });
});

describe("drag types", () => {
    it("recognise each source by its tag", () => {
        expect(isTileSource({ type: tileItemType })).toBe(true);
        expect(isTabSource({ type: tabItemType })).toBe(true);
        expect(isPaneTabSource({ type: paneTabItemType })).toBe(true);
        expect(isTileSource({ type: tabItemType })).toBe(false);
        expect(isTabSource(undefined)).toBe(false);
    });
});
