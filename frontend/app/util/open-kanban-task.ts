// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { asGesture } from "@/app/store/trail-hooks";

/**
 * Open the Kanban (this tab's, or a new one) with this task's drawer open. Every way in (a link in a message, a held
 * chip, the Swarm) comes through here, so the window's trail records it as one step: the pane and the card.
 */
export function openKanbanTask(taskId: string): Promise<void> {
    const meta = { "kanban:open": taskId };
    return asGesture(() => openOrFocusPaneByView("kanban", { meta: { view: "kanban", ...meta } }, meta));
}
