// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Kanban section: what work there is and where it stands, kept current by the agents doing it
// (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md). Eight columns
// (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md): Backlog and Spec'd (grouped by the agent each
// task waits for), In Research, In progress, Needs you, Review (its PR in review), Merged, Released.
// Agents file and move tasks with the `Tasks` tool; srv moves them on what it knows (an agent asks,
// errors, closes; a PR merges; a release ships it); the operator starts, answers, accepts and sends back
// here.

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button, FilterInput, IconButton, Select, Switch } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";
import { useAccountEpisodes } from "@/app/store/account-availability";
import { settingsAtom } from "@/app/store/config-signals";
import { AccountAvailabilityStrip } from "@/app/view/account-availability/account-availability-strip";
import { revealBlock } from "@/app/util/reveal-block";

import { KanbanBoard, useCardDrop } from "./kanban-board";
import { AgentChip, agentStateWords } from "./kanban-card";
import {
    columnOf,
    isFinishedColumn,
    KANBAN_NARROW_PX,
    matchesQuery,
    repoLabel,
    repoLanes,
    reviewLimitOf,
    untrackedAgents,
} from "./kanban-columns";
import { KanbanDrawer } from "./kanban-drawer";
import { KanbanModel } from "./kanban-model";
import { KanbanNewTask } from "./kanban-new-task";
import { KANBAN_REMINDERS_DESCRIPTION, kanbanRemindersOn, setKanbanReminders } from "./kanban-reminders";
import type { KanbanAgent, KanbanTask } from "./kanban-types";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./kanban.scss";

