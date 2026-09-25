// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How window tabs hide and show (`workspace.tsx`), and whether the window tab a
 * component lives in is the displayed one.
 *
 * With `window:keepinactivetabslaidout` (the default,
 * docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md §6.1–6.2)
 * an inactive tab stays laid out and hides with `visibility: hidden`, so it has
 * nothing to catch up on when shown — but its streaming agents would keep
 * rendering. Views pause that through the one pane-tab visibility signal
 * (`usePaneTabVisibility`, pane-tab-visibility.ts), which reads
 * `useWindowTabDisplayed` below in both modes.
 */
import { getSettingsKeyAtom } from "@/store/global";
import { createContext, useContext, type Accessor } from "solid-js";

/**
 * `window:keepinactivetabslaidout`: on unless set to `false`. The default
 * since the A/B on #3686/#3687 (analysis doc §7); `false` restores the
 * previous `content-visibility: hidden` behavior.
 */
export function keepInactiveTabsLaidOut(): boolean {
    return getSettingsKeyAtom("window:keepinactivetabslaidout")() !== false;
}

const WindowTabDisplayedContext = createContext<Accessor<boolean>>(() => true);

/** Provided per window tab by `workspace.tsx`: true while that tab is the
 *  displayed one, in BOTH modes (kept laid out, or `content-visibility:
 *  hidden`). Defaults to true outside any window tab (a floating or torn-off
 *  window). Read through `usePaneTabVisibility` (pane-tab-visibility.ts). */
export const WindowTabDisplayedProvider = WindowTabDisplayedContext.Provider;

export function useWindowTabDisplayed(): Accessor<boolean> {
    return useContext(WindowTabDisplayedContext);
}

/** How a window tab's container hides or shows (`workspace.tsx`). */
export interface TabContainerVisibility {
    "content-visibility": "visible" | "hidden";
    visibility: "hidden" | null;
    /** `"0"` for a tab kept laid out and not shown — see below. */
    opacity: "0" | null;
    "pointer-events": "auto" | "none";
    /** The tab is kept laid out and not shown; its agents pause rendering. */
    hiddenLaidOut: boolean;
}

/**
 * - Default: an inactive tab is `content-visibility: hidden`, which skips its
 *   layout until it's shown again.
 * - `keepLaidOut`: every tab stays `content-visibility: visible` (laid out),
 *   and an inactive one is `visibility: hidden` (not painted), like a hidden
 *   pane-stack member.
 * - Either way, only the displayed tab takes pointer events, and the reveal
 *   gate (`gated`) hides the displayed tab while it settles.
 */
export function tabContainerVisibility(
    displayed: boolean,
    keepLaidOut: boolean,
    gated: boolean
): TabContainerVisibility {
    const hiddenLaidOut = keepLaidOut && !displayed;
    return {
        "content-visibility": keepLaidOut || displayed ? "visible" : "hidden",
        visibility: gated || hiddenLaidOut ? "hidden" : null,
        // A hidden-but-laid-out tab also gets opacity 0: content can keep
        // painting through `visibility: hidden` (a `visibility` transition,
        // or an explicit `visibility: visible`), which showed up as a brief
        // "ghost" over the newly shown tab. Nothing escapes opacity.
        // SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §1.
        opacity: hiddenLaidOut ? "0" : null,
        "pointer-events": displayed ? "auto" : "none",
        hiddenLaidOut,
    };
}
