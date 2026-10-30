// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isEffectivelyMinimized } from "./layoutInvariants";
import type { LayoutNode } from "./types";

/**
 * For each child of a Row node, resolve which sibling it should visually
 * dock onto instead of claiming its own row-slot: the nearest
 * non-minimized sibling, right-preferred, else left — generalizing
 * `SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md`'s per-pane "slip" and
 * `SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md`'s cascading
 * full-column dissolve into one rule via `isEffectivelyMinimized` (a
 * minimized leaf and a fully-collapsed branch are treated identically —
 * both need somewhere to dock). Unlike the original tree-surgery
 * implementation this never mutates the tree or needs restore-context
 * bookkeeping: it's recomputed fresh every render pass from nothing but the
 * `minimized` flag, so there is nothing for it to corrupt (see
 * `docs/retro/retro-minimize-display-mode-lost-slip-requirement-2026-07-17.md`).
 *
 * A child gets no entry (falls back to its own fixed-width chip slot via
 * `computeMainAxisAllocation`'s ordinary minimized-but-not-slipping path)
 * when it isn't effectively minimized, or when every sibling in the row is
 * ALSO effectively minimized (no valid anchor to dock onto — can happen
 * locally even though the last-expanded-pane guard prevents it globally,
 * since that guard counts leaves across the whole tree, not per row).
 * Multiple children can resolve to the same target; callers stack them.
 * `alsoMinimized` names one more child to treat as minimized: where it would dock if it were.
 * Pure; exported for unit tests.
 */
export function resolveRowSlipTargets(children: LayoutNode[], alsoMinimized?: string): Map<string, LayoutNode> {
    const targets = new Map<string, LayoutNode>();
    const minimized = children.map((c) => c.id === alsoMinimized || isEffectivelyMinimized(c));
    for (let i = 0; i < children.length; i++) {
        if (!minimized[i]) continue;
        let target: LayoutNode | undefined;
        for (let j = i + 1; j < children.length; j++) {
            if (!minimized[j]) {
                target = children[j];
                break;
            }
        }
        if (!target) {
            for (let j = i - 1; j >= 0; j--) {
                if (!minimized[j]) {
                    target = children[j];
                    break;
                }
            }
        }
        if (target) targets.set(children[i].id, target);
    }
    return targets;
}
