// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The landing bounce of a pane tab: which pills just arrived in which pane. Set by a strip's own
// drop handler and by code that moves tabs without a drop (a minimize that merges a pane into a
// minimized neighbour: layoutMinimizeMerge.ts). Kept apart from PaneTabStrip.tsx so layout code
// can mark a landing without importing the component.

import { createSignal } from "solid-js";

import { scaledMs } from "@/app/motion/motion";

/** How long a just-landed pill keeps `.pane-tab--landing` — the Window Tab
 *  bar's own clear-timeout for its bounce (tab-reorder.ts), so the two match. */
export const LANDING_BOUNCE_MS = 400;

/** The tab that was just dropped into place and should play the landing
 *  bounce: its id AND the pane (`paneKey`) it landed in. Module-level rather
 *  than per-strip: a cross-pane drop re-renders the pill in the destination
 *  strip (mounting it fresh there), so the flag has to be readable by
 *  whichever strip ends up rendering it. The pane is part of the identity
 *  because one block can have pills in several strips: agent fork lineages
 *  show blocks from OTHER panes as extra tabs (`PaneChromeModel.extraTabs`),
 *  and only the pill in the pane it actually landed in should bounce (Codex
 *  P2 on #3694). Only header strips set it.
 *  SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §3.4. */
const [landedTab, setLandedTabSignal] = createSignal<{ ids: ReadonlySet<string>; paneKey: string | undefined } | null>(null);
export { landedTab };
let landingTimer: ReturnType<typeof setTimeout> | undefined;
/** `id` is one tab, or every tab a multi-tab move brought into the pane. */
export function markLanded(id: string | readonly string[], paneKey: string | undefined): void {
    clearTimeout(landingTimer);
    setLandedTabSignal({ ids: new Set(typeof id === "string" ? [id] : id), paneKey });
    landingTimer = setTimeout(() => setLandedTabSignal(null), scaledMs(LANDING_BOUNCE_MS));
}
