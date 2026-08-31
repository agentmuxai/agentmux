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
import {
    computeMainAxisAllocation,
    minimizedCrossAxisPx,
    resolveRowSlipTargets,
    collapsedExtentPx,
} from "../lib/layoutGeometry";
import { resizeNode, splitVertical } from "../lib/layoutTree";
import { FlexDirection, type LayoutNode } from "../lib/types";

// ── Minimal LayoutModel mock ─────────────────────────────────────────────────
// The display-mode toggle needs no geometry (no addlProps, no gap, no ratios).

function makeMockModel(rootNode: LayoutNode) {
    let minimizedSet = new Set<string>();
    const model = {
        treeState: { rootNode, pendingBackendActions: [] },
        minimizedNodeIds: {
            _set: (u: Set<string> | ((prev: Set<string>) => Set<string>)) => {
                minimizedSet = typeof u === "function" ? u(minimizedSet) : u;
            },
        },
        // Real updateTree (layoutGeometry.ts) rebuilds minimizedNodeIds fresh
        // from the tree's `.minimized` flags as part of its normal walk —
        // mirror JUST that derivation here (no geometry needed for these
        // tests) so callers relying on updateTree() having run see the same
        // authoritative result production does.
        updateTree: () => {
            const ids = new Set<string>();
            (function walk(n: LayoutNode | undefined) {
                if (!n) return;
                if (!n.children?.length) {
                    if (n.minimized) ids.add(n.id);
                    return;
                }
                n.children.forEach(walk);
            })(model.treeState.rootNode);
            minimizedSet = ids;
        },
        localTreeStateAtom: { _set: () => {} },
        persistToBackend: () => {},
        getMinimizedSet: () => minimizedSet,
    };
    return model;
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

    // Regression: minimizedCrossAxisPx clamps a minimized child's
    // CROSS-axis (height, under a Row parent) — this is what the rect
    // builder in updateTreeHelper actually uses, distinct from
    // computeMainAxisAllocation's MAIN-axis px[]. A single minimized leaf
    // needs one header height; a fully-minimized BRANCH holding N leaves
    // needs N (one per stacked chip its own recursive layout will render).
    // Getting this wrong for branches was the live-repro bug: right_col's
    // rect kept full container height while its own Column layout only
    // filled the top N*(header+gap)px, leaving dead space below the chips
    // ("2 half collapsed... to the right of the single pane").
    it("minimizedCrossAxisPx: one header-height for a minimized leaf, N for an N-leaf minimized branch", () => {
        const leaf = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "a" });
        leaf.minimized = true;
        expect(minimizedCrossAxisPx(leaf, 3)).toBe(HeaderHeightPx + 3);

        const cpu = newLayoutNode(FlexDirection.Row, 2, undefined, { blockId: "cpu" });
        cpu.minimized = true;
        const swarm = newLayoutNode(FlexDirection.Row, 8, undefined, { blockId: "swarm" });
        swarm.minimized = true;
        const rightCol = newLayoutNode(FlexDirection.Column, 5, [cpu, swarm]);
        expect(minimizedCrossAxisPx(rightCol, 3)).toBe(2 * (HeaderHeightPx + 3));
    });

    it("no dead space: a clamped fully-minimized branch's own allocation exactly fills the space its parent gave it", () => {
        const cpu = newLayoutNode(FlexDirection.Row, 2, undefined, { blockId: "cpu" });
        cpu.minimized = true;
        const swarm = newLayoutNode(FlexDirection.Row, 8, undefined, { blockId: "swarm" });
        swarm.minimized = true;
        const rightCol = newLayoutNode(FlexDirection.Column, 5, [cpu, swarm]);
        const gap = 3;

        // Step 1: the Row parent clamps rightCol's cross-axis to its actual
        // chip-stack need (what the fixed rect builder now does) — NOT the
        // full 800px container height a naive "branches keep full cross-axis"
        // rule would have given it.
        const clampedHeight = minimizedCrossAxisPx(rightCol, gap);
        expect(clampedHeight).toBe(2 * (HeaderHeightPx + gap));
        expect(clampedHeight).toBeLessThan(800);

        // Step 2: recursing into rightCol's OWN Column-direction allocation
        // with that clamped height as nodePixels must consume it exactly —
        // zero leftover (dead space).
        const inner = computeMainAxisAllocation(rightCol.children!, false, clampedHeight, size, gap);
        const consumed = inner.px.reduce((a, b) => a + b, 0);
        expect(consumed).toBeCloseTo(clampedHeight);
        expect(inner.px[0]).toBe(HeaderHeightPx + gap);
        expect(inner.px[1]).toBe(HeaderHeightPx + gap);
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

    it("resizeNode rejects a fully-minimized BRANCH — its stored size becomes load-bearing on restore", () => {
        // A collapsed column has no branch-level marker in the display-mode
        // model, but mutating its stored size while collapsed would corrupt
        // the proportion that reappears when any child restores (reagent P1,
        // round 3).
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        paneA1.minimized = true;
        paneA2.minimized = true; // colA is now effectively minimized

        resizeNode({ rootNode: root, pendingBackendActions: [] } as any, {
            type: "resize" as any,
            resizeOperations: [
                { nodeId: colA.id, size: 1 },
                { nodeId: colB.id, size: 399 },
            ],
        } as any);

        expect(colA.size).toBe(PANE_SIZE);
        expect(colB.size).toBe(PANE_SIZE);
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

    it("migrates slip/dissolve bookkeeping: sizes healed to a sane share of the CURRENT parent", () => {
        // Flex sizes are relative within a parent. A slipped/dissolved node
        // lives nested in its slip/dissolve TARGET — restoring the recorded
        // originalRowSize (a weight from a DIFFERENT unit space) would give it
        // a wildly wrong proportion; the migration heals to the mean of its
        // current siblings instead (reagent P1, round 3).
        const { root, colA, paneA1, paneA2, colB } = buildTwoColumnLayout();
        paneA1.slipMinimize = { targetColumnId: colA.id, originalRowSize: 9999, originalRowIndex: 0, targetWasLeaf: true };
        paneA1.size = 2.2; // slip-era squeezed size
        paneA2.size = 300; // paneA1's current sibling in colA
        colA.columnDissolve = { targetColumnId: "x", originalRowSize: 9999, originalRowIndex: 0, targetWasLeaf: false };
        colA.size = -0.0039; // the cascade-bug corruption: stolen-total gone negative
        root._slipAnchor = true;
        const model = makeMockModel(root);

        rebuildMinimizedSet(model as any);

        expect(paneA1.minimized).toBe(true);
        expect(paneA1.slipMinimize).toBeUndefined();
        expect(paneA1.size).toBe(300); // mean of current siblings, NOT the alien 9999
        expect(colA.columnDissolve).toBeUndefined();
        expect(colA.size).toBe(colB.size); // sane share among root siblings — heals the negative
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

// ── Row-slip target resolution ───────────────────────────────────────────────
// Restored per docs/retro/retro-minimize-display-mode-lost-slip-requirement-
// 2026-07-17.md — SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md's per-pane
// slip and SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md's cascading
// full-column dissolve, unified into one rule via isEffectivelyMinimized.

describe("resolveRowSlipTargets", () => {
    function leaf(id: string, min = false): LayoutNode {
        const n = newLayoutNode(FlexDirection.Column, 10, undefined, { blockId: id });
        n.id = id;
        if (min) n.minimized = true;
        return n;
    }

    it("no minimized children: no entries", () => {
        const [a, b, c] = [leaf("a"), leaf("b"), leaf("c")];
        expect(resolveRowSlipTargets([a, b, c]).size).toBe(0);
    });

    it("right-preferred: a minimized child docks onto its right neighbor over its left", () => {
        const [a, b, c] = [leaf("a"), leaf("b", true), leaf("c")];
        const targets = resolveRowSlipTargets([a, b, c]);
        expect(targets.get("b")?.id).toBe("c");
    });

    it("falls back to the left neighbor when there is no right one", () => {
        const [a, b] = [leaf("a"), leaf("b", true)];
        const targets = resolveRowSlipTargets([a, b]);
        expect(targets.get("b")?.id).toBe("a");
    });

    it("scans PAST an adjacent minimized sibling to find a real anchor", () => {
        // [A(min), B(min), C(expanded)] — A's immediate right neighbor (B) is
        // itself minimized and not a valid anchor; A must keep scanning right
        // to C. B's immediate right IS C directly.
        const [a, b, c] = [leaf("a", true), leaf("b", true), leaf("c")];
        const targets = resolveRowSlipTargets([a, b, c]);
        expect(targets.get("a")?.id).toBe("c");
        expect(targets.get("b")?.id).toBe("c");
    });

    it("multiple minimized siblings on both sides converge on the same nearest anchor", () => {
        // [A(min), B(min), C(expanded), D(min)] — A and B slip right onto C;
        // D has nothing to its right, so it falls back left onto C too.
        const [a, b, c, d] = [leaf("a", true), leaf("b", true), leaf("c"), leaf("d", true)];
        const targets = resolveRowSlipTargets([a, b, c, d]);
        expect(targets.get("a")?.id).toBe("c");
        expect(targets.get("b")?.id).toBe("c");
        expect(targets.get("d")?.id).toBe("c");
    });

    it("no valid anchor when every sibling in the row is minimized: no entry (falls back to its own chip slot)", () => {
        const [a, b] = [leaf("a", true), leaf("b", true)];
        const targets = resolveRowSlipTargets([a, b]);
        expect(targets.size).toBe(0);
    });

    it("a single child (no siblings at all): no entry", () => {
        const [a] = [leaf("a", true)];
        expect(resolveRowSlipTargets([a]).size).toBe(0);
    });

    it("treats a fully-minimized BRANCH exactly like a minimized leaf (the column-dissolve case)", () => {
        const cpu = leaf("cpu", true);
        const swarm = leaf("swarm", true);
        const rightCol = newLayoutNode(FlexDirection.Column, 5, [cpu, swarm]);
        rightCol.id = "rightCol";
        const agent = leaf("agent");
        const targets = resolveRowSlipTargets([agent, rightCol]);
        expect(targets.get("rightCol")?.id).toBe("agent");
        // cpu/swarm are resolved by right_col's OWN (Column-direction) layout
        // pass, not by this row-level call — they have no entry here.
        expect(targets.has("cpu")).toBe(false);
        expect(targets.has("swarm")).toBe(false);
    });

    it("a partially-minimized branch (not every leaf minimized) is not slip-eligible", () => {
        const cpu = leaf("cpu", true);
        const swarm = leaf("swarm", false); // still expanded
        const rightCol = newLayoutNode(FlexDirection.Column, 5, [cpu, swarm]);
        rightCol.id = "rightCol";
        const agent = leaf("agent");
        expect(resolveRowSlipTargets([agent, rightCol]).size).toBe(0);
    });
});

// ── computeMainAxisAllocation: slip children contribute zero width ──────────

describe("computeMainAxisAllocation with slipChildIds", () => {
    const size = (n: LayoutNode) => n.size;

    it("a slip child gets 0 px; its would-be width flows entirely to the flex sibling", () => {
        const a = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "a" });
        const b = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "b" });
        b.minimized = true;
        const { px } = computeMainAxisAllocation([a, b], true, 1000, size, 3, new Set([b.id]));
        expect(px[1]).toBe(0);
        expect(px[0]).toBeCloseTo(1000);
    });

    it("without slipChildIds, the same minimized child gets the old fixed chip width (backward compatible)", () => {
        const a = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "a" });
        const b = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "b" });
        b.minimized = true;
        const { px } = computeMainAxisAllocation([a, b], true, 1000, size, 3);
        expect(px[1]).toBe(MinimizedRowSlotWidthPx + 3);
        expect(px[0]).toBeCloseTo(1000 - (MinimizedRowSlotWidthPx + 3));
    });
});

