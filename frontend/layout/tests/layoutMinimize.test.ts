// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { newLayoutNode } from "../lib/layoutNode";
import {
    minimizeNodeToggle,
    rebuildMinimizedSet,
    enforceMinimizedLocks,
    isNodeLocked,
    HeaderHeightPx,
} from "../lib/layoutMinimize";
import { resizeNode, splitVertical } from "../lib/layoutTree";
import { balanceNode } from "../lib/layoutNode";
import { FlexDirection, type LayoutNode, type LayoutNodeAdditionalProps } from "../lib/types";

// ── Minimal LayoutModel mock ─────────────────────────────────────────────────

function makeMockModel(
    rootNode: LayoutNode,
    addlProps: Record<string, Partial<LayoutNodeAdditionalProps>> = {},
    gapSizePx = 0,
) {
    let minimizedSet = new Set<string>();
    const treeState = { rootNode, pendingBackendActions: [] };
    const model = {
        treeState,
        getter: (_sig: unknown) => addlProps as Record<string, LayoutNodeAdditionalProps>,
        additionalProps: {} as any,
        gapSizePx: () => gapSizePx,
        minimizedNodeIds: {
            // Real SignalAtom._set accepts both a value and an updater function.
            _set: (updaterOrValue: Set<string> | ((prev: Set<string>) => Set<string>)) => {
                minimizedSet = typeof updaterOrValue === "function"
                    ? updaterOrValue(minimizedSet)
                    : updaterOrValue;
            },
        },
        updateTree: () => {},
        localTreeStateAtom: { _set: () => {} },
        persistToBackend: () => {},
        getMinimizedSet: () => minimizedSet,
    };
    return model;
}

// Sizes must exceed HeaderHeightPx (33) so freedUnits > 0 and minimize doesn't no-op.
const PANE_SIZE = 200;

// ── Helper: build a 2-column layout ─────────────────────────────────────────
//
//   root (Row)
//   ├── colA (Column, size=200)
//   │   ├── paneA1 (leaf, size=200)
//   │   └── paneA2 (leaf, size=200)
//   └── colB (Column, size=200)
//       └── paneB  (leaf, size=200)

function buildTwoColumnLayout() {
    const paneA1 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA1" });
    const paneA2 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA2" });
    const colA   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneA1, paneA2]);

    const paneB  = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneB" });
    const colB   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneB]);

    const root   = newLayoutNode(FlexDirection.Row, PANE_SIZE, [colA, colB]);
    return { root, colA, paneA1, paneA2, colB, paneB };
}

// ── Normal column-collapse (no dissolve) ─────────────────────────────────────

describe("normal column collapse (partial — dissolve not triggered)", () => {
    it("collapses the pane to header height, gives freed space to sibling", () => {
        const { root, colA, paneA1, paneA2 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, paneA1.id);

        expect(paneA1.minimizedSize).toBe(PANE_SIZE);
        expect(paneA1.size).toBe(HeaderHeightPx);
        expect(paneA2.size).toBe(PANE_SIZE + (PANE_SIZE - HeaderHeightPx));
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
        // colA stays in root Row — paneA2 still expanded
        expect(root.children).toHaveLength(2);
    });

    it("restores pane to its original size", () => {
        const { root, colA, paneA1 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA1.id);

        expect(paneA1.minimizedSize).toBeUndefined();
        expect(paneA1.size).toBe(PANE_SIZE);
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(false);
        void root;
    });
});

// ── Column dissolve ───────────────────────────────────────────────────────────

