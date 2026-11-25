// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Atlas reads from srv and the Kanban: the Kanban's tasks while the map is open (the nodes list them,
 * atlas-node-tasks.ts), and srv's work facts (`GET /api/v1/work-facts`) while a card of one of this instance's agents is
 * open, asked again every `REFRESH_MS`. The tasks are followed through the Kanban's own model.
 */

import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { KanbanModel } from "../kanban/kanban-model";
import type { KanbanTask } from "../kanban/kanban-types";
import { srvList } from "./atlas-srv";
import { agentWork, type AtlasWork, type TaskOwner, type WorkFact } from "./atlas-work";

/** How often an open card asks for the work facts again: a branch changes rarely. */
export const REFRESH_MS = 20_000;

/** The Kanban's tasks, followed for as long as the map is open (the Kanban's own refresh: its change event, and a poll). */
export function useAtlasTasks(): Accessor<readonly KanbanTask[]> {
    const kanban = new KanbanModel();
    kanban.setVisible(true);
    onCleanup(() => kanban.setVisible(false));
    return () => kanban.board()?.tasks ?? [];
}

/** The work of `agent`, the one whose card is open, or null with no card open or nothing known. */
export function useAtlasWork(
    agent: Accessor<(TaskOwner & { blockId: string | null }) | null>,
    tasks: Accessor<readonly KanbanTask[]>
): Accessor<AtlasWork | null> {
    const [facts, setFacts] = createSignal<readonly WorkFact[]>([]);
    let timer: ReturnType<typeof setInterval> | undefined;
    let disposed = false;
    const load = () =>
        void srvList<WorkFact>("/api/v1/work-facts", "agents").then((list) => !disposed && setFacts(list));
    const stop = () => {
        clearInterval(timer);
        timer = undefined;
    };
    const open = createMemo(() => agent()?.blockId != null);
    createEffect(() => {
        if (!open()) return stop();
        load();
        timer ??= setInterval(load, REFRESH_MS);
    });
    onCleanup(() => {
        disposed = true;
        stop();
    });
    return createMemo(() => {
        const who = agent();
        return who?.blockId ? agentWork(who.blockId, facts(), tasks(), who) : null;
    });
}
