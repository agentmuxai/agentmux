// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the Kanban pane passes the board (kanban-instance.tsx). The board fills its container
// (`.kanban-pane`: a flex column with a definite width and height) and measures its own width, so the
// pane needs no knowledge of it.

export interface KanbanSectionProps {
    /** The pane is shown: pauses polling when false. */
    visible: boolean;
    /** A task id to open (the held-task chip asks for it, `kanban:open` in the pane's meta). */
    openRequest: string | null;
    onOpenHandled(): void;
    /** Feeds the title and the Needs-you badge. */
    onNeedsCount(n: number): void;
    /** Pane meta `kanban:lanes`: the lanes view on or off. */
    lanes: boolean;
    onLanes(on: boolean): void;
    /** Pane meta `kanban:collapsed`: the columns folded to a strip, by id. */
    collapsed: string[];
    onCollapsed(ids: string[]): void;
    /** Where a card's agent opens; without it the agent's own pane is revealed (`revealBlock`). */
    onOpenAgent?(blockId: string): void;
}
