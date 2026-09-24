// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the window tab a component lives in is hidden right now, when
 * inactive tabs are kept laid out (`window:keepinactivetabslaidout`,
 * docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md §6.1–6.2).
 *
 * Kept-laid-out tabs hide with `visibility: hidden`, so their content keeps
 * taking part in layout and has nothing to catch up on when shown. The cost
 * is that a hidden tab's streaming agents would keep rendering; this signal
 * lets them pause rendering the way hidden pane-stack members already do
 * (`agent-dormancy.tsx`).
 *
 * Always `false` with the setting set to `false`: `content-visibility: hidden` already
 * skips a hidden tab's rendering, so there is nothing extra to pause.
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

const WindowTabHiddenContext = createContext<Accessor<boolean>>(() => false);

export const WindowTabHiddenProvider = WindowTabHiddenContext.Provider;

/** True while this component's window tab is kept laid out but not shown. */
export function useWindowTabHidden(): Accessor<boolean> {
    return useContext(WindowTabHiddenContext);
}

/**
 * Fired on `window` after the displayed window tab changes. Native browser
 * panes composite above the DOM and only learn they're hidden or shown by
 * re-syncing their rect; hiding a tab doesn't change their placeholder's
 * geometry, so without this they'd wait for their 200 ms poll
 * (`use-pane-rect-sync.ts`; codex P2 on #3686).
 */
export const TAB_VISIBILITY_CHANGED_EVENT = "agentmux:tab-visibility-changed";

/** How a window tab's container hides or shows (`workspace.tsx`). */
export interface TabContainerVisibility {
    "content-visibility": "visible" | "hidden";
    visibility: "hidden" | null;
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
        "pointer-events": displayed ? "auto" : "none",
        hiddenLaidOut,
    };
}