describe("column dissolve — all panes minimized triggers dissolve into adjacent column", () => {
    it("dissolves colA into colB when both panes in colA are minimized", () => {
        const { root, colA, paneA1, paneA2, colB, paneB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        expect(root.children).toHaveLength(2); // no dissolve yet

        minimizeNodeToggle(model as any, paneA2.id); // triggers dissolve

        // Root Row now holds only colB
        expect(root.children).toHaveLength(1);
        expect(root.children![0].id).toBe(colB.id);
        expect(root._slipAnchor).toBe(true);

        // colA is now the first child of colB
        expect(colB.children![0].id).toBe(colA.id);
        expect(colA.columnDissolve).toBeDefined();
        expect(colA.columnDissolve!.targetColumnId).toBe(colB.id);
        expect(colA.columnDissolve!.originalRowSize).toBe(PANE_SIZE);
        expect(colA.columnDissolve!.originalRowIndex).toBe(0);

        // colB absorbed colA's Row-slot width
        expect(colB.size).toBe(PANE_SIZE * 2);

        // paneB is still in colB (after dissolved colA)
        expect(colB.children![1].id).toBe(paneB.id);

        // Both panes inside colA retain their minimizedSize
        expect(paneA1.minimizedSize).toBeDefined();
        expect(paneA2.minimizedSize).toBeDefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
        expect(model.getMinimizedSet().has(paneA2.id)).toBe(true);
    });

    it("colA occupies 2 × HeaderHeightPx flex-units inside colB after dissolve", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);

        expect(colA.size).toBe(2 * HeaderHeightPx);
        void colB;
    });

    // Regression: production calls balanceNode(root) on every geometry pass
    // (layoutGeometry.ts's updateTree), which this test suite otherwise never
    // exercises since makeMockModel's updateTree is a no-op stub. A dissolved
    // column is nested inside a same-direction Column sibling by design, so
    // balanceNode's direction-alternation rule used to flip colA's own
    // flexDirection to Row, laying its two minimized headers out side-by-side
    // ("narrow") instead of stacked ("short").
    it("keeps the dissolved column's own flexDirection stacked after balanceNode runs", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id); // triggers dissolve: colA nests inside colB

        expect(colB.children![0].id).toBe(colA.id);
        expect(colA.flexDirection).toBe(FlexDirection.Column);
        expect(colB.flexDirection).toBe(FlexDirection.Column);

        balanceNode(root);

        // colA is still nested first inside colB and still stacks vertically.
        expect(colB.children![0].id).toBe(colA.id);
        expect(colA.flexDirection).toBe(FlexDirection.Column);
    });
});

// ── Undissolve on restore ─────────────────────────────────────────────────────

describe("undissolve — clicking a pane in a dissolved column restores the column", () => {
    it("undissolves colA and restores the clicked pane in a single toggle", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);
        expect(root.children).toHaveLength(1);

        // One toggle on paneA2: undissolves colA AND restores paneA2
        minimizeNodeToggle(model as any, paneA2.id);

        // colA is back in the root Row at its original index
        expect(root.children).toHaveLength(2);
        expect(root.children![0].id).toBe(colA.id);
        expect(colA.columnDissolve).toBeUndefined();
        expect(root._slipAnchor).toBeUndefined();

        // colB width is restored
        expect(colB.size).toBe(PANE_SIZE);

        // paneA2 is restored
        expect(paneA2.minimizedSize).toBeUndefined();
        expect(model.getMinimizedSet().has(paneA2.id)).toBe(false);

        // paneA1 remains minimized inside colA
        expect(paneA1.minimizedSize).toBeDefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
    });

    it("undissolves via paneA1 as well", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);

        minimizeNodeToggle(model as any, paneA1.id);

        expect(root.children).toHaveLength(2);
        expect(colA.columnDissolve).toBeUndefined();
        expect(paneA1.minimizedSize).toBeUndefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(false);
        expect(paneA2.minimizedSize).toBeDefined();
        void colB;
    });
});

// ── rebuildMinimizedSet ───────────────────────────────────────────────────────

describe("rebuildMinimizedSet", () => {
    it("rebuilds minimizedNodeIds from minimizedSize fields after tree load", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);

        // Simulate fresh load: clear the set, then rebuild
        model.minimizedNodeIds._set(new Set<string>());
        rebuildMinimizedSet(model as any);

        expect(model.getMinimizedSet().has(paneA1.id)).toBe(true);
        expect(model.getMinimizedSet().has(paneA2.id)).toBe(true);
    });

    it("restores _slipAnchor on the owning Row for a dissolved column", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);

        root._slipAnchor = undefined; // simulate stale serialised state
        rebuildMinimizedSet(model as any);

        expect(root._slipAnchor).toBe(true);
    });
});

// ── Cascade dissolve — 3 columns → 2 → 1 ────────────────────────────────────

