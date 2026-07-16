// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Layout doctor — pure invariant validation for the layout tree, plus a
// compact tree dumper for logs. Mirrored by `validate_layout_invariants` in
// `agentmux-srv/src/backend/layout/mod.rs`; keep the two check lists in sync.
//
// Motivation (issue #2179): pane-minimize corruption kept shipping because
// nothing ever *observed* an illegal tree — each bug was reconstructed after
// the fact from db_layout archaeology. These checks run at the same choke
// points as `balanceNode`/`enforceMinimizedLocks` and turn a silent
// corruption into a loud, attributable log line at the moment it appears.

import type { LayoutNode } from "./types";
import { FlexDirection } from "./types";

export interface LayoutViolation {
    code: string;
    nodeId: string;
    detail: string;
}

const SIZE_EPS = 1e-4;

/**
 * A locked node is one whose size is owned by the minimize subsystem: a minimized
 * leaf (`minimizedSize`), a slipped header (`slipMinimize`), or a dissolved column
 * (`columnDissolve`). No other writer may resize, move, swap, or split it, and no
 * resize handle is rendered on its edges. See
 * `docs/specs/SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md`.
 *
 * Canonical home is here (the invariant module) so `layoutMinimize` can import
 * the doctor without an import cycle; `layoutMinimize` re-exports it for its
 * existing importers.
 */
export function isNodeLocked(node: LayoutNode | undefined): boolean {
    if (!node) return false;
    return node.minimizedSize !== undefined || node.slipMinimize !== undefined || node.columnDissolve !== undefined;
}

/**
 * Validate structural invariants of a layout tree. Returns an empty array on
 * a healthy tree. Never throws and never mutates.
 */
export function validateLayoutInvariants(root: LayoutNode | undefined): LayoutViolation[] {
    const violations: LayoutViolation[] = [];
    if (!root) return violations;

    function walk(node: LayoutNode, isRoot: boolean) {
        const isBranch = !!node.children?.length;
        const isLeaf = node.data !== undefined;

        // I1 — leaf XOR branch (matches validateNode / balance's own check,
        // but reported instead of thrown).
        if (isBranch === isLeaf) {
            violations.push({
                code: "LEAF_XOR_BRANCH",
                nodeId: node.id,
                detail: `data ${isLeaf ? "present" : "absent"}, children ${isBranch ? "present" : "absent"}`,
            });
        }

        // I2 — minimizedSize / slipMinimize are leaf-only markers. A branch
        // carrying one means a minimized leaf was promoted to a group without
        // migrating its minimize fields (e.g. addIntermediateNode, or the
        // leaf→Column conversions in _slipMinimize/_dissolveColumn).
        if (isBranch && (node.minimizedSize !== undefined || node.slipMinimize !== undefined)) {
            violations.push({
                code: "MIN_MARKER_ON_BRANCH",
                nodeId: node.id,
                detail: `branch has ${node.minimizedSize !== undefined ? "minimizedSize" : "slipMinimize"}`,
            });
        }

        // I3 — columnDissolve is branch-only.
        if (!isBranch && node.columnDissolve !== undefined) {
            violations.push({ code: "DISSOLVE_ON_LEAF", nodeId: node.id, detail: "leaf has columnDissolve" });
        }

        // I4 — a dissolved column must stack its headers vertically. A Row
        // direction here is the "narrow instead of short" signature (#2176).
        if (node.columnDissolve !== undefined && node.flexDirection !== FlexDirection.Column) {
            violations.push({
                code: "DISSOLVED_NOT_COLUMN",
                nodeId: node.id,
                detail: `dissolved column has flexDirection=${node.flexDirection}`,
            });
        }

        // I5 — every child of a dissolved column is itself locked (dissolve
        // fires only when allCollapsed; an unlocked child means something was
        // inserted into the header strip).
        if (node.columnDissolve !== undefined && node.children) {
            for (const c of node.children) {
                if (!isNodeLocked(c)) {
                    violations.push({
                        code: "DISSOLVED_CHILD_UNLOCKED",
                        nodeId: c.id,
                        detail: `unlocked child inside dissolved column ${node.id}`,
                    });
                }
            }
        }

        // I6 — a locked node's size honors its recorded lock (#2180).
        if (isNodeLocked(node) && node.minimizedLockedSize !== undefined) {
            if (Math.abs(node.size - node.minimizedLockedSize) > SIZE_EPS) {
                violations.push({
                    code: "LOCK_SIZE_MISMATCH",
                    nodeId: node.id,
                    detail: `size=${node.size} locked=${node.minimizedLockedSize}`,
                });
            }
        }

        // I7 — minimizedLockedSize must not outlive its lock marker.
        if (!isNodeLocked(node) && node.minimizedLockedSize !== undefined) {
            violations.push({
                code: "ORPHAN_LOCKED_SIZE",
                nodeId: node.id,
                detail: `minimizedLockedSize=${node.minimizedLockedSize} with no lock marker`,
            });
        }

        // I8 — sizes are positive (a zero/negative flex size renders as dead
        // or inverted space).
        if (!isRoot && !(node.size > 0)) {
            violations.push({ code: "NONPOSITIVE_SIZE", nodeId: node.id, detail: `size=${node.size}` });
        }

        node.children?.forEach((c) => walk(c, false));
    }

    walk(root, true);

    // I9 — at least one pane stays expanded. A tree whose every leaf is
    // minimize-locked is an all-headers window with nothing restorable in
    // view; `minimizeNodeToggle` guards against producing this.
    let leafCount = 0;
    let expandedCount = 0;
    (function countLeaves(node: LayoutNode) {
        if (!node.children?.length) {
            if (node.data !== undefined) {
                leafCount++;
                if (
                    node.minimizedSize === undefined &&
                    node.slipMinimize === undefined &&
                    node.columnDissolve === undefined
                ) {
                    expandedCount++;
                }
            }
            return;
        }
        node.children.forEach(countLeaves);
    })(root);
    if (leafCount > 0 && expandedCount === 0) {
        violations.push({
            code: "ALL_LEAVES_LOCKED",
            nodeId: root.id,
            detail: `all ${leafCount} leaves are minimize-locked; no expanded pane remains`,
        });
    }

    return violations;
}

