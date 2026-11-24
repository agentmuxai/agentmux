// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resizedLeafIds } from "../lib/layoutResize";
import type { LayoutNode } from "../lib/types";

const leaf = (id: string) => ({ id }) as LayoutNode;
const group = (id: string, children: LayoutNode[]) => ({ id, children }) as LayoutNode;

describe("resizedLeafIds: the panes a border drag holds", () => {
    it("is each leaf of the two nodes the border sits between", () => {
        expect([...resizedLeafIds([leaf("a"), leaf("b")])]).toEqual(["a", "b"]);
    });

    it("walks a nested group down to its panes, and leaves the group's own id out", () => {
        const ids = resizedLeafIds([leaf("a"), group("g", [leaf("b"), group("h", [leaf("c"), leaf("d")])])]);
        expect([...ids].sort()).toEqual(["a", "b", "c", "d"]);
        expect(ids.has("g")).toBe(false);
    });

    it("leaves out a pane that is not under either node, so it keeps laying out live", () => {
        expect(resizedLeafIds([leaf("a"), leaf("b")]).has("far")).toBe(false);
    });

    it("is empty for nothing", () => {
        expect(resizedLeafIds([]).size).toBe(0);
    });
});