describe("cascade dissolve — multiple columns collapse into one", () => {
    it("colA dissolves into colB, then colB dissolves into colC when all minimized", () => {
        // root (Row)
        // ├── colA (Column) → paneA1, paneA2
        // ├── colB (Column) → paneB
        // └── colC (Column) → paneC
        const paneA1 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA1" });
        const paneA2 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA2" });
        const colA   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneA1, paneA2]);
        const paneB  = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneB" });
        const colB   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneB]);
        const paneC  = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneC" });
        const colC   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneC]);
        const root   = newLayoutNode(FlexDirection.Row, PANE_SIZE, [colA, colB, colC]);

        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
            [colC.id]: { pixelToSizeRatio: 1 },
        });

        // Minimize colA's panes → colA dissolves into colB
        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);
        expect(root.children).toHaveLength(2); // colA gone, root has colB + colC
        expect(colA.columnDissolve).toBeDefined();
        expect(colB.children![0].id).toBe(colA.id);

        // Minimize colB's pane (colB now has [colA-dissolved, paneB]) →
        // allCollapsed includes columnDissolve, so colB dissolves into colC
        minimizeNodeToggle(model as any, paneB.id);
        expect(root.children).toHaveLength(1); // only colC remains
        expect(root.children![0].id).toBe(colC.id);
        expect(colB.columnDissolve).toBeDefined();
        expect(colC.children![0].id).toBe(colB.id);

        // colB (inside colC) still contains colA-dissolved
        expect(colB.children!.some(c => c.id === colA.id)).toBe(true);

        // paneC can still be minimized (colB-dissolved is its sibling)
        minimizeNodeToggle(model as any, paneC.id);
        expect(paneC.minimizedSize).toBeDefined();
        expect(root.children).toHaveLength(1); // dissolve bails (no Row sibling) — correct
    });

    it("restores pane from 3-deep cascade (A→B→C → restore colA pane)", () => {
        // root (Row)
        // ├── colA (Column) → paneA1, paneA2
        // ├── colB (Column) → paneB
        // └── colC (Column) → paneC
        const paneA1 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA1" });
        const paneA2 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneA2" });
        const colA   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneA1, paneA2]);
        const paneB  = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneB" });
        const colB   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneB]);
        const paneC  = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "paneC" });
        const colC   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [paneC]);
        const root   = newLayoutNode(FlexDirection.Row, PANE_SIZE, [colA, colB, colC]);

        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
            [colC.id]: { pixelToSizeRatio: 1 },
        });

        // Step 1: Dissolve colA into colB (minimize all panes in colA)
        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id);
        expect(colA.columnDissolve).toBeDefined();
        expect(root.children).toHaveLength(2); // colB + colC remain

        // Step 2: Dissolve colB into colC (colB now has [colA-dissolved, paneB]; minimizing paneB collapses it)
        minimizeNodeToggle(model as any, paneB.id);
        expect(colB.columnDissolve).toBeDefined();
        expect(root.children).toHaveLength(1); // only colC remains
        expect(root.children![0].id).toBe(colC.id);

        // Step 3: Restore paneA1 from the deeply dissolved colA.
        // The recursive-undissolve-ancestors path must undissolve colB (outermost) then colA
        // (immediate parent) before restoring paneA1 — exercising the path added in commit 360dd82.
        minimizeNodeToggle(model as any, paneA1.id);

        // All three columns must be back in the root Row
        expect(root.children).toHaveLength(3);
        const rootIds = root.children!.map((c) => c.id);
        expect(rootIds).toContain(colA.id);
        expect(rootIds).toContain(colB.id);
        expect(rootIds).toContain(colC.id);

        // No column retains a columnDissolve marker
        expect(colA.columnDissolve).toBeUndefined();
        expect(colB.columnDissolve).toBeUndefined();
        expect(colC.columnDissolve).toBeUndefined();

        // Each column is restored to its original Row-slot size
        expect(colA.size).toBe(PANE_SIZE);
        expect(colB.size).toBe(PANE_SIZE);
        expect(colC.size).toBe(PANE_SIZE);

        // root Row no longer needs the slip anchor
        expect(root._slipAnchor).toBeUndefined();

        // paneA1 is restored (the pane we clicked)
        expect(paneA1.minimizedSize).toBeUndefined();
        expect(model.getMinimizedSet().has(paneA1.id)).toBe(false);

        // paneA2 remains minimized inside colA
        expect(paneA2.minimizedSize).toBeDefined();
        expect(model.getMinimizedSet().has(paneA2.id)).toBe(true);

        // paneB remains minimized inside colB
        expect(paneB.minimizedSize).toBeDefined();
        expect(model.getMinimizedSet().has(paneB.id)).toBe(true);
    });
});

// ── Bail case — no adjacent sibling ─────────────────────────────────────────