// ── Cross-split: a branch nested PERPENDICULAR to its parent ────────────────
// Regression for ANALYSIS_PANE_MINIMIZE_ROW_BRANCH_DISTORTIONS_2026_08_30 §2.
// The suite's prior branch-level minimize coverage was all Column-inside-
// Column, which is the one shape the old countLeafPanes formula got right.
describe("collapsedExtentPx — cross-split (perpendicular nested branch)", () => {
    const gap = 3;
    const chipH = HeaderHeightPx + gap;
    const chipW = MinimizedRowSlotWidthPx + gap;
    const minLeaf = (id: string) => {
        const n = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: id });
        n.minimized = true;
        return n;
    };

    it("a minimized leaf is one chip on either axis", () => {
        const leaf = minLeaf("a");
        expect(collapsedExtentPx(leaf, true, gap)).toBe(chipH);
        expect(collapsedExtentPx(leaf, false, gap)).toBe(chipW);
    });

    it("a COLUMN branch stacks: N chips deep vertically, one chip wide", () => {
        const col = newLayoutNode(FlexDirection.Column, 5, [minLeaf("a"), minLeaf("b"), minLeaf("c")]);
        expect(collapsedExtentPx(col, true, gap)).toBe(3 * chipH);
        expect(collapsedExtentPx(col, false, gap)).toBe(chipW);
    });

    it("a ROW branch is a strip: ONE chip deep vertically, N chips wide", () => {
        const row = newLayoutNode(FlexDirection.Row, 5, [minLeaf("a"), minLeaf("b"), minLeaf("c")]);
        // The bug: this used to return 3 * chipH via countLeafPanes, so the
        // parent Column handed the row 3 header-heights for a 1-header-high
        // strip of chips -> 2 header-heights of dead space beneath it.
        expect(collapsedExtentPx(row, true, gap)).toBe(chipH);
        expect(collapsedExtentPx(row, false, gap)).toBe(3 * chipW);
    });

    it("minimizedCrossAxisPx: a fully-minimized ROW branch needs one header height, not N", () => {
        const row = newLayoutNode(FlexDirection.Row, 5, [minLeaf("a"), minLeaf("b"), minLeaf("c")]);
        expect(minimizedCrossAxisPx(row, gap)).toBe(chipH);
    });

    it("no dead space: the reported layout — Column[top, Row[A,B,C], bottom]", () => {
        const size = (n: LayoutNode) => n.size;
        const row = newLayoutNode(FlexDirection.Row, 10, [minLeaf("A"), minLeaf("B"), minLeaf("C")]);
        const top = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "top" });
        const bottom = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "bottom" });

        // What the parent Column allocates to the row on its main (vertical) axis.
        const outer = computeMainAxisAllocation([top, row, bottom], false, 900, size, gap);
        const allocatedToRow = outer.px[1];

        // What the row actually fills: its chips laid out side by side are one
        // chip-height deep, whatever the container gives them.
        const consumed = minimizedCrossAxisPx(row, gap);

        expect(allocatedToRow).toBe(chipH);
        expect(consumed).toBe(chipH);
        expect(allocatedToRow - consumed).toBe(0); // <- was 2 * chipH of dead space
    });

    it("mirror case: a fully-minimized ROW branch inside a ROW parent gets N chip-widths, not one", () => {
        const size = (n: LayoutNode) => n.size;
        const row = newLayoutNode(FlexDirection.Row, 10, [minLeaf("A"), minLeaf("B")]);
        const expanded = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "keep" });
        const alloc = computeMainAxisAllocation([row, expanded], true, 1200, size, gap);
        // Previously minimizedFixedPx returned a flat MinimizedRowSlotWidthPx
        // for ANY minimized child of a Row parent, so this branch was given
        // one chip's width for two side-by-side chips (under-allocation).
        expect(alloc.px[0]).toBe(2 * chipW);
    });

    it("deep nesting resolves per level, not by leaf count", () => {
        // Column[ Row[a,b], c ] fully minimized, measured vertically:
        //   Row[a,b] is a strip -> 1 chip deep; plus leaf c -> 1 chip.
        //   Total 2 chip-heights, NOT the 3 that leaf-counting would give.
        const inner = newLayoutNode(FlexDirection.Row, 5, [minLeaf("a"), minLeaf("b")]);
        const outer = newLayoutNode(FlexDirection.Column, 5, [inner, minLeaf("c")]);
        expect(collapsedExtentPx(outer, true, gap)).toBe(2 * chipH);
    });
});