/**
 * Compact one-line-per-node dump of a layout tree for violation logs —
 * enough to reconstruct the shape without a full JSON blob.
 */
export function describeLayoutTree(root: LayoutNode | undefined): string {
    if (!root) return "(empty tree)";
    const lines: string[] = [];
    function walk(node: LayoutNode, depth: number) {
        const flags = [
            node.minimizedSize !== undefined ? `MIN(orig=${node.minimizedSize})` : "",
            node.minimizedLockedSize !== undefined ? `LOCK=${node.minimizedLockedSize}` : "",
            node.slipMinimize !== undefined ? "SLIP" : "",
            node.columnDissolve !== undefined ? "DISSOLVED" : "",
            node._slipAnchor ? "anchor" : "",
        ]
            .filter(Boolean)
            .join(" ");
        const kind = node.data?.blockId ? `leaf ${String(node.data.blockId).slice(0, 8)}` : "branch";
        lines.push(
            `${"  ".repeat(depth)}${kind} id=${node.id.slice(0, 8)} dir=${node.flexDirection} size=${node.size}${flags ? " " + flags : ""}`
        );
        node.children?.forEach((c) => walk(c, depth + 1));
    }
    walk(root, 0);
    return lines.join("\n");
}

// Dedupe repeated identical violation reports (updateTree runs on every
// geometry pass; one corrupted tree would otherwise spam the console).
let lastReportSignature: string | undefined;

/**
 * Validate and report violations to the console. `context` says which choke
 * point observed the tree (e.g. "updateTree", "minimizeToggle:restore").
 * Returns the violations so callers can react programmatically.
 */
export function reportLayoutViolations(
    root: LayoutNode | undefined,
    context: string
): LayoutViolation[] {
    const violations = validateLayoutInvariants(root);
    if (violations.length === 0) {
        lastReportSignature = undefined;
        return violations;
    }
    const signature = JSON.stringify(violations);
    if (signature !== lastReportSignature) {
        lastReportSignature = signature;
        console.error(
            `[layout-doctor] ${violations.length} invariant violation(s) after ${context}:\n` +
                violations.map((v) => `  ${v.code} @ ${v.nodeId.slice(0, 8)}: ${v.detail}`).join("\n") +
                `\ntree:\n${describeLayoutTree(root)}`
        );
    }
    return violations;
}
