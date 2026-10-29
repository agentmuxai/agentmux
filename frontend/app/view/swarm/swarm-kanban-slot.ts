// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the Swarm passes the Kanban view that mounts in its Kanban section. The Swarm mounts it
// lazily the first time the section is shown and keeps it mounted; the view fills the slot
// (`.swarm-section-body`: a flex column child with a definite width and height) and measures its
// own width, so the Swarm needs no knowledge of its container.

export interface SwarmKanbanSlotProps {
    /** The section is shown: pauses polling when false. */
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
    /** Select that agent's row in the Swarm; without it, `revealBlock` as the Kanban does today. */
    onOpenAgent?(blockId: string): void;
}
