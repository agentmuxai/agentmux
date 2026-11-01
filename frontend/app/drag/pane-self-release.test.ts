// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A whole-pane drag released over the pane itself tears it off (pane-self-release.ts); any other release is
// decided as before.

import { describe, expect, it } from "vitest";
import { isSelfRelease, pointerOverOwnPane, selfReleasePayload } from "./pane-self-release";

const rect = { top: 100, left: 50, width: 400, height: 300 };
const tile = { kind: "tile", node: { id: "n1" }, sourceTabId: "t1" };
const release = (over: Partial<Parameters<typeof selfReleasePayload>[0]> = {}) =>
    selfReleasePayload({ payload: tile, nodeId: "n1", innermost: true, own: rect, x: 200, y: 200, ...over });

describe("pointerOverOwnPane", () => {
    it("is inside the rect, edges included, and not outside it or without one", () => {
        expect(pointerOverOwnPane(rect, 200, 200)).toBe(true);
        expect(pointerOverOwnPane(rect, 50, 100)).toBe(true);
        expect(pointerOverOwnPane(rect, 450, 400)).toBe(true);
        expect(pointerOverOwnPane(rect, 49, 200)).toBe(false);
        expect(pointerOverOwnPane(rect, 200, 401)).toBe(false);
        expect(pointerOverOwnPane(undefined, 200, 200)).toBe(false);
    });
});

describe("selfReleasePayload", () => {
    it("marks the pane's own drag when the pointer is over the pane, keeping the rest of the payload", () => {
        expect(release()).toEqual({ ...tile, selfRelease: true });
    });

    it("leaves a release over another pane or a gap to the layout's own handling", () => {
        expect(release({ x: 10, y: 10 })).toBeNull();
    });

    it("leaves a release an inner pane's overlay took", () => {
        expect(release({ innermost: false })).toBeNull();
    });

    it("is not another tab's pane (its rect is not in this tree) nor a pane-tab or window-tab drag", () => {
        expect(release({ own: undefined })).toBeNull();
        expect(release({ nodeId: "n2" })).toBeNull();
        expect(release({ payload: { kind: "pane-tab", node: { id: "n1" } } })).toBeNull();
        expect(release({ payload: { kind: "tab" } })).toBeNull();
        expect(release({ payload: null })).toBeNull();
        expect(release({ nodeId: undefined })).toBeNull();
    });
});

describe("isSelfRelease", () => {
    it("is true only for a marked tile payload", () => {
        expect(isSelfRelease({ ...tile, selfRelease: true })).toBe(true);
        expect(isSelfRelease(tile)).toBe(false);
        expect(isSelfRelease({ kind: "pane-tab", selfRelease: true })).toBe(false);
        expect(isSelfRelease(null)).toBe(false);
    });
});
