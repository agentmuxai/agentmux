// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The work on an open card (atlas-work.ts), read only while a card of one of this instance's agents is open:
 * srv's work facts (`GET /api/v1/work-facts`), asked again every `REFRESH_MS`, and the Kanban's board, followed
 * through the Kanban's own model. With no card open nothing is asked.
 */

import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { KanbanModel } from "../kanban/kanban-model";
import { srvList } from "./atlas-srv";
import { agentWork, type AtlasWork, type WorkFact } from "./atlas-work";

/** How often an open card asks for the work facts again: a branch changes rarely. */
export const REFRESH_MS = 20_000;

/** The work of the agent in pane `blockId()`, or null with no such card open or nothing known. */
export function useAtlasWork(blockId: Accessor<string | null>): Accessor<AtlasWork | null> {
    const [facts, setFacts] = createSignal<readonly WorkFact[]>([]);
    const kanban = new KanbanModel();
    let timer: ReturnType<typeof setInterval> | undefined;
    let disposed = false;
    const load = () => void srvList<WorkFact>("/api/v1/work-facts", "agents").then((list) => !disposed && setFacts(list));
    const stop = () => {
        clearInterval(timer);
        timer = undefined;
        kanban.setVisible(false);
    };
    const open = createMemo(() => blockId() !== null);
    createEffect(() => {
        if (!open()) return stop();
        kanban.setVisible(true);
        load();
        timer ??= setInterval(load, REFRESH_MS);
    });
    onCleanup(() => {
        disposed = true;
        stop();
    });
    return createMemo(() => {
        const id = blockId();
        return id ? agentWork(id, facts(), kanban.board()?.tasks ?? []) : null;
    });
}
