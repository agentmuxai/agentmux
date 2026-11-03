// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

/** Open the Kanban (this tab's, or a new one) with this task's drawer open. */
export function openKanbanTask(taskId: string): Promise<void> {
    const meta = { "kanban:open": taskId };
    return openOrFocusPaneByView("kanban", { meta: { view: "kanban", ...meta } }, meta);
}
