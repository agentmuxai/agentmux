// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's document tabs as drag targets: moving a tab within its strip
 * (and, later, between Editor panes). Kept out of `editor-model.ts`, which is
 * at its size cap; it acts on the pane's store directly.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md.
 */

import { dispatch, snapshot } from "@/app/store/editor-pane-state-store";

/** Tab `tabId` of Editor `blockId` dragged to just before or after another.
 *  False when nothing moved (dropped where it already was). */
export function moveEditorTabTo(blockId: string, tabId: string, targetId: string, position: "before" | "after"): boolean {
    const before = snapshot(blockId)?.doc;
    dispatch(blockId, { type: "MoveTabTo", tabId, targetId, position, source: "user" });
    return snapshot(blockId)?.doc !== before;
}
