// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which Kanban task each agent pane holds, for other views (the Swarm's rows) to show and link to
// (SPEC_WARDEN_KANBAN_2026_10_20.md §6.14), and how many tasks need the operator, for the widget-bar
// Kanban button's badge (§6.13). One shared copy of the board, fetched while any chip or badge is
// mounted: on `workqueue:changed` and every 30 s (an agent of another AgentMux on this computer moves
// tasks without an event reaching this one).

import { createSignal, onCleanup, Show, type Accessor, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { useAccountEpisodes } from "@/app/store/account-availability";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import { needsCountOf } from "./kanban-columns";
import type { KanbanBoard, KanbanTask } from "./kanban-types";

import "./kanban-held.scss";

export interface HeldTask {
    id: string;
    num: number;
    title: string;
    state: string;
    /** Waiting on the operator: the agent asked through the task, or a question waits in its pane. */
    waiting: boolean;
}

const POLL_MS = 30_000;
const DEBOUNCE_MS = 300;

/** Each agent pane's task: claimed, or blocked on a question, by the agent registered on that block. */
export function heldByBlock(board: KanbanBoard): Map<string, HeldTask> {
    const out = new Map<string, HeldTask>();
    for (const t of board.tasks) {
        if (t.state !== "claimed" && t.state !== "blocked") continue;
        const agent =
            t.holder ??
            board.agents.find((a) => (t.claimed_by_uid ? a.uid === t.claimed_by_uid : a.name === t.claimed_by)) ??
            null;
        if (!agent) continue;
        const prev = out.get(agent.block_id);
        // An agent holding several: the one that moved last.
        if (!prev || t.updated_at > (board.tasks.find((x) => x.id === prev.id)?.updated_at ?? 0)) {
            out.set(agent.block_id, {
                id: t.id,
                num: t.card.num,
                title: t.title,
                state: t.state,
                waiting: t.state === "blocked" || agent.state === "waiting",
            });
        }
    }
    return out;
}

const [held, setHeld] = createSignal<Map<string, HeldTask>>(new Map());
const [boardTasks, setBoardTasks] = createSignal<KanbanTask[]>([]);
let users = 0;
let unsub: (() => void) | null = null;
let poll: ReturnType<typeof setInterval> | null = null;
let debounce: ReturnType<typeof setTimeout> | null = null;

async function refresh(): Promise<void> {
    try {
        const board = await RpcApi.KanbanListCommand(TabRpcClient, {});
        setHeld(heldByBlock(board));
        setBoardTasks(board.tasks);
    } catch {
        // An older srv, or none yet: no chips.
    }
}

function start(): void {
    void refresh();
    unsub = onMuxEvent(WpsEvent.WorkQueueChanged, () => {
        if (debounce) clearTimeout(debounce);
        debounce = setTimeout(() => void refresh(), DEBOUNCE_MS);
    });
    poll = setInterval(() => void refresh(), POLL_MS);
}

function stop(): void {
    unsub?.();
    unsub = null;
    if (poll) clearInterval(poll);
    if (debounce) clearTimeout(debounce);
    poll = debounce = null;
}

/** Keeps the shared board current while the calling component is mounted. */
function follow(): void {
    if (users++ === 0) start();
    onCleanup(() => {
        if (--users === 0) stop();
    });
}

/** The task this agent pane holds, kept current while the caller is mounted. */
export function useHeldTask(blockId: () => string): Accessor<HeldTask | null> {
    follow();
    return () => held().get(blockId()) ?? null;
}

/** How many tasks need the operator (the same count as the Warden tab title), kept current while the
 *  caller is mounted. An account episode counts once per account, however many agents it holds
 *  (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2); its `agent_ids` and `block_ids` say which holders it
 *  covers, so the Needs-you reason for a stuck task (#1847) skips those holders instead of counting them
 *  again. */
export function useKanbanNeeds(): Accessor<number> {
    follow();
    const episodes = useAccountEpisodes();
    return () => needsCountOf(boardTasks(), episodes()) + episodes().length;
}

/** Open the Kanban (this tab's Warden, or a new one) with this task's drawer open. */
export function openKanbanTask(taskId: string): Promise<void> {
    const meta = { "warden:section": "kanban" as const, "kanban:open": taskId };
    return openOrFocusPaneByView("warden", { meta: { view: "warden", ...meta } }, meta);
}

/** "#12 Write the spec" under an agent in the Swarm; opens the task on the Kanban. */
export function HeldTaskChip(props: { blockId: string }): JSX.Element {
    const task = useHeldTask(() => props.blockId);
    return (
        <Show when={task()}>
            {(t) => (
                <Button
                    density="compact"
                    tone={t().waiting ? "attention" : "quiet"}
                    icon="table-columns"
                    class="kanban-held-chip"
                    title={`${t().waiting ? "Waiting on your answer: " : "On task "}#${t().num} ${t().title} (open it on the Kanban)`}
                    onClick={(e) => {
                        e.stopPropagation();
                        void openKanbanTask(t().id);
                    }}
                >
                    <span class="kanban-held-chip__text">
                        #{t().num} {t().title}
                    </span>
                </Button>
            )}
        </Show>
    );
}
