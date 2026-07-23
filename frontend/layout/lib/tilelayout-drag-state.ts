// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Module-level drag state shared across all three platform TileLayout
// implementations (win32/linux/darwin).
//
// Each platform's `DisplayNode` (defined locally in TileLayout.{win32,linux,
// darwin}.tsx — drag REGISTRATION is genuinely platform-specific, see those
// files) writes this state on drag start/drop. `OverlayNode` and
// `OverlayNodeWrapper` (shared, tilelayout-shared.tsx) — plus, on win32, the
// Windows-11 dragend safety net in TileLayoutComponent — read it. Extracted
// into its own module (rather than a plain module-level `let` per file) so
// all three platforms observe the SAME instance: a bare exported `let`
// cannot be reassigned from an importing module (only read), so a mutable
// object is used instead — `dragState.nodeId = x` mutates a property of the
// shared object, which import bindings allow.

import { LayoutModel } from "./layoutModel";
import { LayoutNode } from "./types";

export const dragState = {
    /** The node ID currently being dragged, or null when no drag is active. */
    nodeId: null as string | null,
    /** The LayoutModel that owns the in-progress drag. */
    layoutModel: null as LayoutModel | null,
    /**
     * The dragged LayoutNode itself — needed by cross-tab drags, where the
     * TARGET tab's overlay must compute a ghost placeholder for a node that
     * doesn't exist in its own tree (LayoutTreeComputeMoveNodeAction.nodeToMove).
     */
    node: null as LayoutNode | null,
};

/**
 * True when the drag in progress originated in a DIFFERENT tab's layout than
 * `model`'s — i.e. the user spring-switched tabs mid-drag
 * (SPEC_PANE_DRAG_TO_TAB_2026_07_10.md) and is now hovering this tab's panes.
 */
export function isCrossTabDrag(model: LayoutModel): boolean {
    return dragState.layoutModel != null && dragState.layoutModel !== model;
}
