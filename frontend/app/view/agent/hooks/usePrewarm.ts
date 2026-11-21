// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Lets an agent pane in a hidden window tab be drawn ahead of its visit (store/agent-prewarm.ts): before the tab's
 * first visit while the window is idle, and, on intent, when the pointer rests on the tab's pill. Returns whether
 * the pane holds the lease now.
 *
 * `changes` is what the pane's document is now (a new value whenever it changes). The hook keeps what it was when the
 * pane last stopped drawing (its tab hidden, or its lease ended), so an intent draw can tell a pane that holds
 * what it has now from one that does not.
 */
import { usePrewarmLease } from "@/app/store/agent-prewarm";
import { isBlockDormant } from "@/app/store/block-component-registry";
import { tabWasShown } from "@/app/store/tab-reveal";
import { agentPrewarm } from "@/app/workspace/agent-prewarm-driver";
import { useWindowTabDisplayed, useWindowTabId } from "@/app/workspace/window-tab-visibility";
import { createEffect, on, onCleanup, untrack, type Accessor } from "solid-js";

/** Nothing drawn yet: differs from every document. */
const NEVER_DRAWN = Symbol("never-drawn");

export function usePrewarm(blockId: string, changes?: () => unknown): Accessor<boolean> {
    const displayed = useWindowTabDisplayed();
    const tabId = useWindowTabId();
    const behindAnotherPaneTab = isBlockDormant(blockId);
    const leased = usePrewarmLease(blockId);
    let drawn: unknown = NEVER_DRAWN;
    // The pane draws while its tab is displayed or it holds the lease; what it held when that ends is what it drew.
    createEffect(
        on(
            () => displayed() || leased(),
            (drawing) => {
                if (!drawing && changes) drawn = untrack(changes);
            },
            { defer: true }
        )
    );
    const remove = agentPrewarm.add({
        blockId,
        tabId,
        // Not for a pane kept alive behind another pane tab: it has nothing to show on a tab's first visit.
        tabSeen: () => displayed() || behindAnotherPaneTab() || (tabId != null && tabWasShown(tabId)),
        nodeCount: () => document.querySelector(`.pane-stack[data-blockid="${blockId}"]`)?.getElementsByTagName("*").length ?? 0,
        drawable: () => !displayed() && !behindAnotherPaneTab(),
        stale: () => changes != null && changes() !== drawn,
    });
    onCleanup(remove);
    return leased;
}
