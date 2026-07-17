// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { newLayoutNode, balanceNode } from "../lib/layoutNode";
import {
    minimizeNodeToggle,
    rebuildMinimizedSet,
    countExpandedLeaves,
    isNodeLocked,
    isEffectivelyMinimized,
    HeaderHeightPx,
    MinimizedRowSlotWidthPx,
} from "../lib/layoutMinimize";
import { computeMainAxisAllocation } from "../lib/layoutGeometry";
import { resizeNode, splitVertical } from "../lib/layoutTree";
import { FlexDirection, type LayoutNode } from "../lib/types";

// ── Minimal LayoutModel mock ─────────────────────────────────────────────────
// The display-mode toggle needs no geometry (no addlProps, no gap, no ratios).

function makeMockModel(rootNode: LayoutNode) {
    let minimizedSet = new Set<string>();
    return {
        treeState: { rootNode, pendingBackendActions: [] },
        minimizedNodeIds: {
            _set: (u: Set<string> | ((prev: Set<string>) => Set<string>)) => {
                minimizedSet = typeof u === "function" ? u(minimizedSet) : u;
            },
        },
        updateTree: () => {},
        localTreeStateAtom: { _set: () => {} },
        persistToBackend: () => {},
        getMinimizedSet: () => minimizedSet,
    };
}

const PANE_SIZE = 200;

//   root (Row)
//   ├── colA (Column) → paneA1, paneA2
//   └── colB (Column) → paneB
function buildTwoColumnLayout() {
    const paneA1 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA1" });
    const paneA2 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA2" });
    const colA = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneA1, paneA2]);
    const paneB = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneB" });
    const colB = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneB]);
    const root = newLayoutNode(FlexDirection.Row, PANE_SIZE, [colA, colB]);
    return { root, colA, paneA1, paneA2, colB, paneB };
}

// ── Display-mode toggle ──────────────────────────────────────────────────────

describe("minimize as a display mode", () => {
    it("sets only the flag — stored sizes of the pane AND its siblings never change", () => {
        const { root, paneA1, paneA2, colA, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root);

        minimizeNodeToggle(model as any, paneA1.id);

        expect(paneA1.minimized).toBe(true);
        expect(isNodeLocked(paneA1)).toBe(true);
        expect(paneA1.size).toBe(PANE_SIZE);
        expect(paneA2.size).toBe(PANE_SIZE);
        expect(colA.size).toBe(PANE_SIZE);
        expect(colB.size).toBe(PANE_SIZE);
        expect(paneA1.minimizedSize).toBeUndefined();
        expect(paneA1.minimizedLockedSize).toBeUndefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
    });

    it("restore clears the flag — original geometry is intact by construction", () => {
        const { root, paneA1 } = buildTwoColumnLayout();
        const model = makeMockModel(root);

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA1.id);

        expect(paneA1.minimized).toBeUndefined();
        expect(paneA1.size).toBe(PANE_SIZE);
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(false);
    });

    it("no structural surgery: minimizing every pane of a column leaves the tree shape untouched", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root);

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);

        // No dissolve: colA stays in the root Row with both children.
        expect(root.children).toHaveLength(2);
        expect(root.children![0].id).toBe(colA.id);
        expect(colA.children).toHaveLength(2);
        expect(colA.columnDissolve).toBeUndefined();
        expect(root._slipAnchor).toBeUndefined();
        // colA is now effectively minimized (renders as a chip stack).
        expect(isEffectivelyMinimized(colA)).toBe(true);
        expect(isEffectivelyMinimized(colB)).toBe(false);
    });

    it("toggle on a branch is a no-op", () => {
        const { root, colA } = buildTwoColumnLayout();
        const model = makeMockModel(root);
        minimizeNodeToggle(model as any, colA.id);
        expect(colA.minimized).toBeUndefined();
    });
});

// ── Last expanded pane guard ─────────────────────────────────────────────────

describe("last expanded pane guard", () => {
    it("refuses to minimize the last expanded pane; restore re-enables", () => {
        const { root, paneA1, paneA2, paneB } = buildTwoColumnLayout();
        const model = makeMockModel(root);

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);
        expect(countExpandedLeaves(root)).toBe(1);

        minimizeNodeToggle(model as any, paneB.id); // last expanded — no-op
        expect(paneB.minimized).toBeUndefined();

        minimizeNodeToggle(model as any, paneA1.id); // restore one
        minimizeNodeToggle(model as any, paneB.id); // now allowed
        expect(paneB.minimized).toBe(true);
        expect(paneA1.minimized).toBeUndefined();
    });
});

