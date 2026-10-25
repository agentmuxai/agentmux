// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Lets an agent pane in a hidden window tab be drawn ahead of its first visit
 * (store/agent-prewarm.ts). Returns whether the pane holds the lease now.
 */
import { usePrewarmLease } from "@/app/store/agent-prewarm";
import { tabWasShown } from "@/app/store/tab-reveal";
import { agentPrewarm } from "@/app/workspace/agent-prewarm-driver";
import { useWindowTabDisplayed, useWindowTabId } from "@/app/workspace/window-tab-visibility";
import { onCleanup, type Accessor } from "solid-js";

export function usePrewarm(blockId: string): Accessor<boolean> {
    const displayed = useWindowTabDisplayed();
    const tabId = useWindowTabId();
    const remove = agentPrewarm.add({
        blockId,
        tabId,
        tabSeen: () => displayed() || (tabId != null && tabWasShown(tabId)),
        nodeCount: () => document.querySelector(`.pane-stack[data-blockid="${blockId}"]`)?.getElementsByTagName("*").length ?? 0,
    });
    onCleanup(remove);
    return usePrewarmLease(blockId);
}
