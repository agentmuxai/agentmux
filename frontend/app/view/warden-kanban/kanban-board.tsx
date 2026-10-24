// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's columns over a set of tasks (SPEC_WARDEN_KANBAN_2026_10_20.md §6.3;
// SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §4): the whole board, or one repository's lane of
// it (§6.11). Backlog and Spec'd grouped by the agent each task waits for, each group a drop target;
// untracked work in In progress; Review's load in its header; cancelled tasks folded under Merged.

import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { Button } from "@/app/element/ui";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { AgentChip, agentStateWords, KanbanCard, UntrackedCard } from "./kanban-card";
import {
    backlogGroups,
    COLUMNS,
    columnTasks,
    rankBetween,
    rankOf,
    reviewLoad,
    type BacklogGroup,
    type ColumnId,
} from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type { KanbanAgent, KanbanTask } from "./kanban-types";

/** A drop target for Kanban cards; `hover` shows it will take the card. */
export function useCardDrop(el: () => HTMLElement | undefined, onDrop: (id: string) => void): () => boolean {
    const [hover, setHover] = createSignal(false);
    onMount(() => {
        const element = el();
        if (!element) return;
        onCleanup(
            dropTargetForElements({
                element,
                canDrop: ({ source }) => source.data.type === kanbanCardItemType,
                onDragEnter: () => setHover(true),
                onDragLeave: () => setHover(false),
                onDrop: ({ source }) => {
                    setHover(false);
                    const id = source.data.id;
                    // After pragmatic-dnd finishes dispatching: the move re-renders the dragged card.
                    if (typeof id === "string") setTimeout(() => onDrop(id), 0);
                },
            })
        );
    });
    return hover;
}

