// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's Kanban section (it moved here from Warden, SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §4):
// what work there is and where it stands, kept current by the agents doing it
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
import { revealBlock } from "@/app/util/reveal-block";

import { KanbanBoard, useCardDrop } from "./kanban-board";
import { AgentChip, agentStateWords } from "./kanban-card";
import {
    columnOf,
    isFinishedColumn,
    KANBAN_NARROW_PX,
    matchesQuery,
    needsCountOf,
    parseCollapsed,
    repoLabel,
    repoLanes,
    reviewLimitOf,
    toggleCollapsed,
    untrackedAgents,
    type ColumnId,
} from "./kanban-columns";
import { KanbanDrawer } from "./kanban-drawer";
import { KanbanModel } from "./kanban-model";
import { KanbanNewTask } from "./kanban-new-task";
import { KANBAN_REMINDERS_DESCRIPTION, kanbanRemindersOn, setKanbanReminders } from "./kanban-reminders";
import { reportsTo, ReportsToSelect, seatOf, taskReportsTo, useSeatIndex } from "./kanban-seats";
import type { KanbanAgent, KanbanTask } from "./kanban-types";
import type { SwarmKanbanSlotProps } from "@/app/view/swarm/swarm-kanban-slot";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./kanban.scss";

function AgentStripChip(props: { agent: KanbanAgent; model: KanbanModel; onOpenAgent: (blockId: string) => void }): JSX.Element {
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
            onClick={() => props.onOpenAgent(props.agent.block_id)}
        >
            <AgentChip name={props.agent.name} agent={props.agent} />
        </Button>
    );
}

/** What the Swarm passes (swarm-kanban-slot.ts); all but `visible` are optional, so the board also mounts alone. */
export const KanbanSection = (props: Pick<SwarmKanbanSlotProps, "visible"> & Partial<Omit<SwarmKanbanSlotProps, "visible">>): JSX.Element => {
    const model = new KanbanModel();
    onCleanup(() => model.dispose());
    createEffect(() => model.setVisible(props.visible));
    // An account episode counts once (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2).
    const accountEpisodes = useAccountEpisodes();
    createEffect(() => props.onNeedsCount?.(needsCountOf(model.board()?.tasks ?? [], accountEpisodes()) + accountEpisodes().length));

    const tick = useTick(10_000);
    // The narrow layout (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §8): this section's own width
    // at KANBAN_NARROW_PX or less, as `kanban.scss`'s container query; the board folds Merged and Released.
    // The section is the named container (`.kanban-container`), so the Swarm needs no knowledge of it.
    let container: HTMLDivElement | undefined;
    const [narrow, setNarrow] = createSignal(false);
    onMount(() => {
        if (!container) return;
        const observer = new ResizeObserver(([entry]) => setNarrow(entry.contentRect.width <= KANBAN_NARROW_PX));
        observer.observe(container);
        onCleanup(() => observer.disconnect());
    });
    const now = () => (tick(), Date.now());
    const [query, setQuery] = createSignal("");
    const [repo, setRepo] = createSignal("");
    // "Reports to …" (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4): the owner's seat names its lead.
    const [lead, setLead] = createSignal("");
    const seats = useSeatIndex();
    const [showUntracked, setShowUntracked] = createSignal(true);
    const [adding, setAdding] = createSignal(false);
    const [openId, setOpenId] = createSignal<string | null>(null);
    const [ownLanes, setOwnLanes] = createSignal(false);
    const lanes = () => props.lanes ?? ownLanes();
    const setLanes = (on: boolean) => (props.onLanes ? props.onLanes(on) : setOwnLanes(on));
    // Columns collapsed to a strip: the pane's (it survives a restart), or this section's own when it
    // mounts alone.
    const [ownCollapsed, setOwnCollapsed] = createSignal<ColumnId[]>([]);
    const collapsed = () => parseCollapsed(props.collapsed ?? ownCollapsed());
    const toggleColumn = (id: ColumnId) => {
        const next = toggleCollapsed(collapsed(), id);
        if (props.onCollapsed) props.onCollapsed(next);
        else setOwnCollapsed(next);
    };
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
        (board()?.tasks ?? []).filter(
            (t) => (!repo() || t.card.repo === repo()) && matchesQuery(t, query()) && taskReportsTo(seats(), t, lead())
        )
    );
    const openTask = () => (board()?.tasks ?? []).find((t) => t.id === openId()) ?? null;
    const holding = (agent: string) =>
        (board()?.tasks ?? []).find((t) => (t.state === "claimed" || t.state === "blocked") && t.claimed_by === agent);
    const untracked = createMemo(() =>
        showUntracked()
            ? untrackedAgents(agents(), board()?.tasks ?? []).filter((a) =>
                  reportsTo(seatOf(seats(), a.uid, a.name), lead())
              )
            : []
    );
    const laneList = createMemo(() => repoLanes(tasks()));
    // The soft limit is on the board's whole Review column: every lane's, and those a filter hides.
    const reviewTotal = createMemo(() => (board()?.tasks ?? []).filter((t) => columnOf(t) === "review").length);
    const open = (t: KanbanTask) => setOpenId(t.id);
    // "Go to this agent": in the Swarm, its row is selected; on its own, the agent is revealed where it is.
    const openAgent = (blockId: string) =>
        props.onOpenAgent ? props.onOpenAgent(blockId) : void revealBlock(blockId, { source: "kanban" });

    return (
        <div ref={container} class="kanban-container">
            <div class="warden-manager-body kanban">
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
                    <ReportsToSelect value={lead()} onChange={setLead} />
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
                        <For each={agents()}>{(a) => <AgentStripChip agent={a} model={model} onOpenAgent={openAgent} />}</For>
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
                                collapsed={collapsed()}
                                onToggleCollapsed={toggleColumn}
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
                                        collapsed={collapsed()}
                                        onToggleCollapsed={toggleColumn}
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
                                                        collapsed={collapsed()}
                                                        onToggleCollapsed={toggleColumn}
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
        </div>
    );
};