// ── Derived geometry (computeMainAxisAllocation) ─────────────────────────────

describe("derived chip geometry", () => {
    const size = (n: LayoutNode) => n.size;

    it("Column parent: minimized leaf gets exactly header height; sibling gets the rest", () => {
        const { colA, paneA1, paneA2 } = buildTwoColumnLayout();
        paneA1.minimized = true;
        const { px, pixelToSizeRatio } = computeMainAxisAllocation(colA.children!, false, 800, size);
        expect(px[0]).toBe(HeaderHeightPx);
        expect(px[1]).toBe(800 - HeaderHeightPx);
        // Ratio describes only the flex-distributed space.
        expect(pixelToSizeRatio).toBeCloseTo(PANE_SIZE / (800 - HeaderHeightPx));
    });

    it("Row parent: minimized leaf gets the fixed chip width", () => {
        const a = newLayoutNode(FlexDirection.Column, 10, undefined, { blockId: "a" });
        const b = newLayoutNode(FlexDirection.Column, 10, undefined, { blockId: "b" });
        a.minimized = true;
        const { px } = computeMainAxisAllocation([a, b], true, 1000, size);
        expect(px[0]).toBe(MinimizedRowSlotWidthPx);
        expect(px[1]).toBe(1000 - MinimizedRowSlotWidthPx);
    });

    it("fully-minimized column in a Row parent renders as one fixed-width chip stack", () => {
        const { root, colA, paneA1, paneA2 } = buildTwoColumnLayout();
        paneA1.minimized = true;
        paneA2.minimized = true;
        const { px } = computeMainAxisAllocation(root.children!, true, 1000, size);
        expect(px[0]).toBe(MinimizedRowSlotWidthPx); // colA: chip stack slot
        expect(px[1]).toBe(1000 - MinimizedRowSlotWidthPx); // colB absorbs the rest
    });

    it("fully-minimized column in a Column parent gets one header height per leaf", () => {
        const inner1 = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "i1" });
        const inner2 = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "i2" });
        inner1.minimized = true;
        inner2.minimized = true;
        const stack = newLayoutNode(FlexDirection.Column, 10, [inner1, inner2]);
        const other = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "o" });
        const { px } = computeMainAxisAllocation([stack, other], false, 600, size);
        expect(px[0]).toBe(2 * HeaderHeightPx);
        expect(px[1]).toBe(600 - 2 * HeaderHeightPx);
    });

    it("chips scale down proportionally when the container is too small for them", () => {
        const a = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "a" });
        const b = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "b" });
        a.minimized = true;
        b.minimized = true;
        const { px } = computeMainAxisAllocation([a, b], false, 40, size);
        expect(px[0]).toBeCloseTo(20);
        expect(px[1]).toBeCloseTo(20);
        expect(px[0] + px[1]).toBeCloseTo(40);
    });

    it("gap compensation: chip slot = header + gap so the inset inner box is exactly header-sized", () => {
        // innerRect renders each tile at calc(size - gapSizePx); the header has
        // a FIXED --header-height, so allocating raw HeaderHeightPx would clip
        // it by the gap (reagent P1 on PR #2197).
        const gap = 3;
        const { colA, paneA1 } = buildTwoColumnLayout();
        paneA1.minimized = true;
        const { px } = computeMainAxisAllocation(colA.children!, false, 800, size, gap);
        expect(px[0]).toBe(HeaderHeightPx + gap);
        expect(px[1]).toBe(800 - HeaderHeightPx - gap);

        // Row parent: chip width also carries the gap.
        const a = newLayoutNode(FlexDirection.Column, 10, undefined, { blockId: "a" });
        const b = newLayoutNode(FlexDirection.Column, 10, undefined, { blockId: "b" });
        a.minimized = true;
        const row = computeMainAxisAllocation([a, b], true, 1000, size, gap);
        expect(row.px[0]).toBe(MinimizedRowSlotWidthPx + gap);
    });

    it("no minimized children: identical to plain proportional split", () => {
        const a = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "a" });
        const b = newLayoutNode(FlexDirection.Row, 300, undefined, { blockId: "b" });
        const { px, pixelToSizeRatio } = computeMainAxisAllocation([a, b], false, 800, size);
        expect(px[0]).toBeCloseTo(200);
        expect(px[1]).toBeCloseTo(600);
        expect(pixelToSizeRatio).toBeCloseTo(400 / 800);
    });
});

// ── Reducer guards (minimized = locked) ──────────────────────────────────────