function BacklogGroupView(props: {
    group: BacklogGroup;
    model: KanbanModel;
    now: number;
    agents: KanbanAgent[];
    colors?: Record<string, string>;
    holding: (agent: string) => KanbanTask | undefined;
    onOpen: (t: KanbanTask) => void;
}): JSX.Element {
    let el: HTMLDivElement | undefined;
    // Dropped here: the task waits for this group's agent (or anyone), at the end of its backlog.
    const hover = useCardDrop(
        () => el,
        (id) => {
            const last = props.group.tasks[props.group.tasks.length - 1];
            void props.model.act({
                action: "move",
                id,
                agent: props.group.key,
                rank: rankBetween(last ? rankOf(last) : undefined, undefined),
            });
        }
    );
    const busyOn = () => (props.group.key ? props.holding(props.group.key) : undefined);
    return (
        <div ref={el} class="kanban-group" classList={{ "kanban-drop--hover": hover() }}>
            <div class="kanban-group__head">
                <Show when={props.group.key} fallback={<span class="kanban-group__anyone">Anyone</span>}>
                    <AgentChip
                        name={props.group.label}
                        agent={props.group.agent}
                        uid={props.group.tasks[0]?.target_agent_uid}
                        colors={props.colors}
                    />
                </Show>
                <span class="warden-manager-dim">
                    {props.group.tasks.length} waiting
                    <Show when={busyOn()}>{(b) => <>, busy on #{b().card.num}</>}</Show>
                    <Show when={!busyOn() && props.group.agent}>{(a) => <>, {agentStateWords(a().state)}</>}</Show>
                </span>
            </div>
            <For each={props.group.tasks}>
                {(t) => (
                    <KanbanCard
                        task={t}
                        now={props.now}
                        agents={props.agents}
                        colors={props.colors}
                        onOpen={props.onOpen}
                    />
                )}
            </For>
        </div>
    );
}

export function KanbanBoard(props: {
    tasks: KanbanTask[];
    untracked: KanbanAgent[];
    agents: KanbanAgent[];
    model: KanbanModel;
    now: number;
    holding: (agent: string) => KanbanTask | undefined;
    onOpen: (t: KanbanTask) => void;
    onOpenAgent: (blockId: string) => void;
    /** Review's soft limit (`kanban:reviewlimit`): past it the header says so. 0: none. */
    reviewLimit: number;
    /** How many tasks the whole board has in Review, which the limit is about (a lane holds some). */
    reviewTotal: number;
    /** The colours of agents not running here, by uid (`agent_colors`). */
    colors?: Record<string, string>;
    /** The narrow layout (§8): one list, Merged and Released folded until opened. */
    narrow?: boolean;
}): JSX.Element {
    const groups = (column: ColumnId) => backlogGroups(props.tasks, props.agents, column);
    // In the narrow list, Merged and Released start folded: one header line each.
    const [unfolded, setUnfolded] = createSignal<ReadonlySet<ColumnId>>(new Set());
    const foldable = (id: ColumnId) => !!props.narrow && (id === "merged" || id === "released");
    const folded = (id: ColumnId) => foldable(id) && !unfolded().has(id);
    const toggle = (id: ColumnId) =>
        setUnfolded((s) => {
            const next = new Set(s);
            if (!next.delete(id)) next.add(id);
            return next;
        });
    const review = createMemo(() => reviewLoad(props.tasks, props.now));
    const over = () => props.reviewLimit > 0 && props.reviewTotal > props.reviewLimit;
    const [showCancelled, setShowCancelled] = createSignal(false);

    const columnHead = (id: ColumnId, label: string, count: number) => (
        <div
            class="kanban-column__head"
            classList={{ "kanban-column__head--foldable": foldable(id) }}
            role={foldable(id) ? "button" : undefined}
            tabindex={foldable(id) ? 0 : undefined}
            aria-expanded={foldable(id) ? !folded(id) : undefined}
            onClick={() => foldable(id) && toggle(id)}
            onKeyDown={(e) => {
                if (foldable(id) && (e.key === "Enter" || e.key === " ")) {
                    e.preventDefault();
                    toggle(id);
                }
            }}
        >
            <span class="kanban-column__label">
                <Show when={foldable(id)}>
                    <i class={`fa-solid fa-chevron-${folded(id) ? "right" : "down"}`} aria-hidden="true" />
                </Show>
                {label}
            </span>
            <span
                class="kanban-column__count"
                classList={{ "kanban-column__count--over": id === "review" && over() }}
                title={id === "review" && over() ? `More than ${props.reviewLimit} waiting for review` : undefined}
            >
                {count}
                <Show when={id === "review" && review().specs > 0}>
                    {" · "}
                    {review().specs} spec{review().specs === 1 ? "" : "s"}
                </Show>
                <Show when={id === "review" && review().count > 0}> · oldest {formatAge(review().oldestMs)}</Show>
                <Show when={id === "review" && over()}> · over {props.reviewLimit}</Show>
            </span>
        </div>
    );

    const card = (t: KanbanTask) => (
        <KanbanCard task={t} now={props.now} agents={props.agents} colors={props.colors} onOpen={props.onOpen} />
    );

    /** A waiting column: one group per agent, each a drop target. Backlog always shows Anyone, so a card
     *  can be given to anyone; Spec'd only the groups it has. */
    const waiting = (column: ColumnId) => (
        <div class="kanban-column__list">
            {/* Keyed by the group's agent name (a string), so a group, and its drop target, stays mounted
                when the board refreshes. */}
            <For each={groups(column).map((g) => g.key)}>
                {(key) => (
                    <Show when={groups(column).find((g) => g.key === key)}>
                        {(g) => (
                            <BacklogGroupView
                                group={g()}
                                model={props.model}
                                now={props.now}
                                agents={props.agents}
                                colors={props.colors}
                                holding={props.holding}
                                onOpen={props.onOpen}
                            />
                        )}
                    </Show>
                )}
            </For>
            <Show when={column === "backlog" && !groups(column).some((g) => g.key === "")}>
                <BacklogGroupView
                    group={{ key: "", label: "Anyone", agent: undefined, tasks: [] }}
                    model={props.model}
                    now={props.now}
                    agents={props.agents}
                    holding={props.holding}
                    onOpen={props.onOpen}
                />
            </Show>
        </div>
    );

    return (
        <div class="kanban-board">
            <For each={COLUMNS}>
                {(col) => {
                    const tasks = () => columnTasks(props.tasks, col.id);
                    const finished = () =>
                        col.id === "merged" ? tasks().filter((t) => t.state !== "cancelled") : tasks();
                    const cancelled = () => (col.id === "merged" ? tasks().filter((t) => t.state === "cancelled") : []);
                    return (
                        <section
                            class="kanban-column"
                            classList={{
                                [`kanban-column--${col.id}`]: true,
                                "kanban-column--empty":
                                    finished().length + cancelled().length === 0 &&
                                    !(col.id === "progress" && props.untracked.length > 0) &&
                                    col.id !== "backlog",
                                "kanban-column--folded": folded(col.id),
                            }}
                            aria-label={col.label}
                        >
                            {columnHead(
                                col.id,
                                col.label,
                                finished().length + (col.id === "progress" ? props.untracked.length : 0)
                            )}
                            <Show when={col.id !== "backlog" && col.id !== "specd"} fallback={waiting(col.id)}>
                                <div class="kanban-column__list">
                                    <Show when={col.id === "progress"}>
                                        <For each={props.untracked}>
                                            {(a) => (
                                                <UntrackedCard
                                                    agent={a}
                                                    onTrack={(ag) =>
                                                        void props.model.act({ action: "track", block_id: ag.block_id })
                                                    }
                                                    onOpenAgent={(ag) => props.onOpenAgent(ag.block_id)}
                                                />
                                            )}
                                        </For>
                                    </Show>
                                    <For each={finished()}>{card}</For>
                                    <Show when={cancelled().length > 0}>
                                        <Button
                                            density="compact"
                                            tone="quiet"
                                            icon={showCancelled() ? "chevron-down" : "chevron-right"}
                                            class="kanban-column__fold"
                                            aria-expanded={showCancelled()}
                                            onClick={() => setShowCancelled(!showCancelled())}
                                        >
                                            Cancelled · {cancelled().length}
                                        </Button>
                                        <Show when={showCancelled()}>
                                            <For each={cancelled()}>{card}</For>
                                        </Show>
                                    </Show>
                                </div>
                            </Show>
                        </section>
                    );
                }}
            </For>
        </div>
    );
}