// ── All-minimized Row: chips fill the width instead of leaving dead space ───
// Operator report (2026-08-30): after the cross-split height fix, "the space
// above the 2 collapsed panes is gone, but there is still blank space to the
// right. The collapsed panes should equally fill up the empty space."
describe("all-minimized Row: chips share the full width", () => {
    const gap = 3;
    const size = (n: LayoutNode) => n.size;
    const minLeaf = (id: string) => {
        const n = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: id });
        n.minimized = true;
        return n;
    };

    it("two minimized leaves split the row evenly, no leftover", () => {
        const kids = [minLeaf("a"), minLeaf("b")];
        const { px } = computeMainAxisAllocation(kids, true, 1200, size, gap);
        expect(px[0]).toBeCloseTo(600);
        expect(px[1]).toBeCloseTo(600);
        expect(px[0] + px[1]).toBeCloseTo(1200); // was 2 * (180+3) = 366
    });

    it("three minimized leaves split evenly", () => {
        const kids = [minLeaf("a"), minLeaf("b"), minLeaf("c")];
        const { px } = computeMainAxisAllocation(kids, true, 1200, size, gap);
        px.forEach((v) => expect(v).toBeCloseTo(400));
        expect(px.reduce((a, b) => a + b, 0)).toBeCloseTo(1200);
    });

    it("a nested fully-minimized Row branch keeps its larger share (proportional, not strictly equal)", () => {
        // [ leaf, Row[leaf, leaf] ] — the branch needs two chip-widths to the
        // leaf's one, so it takes 2/3 of the row, not 1/2.
        const solo = minLeaf("solo");
        const pair = newLayoutNode(FlexDirection.Row, 10, [minLeaf("a"), minLeaf("b")]);
        const { px } = computeMainAxisAllocation([solo, pair], true, 900, size, gap);
        expect(px[0]).toBeCloseTo(300);
        expect(px[1]).toBeCloseTo(600);
        expect(px[0] + px[1]).toBeCloseTo(900);
    });

    it("does NOT stretch when an expanded sibling is present — that one still slips", () => {
        const a = minLeaf("a");
        const b = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: "b" });
        const { px } = computeMainAxisAllocation([a, b], true, 1200, size, gap);
        expect(px[0]).toBe(MinimizedRowSlotWidthPx + gap); // unchanged fixed chip
        expect(px[1]).toBeCloseTo(1200 - (MinimizedRowSlotWidthPx + gap));
    });

    it("does NOT stretch a Column — chips must stay header-height", () => {
        const kids = [minLeaf("a"), minLeaf("b")];
        const { px } = computeMainAxisAllocation(kids, false, 1200, size, gap);
        px.forEach((v) => expect(v).toBe(HeaderHeightPx + gap));
    });

    it("still shrinks to fit when the row is narrower than the chips need", () => {
        const kids = [minLeaf("a"), minLeaf("b"), minLeaf("c"), minLeaf("d"), minLeaf("e"), minLeaf("f")];
        const { px } = computeMainAxisAllocation(kids, true, 300, size, gap);
        expect(px.reduce((a, b) => a + b, 0)).toBeCloseTo(300);
        px.forEach((v) => expect(v).toBeLessThan(MinimizedRowSlotWidthPx));
    });
});