describe("minimized panes are untargetable by tree mutations", () => {
    it("resizeNode rejects the whole action when any target is minimized", () => {
        const { root, paneA1, paneA2 } = buildTwoColumnLayout();
        paneA1.minimized = true;

        resizeNode({ rootNode: root, pendingBackendActions: [] } as any, {
            type: "resize" as any,
            resizeOperations: [
                { nodeId: paneA1.id, size: 90 },
                { nodeId: paneA2.id, size: 10 },
            ],
        } as any);

        expect(paneA1.size).toBe(PANE_SIZE);
        expect(paneA2.size).toBe(PANE_SIZE);
    });

    it("splitVertical rejects a minimized target", () => {
        const { root, colA, paneA1 } = buildTwoColumnLayout();
        paneA1.minimized = true;
        const newNode = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "newPane" });
        splitVertical({ rootNode: root, pendingBackendActions: [] } as any, {
            type: "splitvertical" as any,
            targetNodeId: paneA1.id,
            newNode,
            position: "after",
        } as any);
        expect(colA.children).toHaveLength(2);
    });
});

// ── Legacy migration ─────────────────────────────────────────────────────────

describe("legacy state migration (rebuildMinimizedSet)", () => {
    it("migrates a size-squeezed legacy pane: size restored, flag set, markers cleared", () => {
        const { root, paneA1, paneA2 } = buildTwoColumnLayout();
        paneA1.minimizedSize = PANE_SIZE; // legacy: original size recorded...
        paneA1.size = HeaderHeightPx; // ...while size was squeezed
        paneA1.minimizedLockedSize = HeaderHeightPx;
        const model = makeMockModel(root);

        rebuildMinimizedSet(model as any);

        expect(paneA1.minimized).toBe(true);
        expect(paneA1.size).toBe(PANE_SIZE);
        expect(paneA1.minimizedSize).toBeUndefined();
        expect(paneA1.minimizedLockedSize).toBeUndefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
        expect(model.getMinimizedSet().has(paneA2.id)).toBe(false);
    });

    it("migrates slip/dissolve/anchor bookkeeping: markers cleared, sizes restored from originalRowSize", () => {
        const { root, colA, paneA1 } = buildTwoColumnLayout();
        paneA1.slipMinimize = { targetColumnId: colA.id, originalRowSize: 150, originalRowIndex: 0, targetWasLeaf: true };
        paneA1.size = 2.2; // slip-era squeezed size
        colA.columnDissolve = { targetColumnId: "x", originalRowSize: 120, originalRowIndex: 0, targetWasLeaf: false };
        colA.size = -0.0039; // the cascade-bug corruption: stolen-total gone negative
        root._slipAnchor = true;
        const model = makeMockModel(root);

        rebuildMinimizedSet(model as any);

        expect(paneA1.minimized).toBe(true);
        expect(paneA1.slipMinimize).toBeUndefined();
        expect(paneA1.size).toBe(150); // pre-slip size restored, not a permanent sliver
        expect(colA.columnDissolve).toBeUndefined();
        expect(colA.size).toBe(120); // pre-dissolve size restored — heals the negative
        expect(colA.minimized).toBeUndefined(); // branch never gets the flag
        expect(root._slipAnchor).toBeUndefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
    });

    it("rebuilds the set from existing flags without touching anything", () => {
        const { root, paneA2 } = buildTwoColumnLayout();
        paneA2.minimized = true;
        const model = makeMockModel(root);
        rebuildMinimizedSet(model as any);
        expect(model.getMinimizedSet()).toEqual(new Set([paneA2.id]));
        expect(paneA2.size).toBe(PANE_SIZE);
    });
});

// ── Legacy balance carve-outs stay inert but protective pre-migration ────────

describe("balanceNode legacy carve-outs", () => {
    it("still refuses to flip a legacy dissolved column's direction before migration runs", () => {
        const l1 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b1" });
        l1.minimizedSize = 200;
        const l2 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b2" });
        l2.minimizedSize = 200;
        const dissolved = newLayoutNode(FlexDirection.Column, 66, [l1, l2]);
        dissolved.columnDissolve = { targetColumnId: "host", originalRowSize: 200, originalRowIndex: 0, targetWasLeaf: true };
        const content = newLayoutNode(FlexDirection.Row, 300, undefined, { blockId: "b3" });
        const host = newLayoutNode(FlexDirection.Column, 400, [dissolved, content]);

        balanceNode(host);

        expect(host.children![0].flexDirection).toBe(FlexDirection.Column);
    });
});