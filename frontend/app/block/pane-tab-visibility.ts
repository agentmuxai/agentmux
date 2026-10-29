// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The ONE active/dormant/hidden signal for a pane tab — Pane Tab contract v1,
 * Phase 3 (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3 rule 5).
 *
 * It folds together what used to be three unrelated mechanisms: pane-stack
 * dormancy (`isBlockDormant`, set by pane-leaf-chrome's keep-alive slots),
 * whether the block's window tab is displayed (`useWindowTabDisplayed`, both
 * window-tab modes), and — for the browser — a DOM walk for hidden-tab
 * markers plus a `window` event to re-sync.
 *
 * - `"active"`: the user can see this tab.
 * - `"dormant"`: a kept-alive tab that isn't its pane's active one, or any tab of a minimized
 *   pane (its content is laid out but hidden, so nobody can see it).
 * - `"windowHidden"`: its window tab isn't displayed (wins over dormant).
 *
 * Call in the block's component scope (it reads a context) — a native
 * instance gets it as `ctx.visibility`.
 */

import { isBlockMinimized } from "@/app/block/pane-minimized";
import { isBlockDormant } from "@/app/store/block-component-registry";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import type { Accessor } from "solid-js";

export type PaneTabVisibility = "active" | "dormant" | "windowHidden";

export function usePaneTabVisibility(blockId: string): Accessor<PaneTabVisibility> {
    const displayed = useWindowTabDisplayed();
    const dormant = isBlockDormant(blockId);
    const minimized = isBlockMinimized(blockId);
    return () => (!displayed() ? "windowHidden" : dormant() || minimized() ? "dormant" : "active");
}