function AgentStripChip(props: { agent: KanbanAgent; model: KanbanModel }): JSX.Element {
    let el: HTMLButtonElement | undefined;
    // A waiting task dropped on an agent starts there now.
    const hover = useCardDrop(
        () => el,
        (id) => void props.model.act({ action: "start", id, agent: props.agent.name })
    );
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
    /** One lane per repository (the pane's `kanban:lanes` meta), and its setter. */
    lanes?: boolean;
    onLanes?: (on: boolean) => void;
}): JSX.Element => {
    const model = new KanbanModel();
    onCleanup(() => model.dispose());
    createEffect(() => model.setVisible(props.visible));
    // An account episode counts once (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2).
    const accountEpisodes = useAccountEpisodes();
    createEffect(() => props.onNeedsCount?.(model.needsCount() + accountEpisodes().length));

    const tick = useTick(10_000);
    // The narrow layout (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §8): Warden's container at
    // KANBAN_NARROW_PX or less, as `kanban.scss`'s container query; the board folds Merged and Released.
    let root: HTMLDivElement | undefined;
    const [narrow, setNarrow] = createSignal(false);
    onMount(() => {
        if (!root) return;
        // The element `kanban.scss`'s container query measures (Warden's container), not this padded body.
        const measured = root.closest<HTMLElement>(".warden-container") ?? root;
        const observer = new ResizeObserver(([entry]) => setNarrow(entry.contentRect.width <= KANBAN_NARROW_PX));
        observer.observe(measured);
        onCleanup(() => observer.disconnect());
    });
    const now = () => (tick(), Date.now());
    const [query, setQuery] = createSignal("");
    const [repo, setRepo] = createSignal("");
    const [showUntracked, setShowUntracked] = createSignal(true);
    const [adding, setAdding] = createSignal(false);
    const [openId, setOpenId] = createSignal<string | null>(null);
    const [ownLanes, setOwnLanes] = createSignal(false);
    const lanes = () => props.lanes ?? ownLanes();
    const setLanes = (on: boolean) => (props.onLanes ? props.onLanes(on) : setOwnLanes(on));
    const [folded, setFolded] = createSignal<ReadonlySet<string>>(new Set());
    const fold = (repo: string) =>
        setFolded((f) => {
            const next = new Set(f);
            if (!next.delete(repo)) next.add(repo);
            return next;
        });
    const reviewLimit = () => reviewLimitOf(settingsAtom() as unknown as Record<string, unknown>);

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
    const laneList = createMemo(() => repoLanes(tasks()));
    // The soft limit is on the board's whole Review column: every lane's, and those a filter hides.
    const reviewTotal = createMemo(() => (board()?.tasks ?? []).filter((t) => columnOf(t) === "review").length);
    const open = (t: KanbanTask) => setOpenId(t.id);
    const openAgent = (blockId: string) => void revealBlock(blockId, { source: "kanban" });

    return (
        <div ref={root} class="warden-manager-body kanban">
            <AccountAvailabilityStrip />
            <div class="kanban-toolbar">
                <FilterInput
                    value={query()}
                    onInput={setQuery}
                    placeholder="Find a task, #number, agent or repository"
                    ariaLabel="Find a task"
                />
                <Show when={repos().length > 1}>
                    <Select
                        density="compact"
                        ariaLabel="Repository"
                        value={repo()}
                        onChange={setRepo}
                        options={[
                            { value: "", label: "All repositories" },
                            ...repos().map((r) => ({ value: r, label: r })),
                        ]}
                    />
                </Show>
                <label class="kanban-toolbar__switch" title="One row of columns per repository">
                    <Switch checked={lanes()} onChange={setLanes} density="compact" ariaLabel="Lanes by repository" />
                    Lanes
                </label>
                <label
                    class="kanban-toolbar__switch"
                    title="Agents working without a task show as dashed cards in In progress"
                >
                    <Switch
                        checked={showUntracked()}
                        onChange={setShowUntracked}
                        density="compact"
                        ariaLabel="Show untracked work"
                    />
                    Untracked
                </label>
                <label class="kanban-toolbar__switch" title={KANBAN_REMINDERS_DESCRIPTION}>
                    <Switch
                        checked={kanbanRemindersOn(settingsAtom() as unknown as Record<string, unknown>)}
                        onChange={setKanbanReminders}
                        density="compact"
                        ariaLabel="Remind idle agents of their backlog"
                    />
                    Reminders
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

            <Show
                when={board()}
                fallback={<p class="warden-manager-dim">{model.error() ? "" : "Loading the Kanban…"}</p>}
            >
                <Show
                    when={lanes()}
                    fallback={
                        <KanbanBoard
                            tasks={tasks()}
                            untracked={untracked()}
                            agents={agents()}
                            model={model}
                            now={now()}
                            holding={holding}
                            onOpen={open}
                            onOpenAgent={openAgent}
                            reviewLimit={reviewLimit()}
                            reviewTotal={reviewTotal()}
                            colors={board()?.agent_colors}
                            narrow={narrow()}
                        />
                    }
                >
                    <div class="kanban-lanes">
                        {/* Untracked work belongs to no task, so to no lane: its own row, In progress only. */}
                        <Show when={untracked().length > 0}>
                            <section class="kanban-lane" aria-label="Agents working without a task">
                                <div class="kanban-lane__head">
                                    <span class="kanban-lane__name">Working without a task</span>
                                    <span class="warden-manager-dim">{untracked().length}</span>
                                </div>
                                <KanbanBoard
                                    tasks={[]}
                                    untracked={untracked()}
                                    agents={agents()}
                                    model={model}
                                    now={now()}
                                    holding={holding}
                                    onOpen={open}
                                    onOpenAgent={openAgent}
                                    reviewLimit={reviewLimit()}
                                    reviewTotal={reviewTotal()}
                                    colors={board()?.agent_colors}
                                    narrow={narrow()}
                                />
                            </section>
                        </Show>
                        {/* Keyed by repository (a string), so a lane stays mounted when the board refreshes. */}
                        <For each={laneList().map((l) => l.repo)}>
                            {(repo) => (
                                <Show when={laneList().find((l) => l.repo === repo)}>
                                    {(lane) => (
                                        <section class="kanban-lane" aria-label={repoLabel(repo)}>
                                            <Button
                                                density="compact"
                                                tone="quiet"
                                                icon={folded().has(repo) ? "chevron-right" : "chevron-down"}
                                                class="kanban-lane__head"
                                                aria-expanded={!folded().has(repo)}
                                                onClick={() => fold(repo)}
                                            >
                                                <span class="kanban-lane__name">{lane().label}</span>
                                                <span class="warden-manager-dim">
                                                    {lane().tasks.filter((t) => !isFinishedColumn(columnOf(t))).length}{" "}
                                                    open
                                                    {" · "}
                                                    {lane().tasks.filter((t) => columnOf(t) === "needs").length} need
                                                    you
                                                </span>
                                            </Button>
                                            <Show when={!folded().has(repo)}>
                                                <KanbanBoard
                                                    tasks={lane().tasks}
                                                    untracked={[]}
                                                    agents={agents()}
                                                    model={model}
                                                    now={now()}
                                                    holding={holding}
                                                    onOpen={open}
                                                    onOpenAgent={openAgent}
                                                    reviewLimit={reviewLimit()}
                                                    reviewTotal={reviewTotal()}
                                                    colors={board()?.agent_colors}
                                                    narrow={narrow()}
                                                />
                                            </Show>
                                        </section>
                                    )}
                                </Show>
                            )}
                        </For>
                    </div>
                </Show>
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
