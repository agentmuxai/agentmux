// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { newLayoutNode } from "../lib/layoutNode";
import { validateLayoutInvariants, describeLayoutTree } from "../lib/layoutInvariants";
import { minimizeNodeToggle } from "../lib/layoutMinimize";
import { FlexDirection, type LayoutNode, type LayoutNodeAdditionalProps } from "../lib/types";

function makeMockModel(rootNode: LayoutNode, addlProps: Record<string, Partial<LayoutNodeAdditionalProps>> = {}) {
    let minimizedSet = new Set<string>();
    return {
        treeState: { rootNode, pendingBackendActions: [] },
        getter: (_sig: unknown) => addlProps as Record<string, LayoutNodeAdditionalProps>,
        additionalProps: {} as any,
        gapSizePx: () => 0,
        minimizedNodeIds: {
            _set: (u: Set<string> | ((prev: Set<string>) => Set<string>)) => {
                minimizedSet = typeof u === "function" ? u(minimizedSet) : u;
            },
        },
        updateTree: () => {},
        localTreeStateAtom: { _set: () => {} },
        persistToBackend: () => {},
    };
}

describe("validateLayoutInvariants", () => {
    it("returns no violations for a healthy tree produced by a real minimize", () => {
        const pane1 = newLayoutNode(FlexDirection.Row, 200, undefined, { blockId: "p1" });
        const pane2 = newLayoutNode(FlexDirection.Row, 200, undefined, { blockId: "p2" });
        const col = newLayoutNode(FlexDirection.Column, 200, [pane1, pane2]);
        const other = newLayoutNode(FlexDirection.Column, 200, [
            newLayoutNode(FlexDirection.Row, 200, undefined, { blockId: "p3" }),
        ]);
        const root = newLayoutNode(FlexDirection.Row, 200, [col, other]);
        const model = makeMockModel(root, { [col.id]: { pixelToSizeRatio: 1 } });

        minimizeNodeToggle(model as any, pane1.id);

        expect(validateLayoutInvariants(root)).toEqual([]);
    });

    // Regression fixture: the exact corrupted shape recovered from a live
    // v0.53.6 instance's db_layout after the user's "minimized 2 panes and it
    // distorted" repro (issue #2179) — a BRANCH carrying the leaf-only
    // `minimizedSize` marker, with a slipped leaf trapped inside it:
    //
    //   root (Row)
    //   ├── branch (Column, _slipAnchor)
    //   │   └── branch (Row, minimizedSize=9)   ← illegal: MIN marker on branch
    //   │       ├── leaf term  (slipMinimize)
    //   │       └── leaf agent
    //   └── leaf armory
    it("flags the corrupted tree recovered from the live 0.53.6 instance", () => {
        const term = newLayoutNode(FlexDirection.Column, 2.2, undefined, { blockId: "term" });
        term.slipMinimize = { targetColumnId: "x", originalRowSize: 10, originalRowIndex: 0, targetWasLeaf: true };
        const agent = newLayoutNode(FlexDirection.Column, 4.4, undefined, { blockId: "agent" });
        const midBranch = newLayoutNode(FlexDirection.Row, 6.8, [term, agent]);
        midBranch.minimizedSize = 9;
        const outerCol = newLayoutNode(FlexDirection.Column, 30, [midBranch]);
        outerCol._slipAnchor = true;
        const armory = newLayoutNode(FlexDirection.Column, 10, undefined, { blockId: "armory" });
        const root = newLayoutNode(FlexDirection.Row, 10, [outerCol, armory]);

        const violations = validateLayoutInvariants(root);
        expect(violations.map((v) => v.code)).toContain("MIN_MARKER_ON_BRANCH");
    });

    it("flags a dissolved column whose direction was flipped (the #2176 signature)", () => {
        const l1 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b1" });
        l1.minimizedSize = 200;
        l1.minimizedLockedSize = 33;
        const l2 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b2" });
        l2.minimizedSize = 200;
        l2.minimizedLockedSize = 33;
        const dissolved = newLayoutNode(FlexDirection.Row, 66, [l1, l2]); // Row = wrong
        dissolved.columnDissolve = {
            targetColumnId: "host",
            originalRowSize: 200,
            originalRowIndex: 0,
            targetWasLeaf: true,
        };
        const content = newLayoutNode(FlexDirection.Row, 300, undefined, { blockId: "b3" });
        const host = newLayoutNode(FlexDirection.Column, 400, [dissolved, content]);

        const codes = validateLayoutInvariants(host).map((v) => v.code);
        expect(codes).toContain("DISSOLVED_NOT_COLUMN");
    });

    it("flags a locked node whose size was tampered with, and an orphaned lock", () => {
        const locked = newLayoutNode(FlexDirection.Row, 120, undefined, { blockId: "b1" });
        locked.minimizedSize = 200;
        locked.minimizedLockedSize = 33; // size 120 ≠ locked 33
        const orphan = newLayoutNode(FlexDirection.Row, 200, undefined, { blockId: "b2" });
        orphan.minimizedLockedSize = 33; // no lock marker
        const root = newLayoutNode(FlexDirection.Column, 400, [locked, orphan]);

        const codes = validateLayoutInvariants(root).map((v) => v.code);
        expect(codes).toContain("LOCK_SIZE_MISMATCH");
        expect(codes).toContain("ORPHAN_LOCKED_SIZE");
    });

    it("flags an unlocked child inside a dissolved column and nonpositive sizes", () => {
        const l1 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b1" });
        l1.minimizedSize = 200;
        const intruder = newLayoutNode(FlexDirection.Row, 0, undefined, { blockId: "b9" }); // also size 0
        const dissolved = newLayoutNode(FlexDirection.Column, 66, [l1, intruder]);
        dissolved.columnDissolve = {
            targetColumnId: "host",
            originalRowSize: 200,
            originalRowIndex: 0,
            targetWasLeaf: true,
        };
        const content = newLayoutNode(FlexDirection.Row, 300, undefined, { blockId: "b3" });
        const host = newLayoutNode(FlexDirection.Column, 400, [dissolved, content]);

        const codes = validateLayoutInvariants(host).map((v) => v.code);
        expect(codes).toContain("DISSOLVED_CHILD_UNLOCKED");
        expect(codes).toContain("NONPOSITIVE_SIZE");
    });

    it("flags a tree where every leaf is minimize-locked (all-headers window) — legacy and flag models", () => {
        const l1 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b1" });
        l1.minimizedSize = 200; // legacy marker
        const l2 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b2" });
        l2.minimized = true; // display-mode flag
        const root = newLayoutNode(FlexDirection.Column, 400, [l1, l2]);

        const codes = validateLayoutInvariants(root).map((v) => v.code);
        expect(codes).toContain("ALL_LEAVES_LOCKED");
    });

    it("flags the minimized display-mode flag on a branch (leaf-only)", () => {
        const l1 = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "b1" });
        const l2 = newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: "b2" });
        const branch = newLayoutNode(FlexDirection.Column, 200, [l1, l2]);
        branch.minimized = true; // illegal: flag is leaf-only
        const root = newLayoutNode(FlexDirection.Row, 200, [
            branch,
            newLayoutNode(FlexDirection.Column, 200, undefined, { blockId: "b3" }),
        ]);

        const codes = validateLayoutInvariants(root).map((v) => v.code);
        expect(codes).toContain("MIN_MARKER_ON_BRANCH");
    });

    it("describeLayoutTree renders flag and legacy lock markers distinctly", () => {
        const l1 = newLayoutNode(FlexDirection.Row, 33, undefined, { blockId: "b1" });
        l1.minimizedSize = 200;
        l1.minimizedLockedSize = 33;
        const l2 = newLayoutNode(FlexDirection.Row, 367, undefined, { blockId: "b2" });
        l2.minimized = true;
        const root = newLayoutNode(FlexDirection.Column, 400, [l1, l2]);
        const desc = describeLayoutTree(root);
        expect(desc).toContain("legacyMIN(orig=200)");
        expect(desc).toContain("legacyLOCK=33");
        expect(desc).toContain(" MIN");
    });
});