describe("dissolve bail cases", () => {
    it("does not dissolve when the column is the only child in the Row", () => {
        const pane1 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "p1" });
        const pane2 = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "p2" });
        const col   = newLayoutNode(FlexDirection.Column, PANE_SIZE, [pane1, pane2]);
        const root  = newLayoutNode(FlexDirection.Row, PANE_SIZE, [col]);

        const model = makeMockModel(root, { [col.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, pane1.id);
        minimizeNodeToggle(model as any, pane2.id);

        // col stays in root Row — dissolve bailed (no Row sibling)
        expect(root.children).toHaveLength(1);
        expect(col.columnDissolve).toBeUndefined();
        // Both panes are still individually minimized
        expect(pane1.minimizedSize).toBeDefined();
        expect(pane2.minimizedSize).toBeDefined();
    });
});

// ── Minimize lock (locked-state spec, 2026-07-16) ───────────────────────────

describe("minimize lock — minimized is a locked state", () => {
    it("records minimizedLockedSize on minimize and clears it on restore", () => {
        const { root, colA, paneA1 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, paneA1.id);
        expect(paneA1.minimizedLockedSize).toBe(HeaderHeightPx);
        expect(paneA1.size).toBe(HeaderHeightPx);
        expect(isNodeLocked(paneA1)).toBe(true);

        minimizeNodeToggle(model as any, paneA1.id);
        expect(paneA1.minimizedLockedSize).toBeUndefined();
        expect(isNodeLocked(paneA1)).toBe(false);
    });

    it("records minimizedLockedSize on the dissolved column and clears it on undissolve", () => {
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        const model = makeMockModel(root, {
            [colA.id]: { pixelToSizeRatio: 1 },
            [colB.id]: { pixelToSizeRatio: 1 },
        });

        minimizeNodeToggle(model as any, paneA1.id);
        minimizeNodeToggle(model as any, paneA2.id); // triggers dissolve
        expect(colA.minimizedLockedSize).toBe(colA.size);
        expect(isNodeLocked(colA)).toBe(true);

        minimizeNodeToggle(model as any, paneA2.id); // undissolves + restores
        expect(colA.minimizedLockedSize).toBeUndefined();
        expect(isNodeLocked(colA)).toBe(false);
    });

    it("resizeNode rejects the whole action when any target is locked (the reported bug)", () => {
        const { root, colA, paneA1, paneA2 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, paneA1.id);
        const lockedSize = paneA1.size;
        const siblingSize = paneA2.size;

        resizeNode({ rootNode: root, pendingBackendActions: [] } as any, {
            type: "resize" as any,
            resizeOperations: [
                { nodeId: paneA1.id, size: 90 },
                { nodeId: paneA2.id, size: 10 },
            ],
        } as any);

        // Atomic reject: neither side of the pair applied.
        expect(paneA1.size).toBe(lockedSize);
        expect(paneA2.size).toBe(siblingSize);
    });

    it("splitVertical rejects a locked target", () => {
        const { root, colA, paneA1 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, paneA1.id);
        const newNode = newLayoutNode(FlexDirection.Row, PANE_SIZE, undefined, { blockId: "newPane" });
        splitVertical({ rootNode: root, pendingBackendActions: [] } as any, {
            type: "splitvertical" as any,
            targetNodeId: paneA1.id,
            newNode,
            position: "after",
        } as any);

        // Tree unchanged: colA still has exactly its two original panes.
        expect(colA.children).toHaveLength(2);
        expect(paneA1.size).toBe(HeaderHeightPx);
    });

    it("enforceMinimizedLocks snaps a tampered minimized size back and repays the sibling", () => {
        const { root, colA, paneA1, paneA2 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, paneA1.id);
        const siblingSize = paneA2.size;

        // Simulate a writer that bypassed the reducer guards (stale tree push).
        paneA1.size = 120;
        paneA2.size = siblingSize - (120 - HeaderHeightPx);

        const snapped = enforceMinimizedLocks(root);
        expect(snapped).toBe(1);
        expect(paneA1.size).toBe(HeaderHeightPx);
        expect(paneA2.size).toBe(siblingSize);
    });

    it("enforceMinimizedLocks is a no-op on a tree that honors its locks", () => {
        const { root, colA, paneA1 } = buildTwoColumnLayout();
        const model = makeMockModel(root, { [colA.id]: { pixelToSizeRatio: 1 } });
        minimizeNodeToggle(model as any, paneA1.id);
        expect(enforceMinimizedLocks(root)).toBe(0);
    });
});

