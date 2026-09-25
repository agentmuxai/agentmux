// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { registerContextMenuRegion, resolveContextMenuRegion } from "./context-menu-region";

function tree() {
    const frame = document.createElement("div");
    const outer = document.createElement("div");
    const inner = document.createElement("div");
    const leaf = document.createElement("span");
    const sibling = document.createElement("div");
    frame.append(outer, sibling);
    outer.append(inner);
    inner.append(leaf);
    return { frame, outer, inner, leaf, sibling };
}

describe("context-menu regions", () => {
    it("resolves a region registered on an ancestor of the target", () => {
        const { outer, leaf } = tree();
        const region = { omit: ["split" as const] };
        registerContextMenuRegion(outer, region);
        expect(resolveContextMenuRegion(leaf)).toBe(region);
    });

    it("resolves a region registered on the target itself", () => {
        const { leaf } = tree();
        const region = {};
        registerContextMenuRegion(leaf, region);
        expect(resolveContextMenuRegion(leaf)).toBe(region);
    });

    it("nearest region wins and regions do not merge", () => {
        const { outer, inner, leaf } = tree();
        const far = { omit: ["split" as const] };
        const near = { omit: ["close" as const] };
        registerContextMenuRegion(outer, far);
        registerContextMenuRegion(inner, near);
        expect(resolveContextMenuRegion(leaf)).toBe(near);
    });

    it("returns null outside any region", () => {
        const { outer, sibling } = tree();
        registerContextMenuRegion(outer, {});
        expect(resolveContextMenuRegion(sibling)).toBeNull();
    });

    it("stops at the boundary and never consults it", () => {
        const { frame, outer, leaf } = tree();
        registerContextMenuRegion(frame, { omit: ["split"] });
        expect(resolveContextMenuRegion(leaf, frame)).toBeNull();
        registerContextMenuRegion(outer, {});
        expect(resolveContextMenuRegion(leaf, frame)).not.toBeNull();
    });

    it("unregister removes the region", () => {
        const { outer, leaf } = tree();
        const unregister = registerContextMenuRegion(outer, {});
        unregister();
        expect(resolveContextMenuRegion(leaf)).toBeNull();
    });

    it("a stale unregister does not remove a newer registration on the same element", () => {
        const { outer, leaf } = tree();
        const unregisterOld = registerContextMenuRegion(outer, { omit: ["split"] });
        const newer = { omit: ["close" as const] };
        registerContextMenuRegion(outer, newer);
        unregisterOld();
        expect(resolveContextMenuRegion(leaf)).toBe(newer);
    });

    it("tolerates a null/undefined/non-element target", () => {
        expect(resolveContextMenuRegion(null)).toBeNull();
        expect(resolveContextMenuRegion(undefined)).toBeNull();
        expect(resolveContextMenuRegion(window)).toBeNull();
    });
});