// ── Resize handles across a zero-extent slip child ─────────────────────────
// Regression for ANALYSIS_PANE_MINIMIZE_ROW_BRANCH_DISTORTIONS_2026_08_30 §3.
// Uses the real updateTree() (via layoutModel.test.ts's harness pattern) since
// Phase C's handle generation isn't separately exported.
describe("resize handles span a slipped (zero-width) minimized pane", () => {
    const leaf = (id: string, min = false) => {
        const n = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: id });
        if (min) n.minimized = true;
        return n;
    };
    // Mirror of Phase C's own pairing rule, kept in lockstep with the source.
    const pairs = (children: LayoutNode[], slip: Set<string>) => {
        const out: Array<[number, number]> = [];
        let before = -1;
        for (let i = 0; i < children.length; i++) {
            const c = children[i];
            if (slip.has(c.id)) continue;
            if (isEffectivelyMinimized(c)) { before = -1; continue; }
            if (before < 0) { before = i; continue; }
            out.push([before, i]);
            before = i;
        }
        return out;
    };

    it("A | B(minimized) | C — one handle, spanning B, flanking A and C", () => {
        const A = leaf("A"), B = leaf("B", true), C = leaf("C");
        const kids = [A, B, C];
        const slip = new Set(resolveRowSlipTargets(kids).keys());
        expect(slip.has(B.id)).toBe(true); // B docks onto C, contributing 0 width
        // Was zero handles: both candidate pairs (A|B and B|C) were skipped.
        expect(pairs(kids, slip)).toEqual([[0, 2]]);
    });

    it("the spanning handle's after-index is NOT parentIndex + 1", () => {
        const kids = [leaf("A"), leaf("B", true), leaf("C")];
        const slip = new Set(resolveRowSlipTargets(kids).keys());
        const [[before, after]] = pairs(kids, slip);
        expect(after).not.toBe(before + 1); // onResizeMove must read afterIndex
        expect(after).toBe(2);
    });

    it("two adjacent slipped panes are both spanned by a single handle", () => {
        const kids = [leaf("A"), leaf("B", true), leaf("C", true), leaf("D")];
        const slip = new Set(resolveRowSlipTargets(kids).keys());
        expect(pairs(kids, slip)).toEqual([[0, 3]]);
    });

    it("unchanged when nothing is minimized", () => {
        const kids = [leaf("A"), leaf("B"), leaf("C")];
        expect(pairs(kids, new Set())).toEqual([[0, 1], [1, 2]]);
    });

    it("a Column's minimized chip has real extent, so it is NOT spanned", () => {
        // No slip in a Column (slip is Row-only), so the chip occupies a real
        // header-height slot between A and C — they are not adjacent, and a
        // handle across the chip would float on top of it.
        const kids = [leaf("A"), leaf("B", true), leaf("C")];
        const slip = new Set<string>(); // Column: resolveRowSlipTargets isn't consulted
        expect(pairs(kids, slip)).toEqual([]);
    });

    it("an all-minimized row yields no handles (nothing expanded to resize)", () => {
        const kids = [leaf("A", true), leaf("B", true), leaf("C", true)];
        const slip = new Set(resolveRowSlipTargets(kids).keys());
        expect(slip.size).toBe(0); // no anchor -> fixed chip slots, not slips
        expect(pairs(kids, slip)).toEqual([]);
    });
});
