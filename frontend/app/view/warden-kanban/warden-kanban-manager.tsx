// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Kanban section: what work there is and where it stands, kept current by the agents doing it
// (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md). Five columns: Backlog (grouped by the agent each
// task waits for), In progress, Needs you, Review, Done. Agents file and move tasks with the `Tasks`
// tool; srv moves them on what it knows (an agent asks, errors, closes); the operator starts, answers,
// accepts and sends back here.

import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { useTick } from "@/app/hook/useTick";
import { Button, FilterInput, IconButton, Select, Switch } from "@/app/element/ui";
import { revealBlock } from "@/app/util/reveal-block";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { AgentChip, agentStateWords, KanbanCard, UntrackedCard } from "./kanban-card";
import {
    backlogGroups,
    columnTasks,
    COLUMNS,
    matchesQuery,
    rankBetween,
    rankOf,
    reviewLoad,
    untrackedAgents,
    type BacklogGroup,
    type ColumnId,
} from "./kanban-columns";
import { KanbanDrawer } from "./kanban-drawer";
import { KanbanModel } from "./kanban-model";
import { KanbanNewTask } from "./kanban-new-task";
import type { KanbanAgent, KanbanTask } from "./kanban-types";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./kanban.scss";

/** A drop target for Kanban cards; `hover` shows it will take the card. */
function useCardDrop(el: () => HTMLElement | undefined, onDrop: (id: string) => void): () => boolean {
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
    holding: (agent: string) => KanbanTask | undefined;
    onOpen: (t: KanbanTask) => void;
}): JSX.Element {
    let el: HTMLDivElement | undefined;
    // Dropped here: the task waits for this group's agent (or anyone), at the end of its backlog.
    const hover = useCardDrop(
        () => el,
        (id) => {
            const last = props.group.tasks[props.group.tasks.length - 1];
            void props.model.act({ action: "move", id, agent: props.group.key, rank: rankBetween(last ? rankOf(last) : undefined, undefined) });
        }
    );
    const busyOn = () => (props.group.key ? props.holding(props.group.key) : undefined);
    return (
        <div ref={el} class="kanban-group" classList={{ "kanban-drop--hover": hover() }}>
            <div class="kanban-group__head">
                <Show when={props.group.key} fallback={<span class="kanban-group__anyone">Anyone</span>}>
                    <AgentChip name={props.group.label} agent={props.group.agent} />
                </Show>
                <span class="warden-manager-dim">
                    {props.group.tasks.length} waiting
                    <Show when={busyOn()}>{(b) => <>, busy on #{b().card.num}</>}</Show>
                    <Show when={!busyOn() && props.group.agent}>{(a) => <>, {agentStateWords(a().state)}</>}</Show>
                </span>
            </div>
            <For each={props.group.tasks}>{(t) => <KanbanCard task={t} now={props.now} agents={props.agents} onOpen={props.onOpen} />}</For>
        </div>
    );
}

function AgentStripChip(props: { agent: KanbanAgent; model: KanbanModel }): JSX.Element {
    let el: HTMLButtonElement | undefined;
    // A waiting task dropped on an agent starts there now.
    const hover = useCardDrop(() => el, (id) => void props.model.act({ action: "start", id, agent: props.agent.name }));
    return (
        <Button
            ref={el}
            density="compact"
            tone="quiet"
            class={hover() ? "kanban-strip__agent kanban-drop--hover" : "kanban-strip__agent"}
            title={`${props.agent.name}: ${agentStateWords(props.agent.state)}. Drop a waiting task here to start it on ${props.agent.name}; click to open it.`}
            onClick={() => void revealBlock(props.agent.block_id, { source: "kanban" })}
        >
            <AgentChip name={props.agent.name} agent={props.agent} />
        </Button>
    );
}

export const WardenKanbanManager = (props: {
    visible: boolean;
    onNeedsCount?: (n: number) => void;
    /** A task to open, asked for from elsewhere (the Swarm's task chip); cleared with onOpenHandled. */
    openRequest?: string | null;
    onOpenHandled?: () => void;
}): JSX.Element => {
    const model = new KanbanModel();
    onCleanup(() => model.dispose());
    createEffect(() => model.setVisible(props.visible));
    createEffect(() => props.onNeedsCount?.(model.needsCount()));

    const tick = useTick(10_000);
    const now = () => (tick(), Date.now());
    const [query, setQuery] = createSignal("");
    const [repo, setRepo] = createSignal("");
    const [showUntracked, setShowUntracked] = createSignal(true);
    const [adding, setAdding] = createSignal(false);
    const [openId, setOpenId] = createSignal<string | null>(null);

    // Opened from elsewhere: show the task once the board has it, then clear the request.
    createEffect(() => {
        const id = props.openRequest;
        if (!id || !model.board()?.tasks.some((t) => t.id === id)) return;
        setOpenId(id);
        props.onOpenHandled?.();
    });

    const board = () => model.board();
    const agents = () => board()?.agents ?? [];
    const repos = createMemo(() => [...new Set((board()?.tasks ?? []).map((t) => t.card.repo).filter(Boolean))].sort());
    const tasks = createMemo(() =>
        (board()?.tasks ?? []).filter((t) => (!repo() || t.card.repo === repo()) && matchesQuery(t, query()))
    );
    const openTask = () => (board()?.tasks ?? []).find((t) => t.id === openId()) ?? null;
    const holding = (agent: string) =>
        (board()?.tasks ?? []).find((t) => (t.state === "claimed" || t.state === "blocked") && t.claimed_by === agent);
    const untracked = createMemo(() => (showUntracked() ? untrackedAgents(agents(), board()?.tasks ?? []) : []));
    const groups = createMemo(() => backlogGroups(tasks(), agents()));
    const review = createMemo(() => reviewLoad(tasks(), now()));
    const open = (t: KanbanTask) => setOpenId(t.id);
    const openAgent = (blockId: string) => void revealBlock(blockId, { source: "kanban" });

    const columnHead = (id: ColumnId, label: string, count: number) => (
        <div class="kanban-column__head">
            <span class="kanban-column__label">{label}</span>
            <span class="kanban-column__count">
                {count}
                <Show when={id === "review" && review().count > 0}> · oldest {formatAge(review().oldestMs)}</Show>
            </span>
        </div>
    );

    return (
        <div class="warden-manager-body kanban">
            <div class="kanban-toolbar">
                <FilterInput value={query()} onInput={setQuery} placeholder="Find a task, #number, agent or repository" ariaLabel="Find a task" />
                <Show when={repos().length > 1}>
                    <Select
                        density="compact"
                        ariaLabel="Repository"
                        value={repo()}
                        onChange={setRepo}
                        options={[{ value: "", label: "All repositories" }, ...repos().map((r) => ({ value: r, label: r }))]}
                    />
                </Show>
                <label class="kanban-toolbar__switch" title="Agents working without a task show as dashed cards in In progress">
                    <Switch checked={showUntracked()} onChange={setShowUntracked} density="compact" ariaLabel="Show untracked work" />
                    Untracked
                </label>
                <Button density="compact" tone="accent" icon="plus" onClick={() => setAdding(true)}>
                    New task
                </Button>
                <IconButton icon="rotate" label="Refresh" onClick={() => void model.refresh()} />
            </div>

            <Show when={model.error()}>
                {(e) => (
                    <div class="warden-manager-error kanban-error" role="alert">
                        {e()}
                        <IconButton icon="xmark" label="Dismiss" onClick={() => model.dismissError()} />
                    </div>
                )}
            </Show>

            <Show when={adding()}>
                <KanbanNewTask model={model} agents={agents()} onDone={() => setAdding(false)} />
            </Show>

            <Show when={agents().length > 0}>
                <div class="kanban-strip" aria-label="Agents">
                    <For each={agents()}>{(a) => <AgentStripChip agent={a} model={model} />}</For>
                </div>
            </Show>

            <Show when={board()} fallback={<p class="warden-manager-dim">{model.error() ? "" : "Loading the Kanban…"}</p>}>
                <div class="kanban-board">
                    <For each={COLUMNS}>
                        {(col) => (
                            <section class="kanban-column" classList={{ [`kanban-column--${col.id}`]: true }} aria-label={col.label}>
                                <Show
                                    when={col.id === "backlog"}
                                    fallback={
                                        <>
                                            {columnHead(col.id, col.label, columnTasks(tasks(), col.id).length + (col.id === "progress" ? untracked().length : 0))}
                                            <div class="kanban-column__list">
                                                <Show when={col.id === "progress"}>
                                                    <For each={untracked()}>
                                                        {(a) => (
                                                            <UntrackedCard
                                                                agent={a}
                                                                onTrack={(ag) => void model.act({ action: "track", block_id: ag.block_id })}
                                                                onOpenAgent={(ag) => openAgent(ag.block_id)}
                                                            />
                                                        )}
                                                    </For>
                                                </Show>
                                                <For each={columnTasks(tasks(), col.id)}>
                                                    {(t) => <KanbanCard task={t} now={now()} agents={agents()} onOpen={open} />}
                                                </For>
                                            </div>
                                        </>
                                    }
                                >
                                    {columnHead(col.id, col.label, columnTasks(tasks(), "backlog").length)}
                                    <div class="kanban-column__list">
                                        {/* Keyed by the group's agent name (a string), so a group, and its drop
                                            target, stays mounted when the board refreshes. */}
                                        <For each={groups().map((g) => g.key)}>
                                            {(key) => (
                                                <Show when={groups().find((g) => g.key === key)}>
                                                    {(g) => <BacklogGroupView group={g()} model={model} now={now()} agents={agents()} holding={holding} onOpen={open} />}
                                                </Show>
                                            )}
                                        </For>
                                        <Show when={!groups().some((g) => g.key === "")}>
                                            <BacklogGroupView
                                                group={{ key: "", label: "Anyone", agent: undefined, tasks: [] }}
                                                model={model}
                                                now={now()}
                                                agents={agents()}
                                                holding={holding}
                                                onOpen={open}
                                            />
                                        </Show>
                                    </div>
                                </Show>
                            </section>
                        )}
                    </For>
                </div>
            </Show>

            <Show when={openTask()}>
                {(t) => (
                    <KanbanDrawer
                        model={model}
                        task={t()}
                        agents={agents()}
                        now={now()}
                        onClose={() => setOpenId(null)}
                        onOpenAgent={openAgent}
                        onOpenTask={(id) => setOpenId(id)}
                    />
                )}
            </Show>
        </div>
    );
};
