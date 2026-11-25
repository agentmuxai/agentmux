// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing a floating window whose tab has no pane left.
 *
 * The floater closes itself from a reactive watcher on its tab (floating-pane-workspace.tsx), but that runs inside
 * the update that empties the tab, which can throw while the pane's views dispose (Solid then drops every effect
 * queued in that update). So a redock that has returned does the close itself, outside the update, and a timer
 * catches anything that still slips through.
 */

import * as MOS from "@/app/store/mos";
import { getApi } from "@/store/global";

/** Whether the tab, read plainly (no reactive graph), holds no block. `null`: not known yet. */
export function tabIsEmpty(tabId: string | null | undefined): boolean | null {
    if (!tabId) return null;
    const blockids = MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabId))?.blockids;
    return blockids ? blockids.length === 0 : null;
}

/** Close the floating window `label` when its tab `tabId` is empty now; a window that still holds a pane stays. */
export function closeFloaterIfEmpty(tabId: string | null | undefined, label: string, why: string): boolean {
    if (!label || tabIsEmpty(tabId) !== true) return false;
    console.log("[floating-pane] closing a floating window whose pane has left it", { label, tabId, why });
    getApi()
        .closeWindowByLabel(label)
        .catch((e) => console.error("[floating-pane] close of an empty floating window failed", e));
    return true;
}
