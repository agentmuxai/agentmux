// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban section's markup in parts (kanban-section.tsx): the toolbar, the agent strip, the board
// (one, or a lane each) and the open task's drawer. Each reads the section's state (kanban-section-state.ts).

import { For, Show, type JSX } from "solid-js";

import { Button, FilterInput, IconButton, Select, Switch } from "@/app/element/ui";
import { tip } from "@/app/element/tip";

import { KanbanBoard, useCardDrop } from "./kanban-board";
import { AgentChip, agentStateWords } from "./kanban-card";
import { columnOf, isFinishedColumn, repoLabel } from "./kanban-columns";
import { KanbanDrawer } from "./kanban-drawer";
import type { KanbanModel } from "./kanban-model";
import { KANBAN_REMINDERS_DESCRIPTION, kanbanRemindersOn, setKanbanReminders } from "./kanban-reminders";
import { ReportsToSelect } from "./kanban-seats";
import { settingsAtom } from "@/app/store/config-signals";
import type { KanbanSectionState } from "./kanban-section-state";
import type { KanbanAgent, KanbanTask } from "./kanban-types";

type Part = { s: KanbanSectionState };

export function KanbanToolbar(props: Part): JSX.Element {
    const s = props.s;
    return (
        <div class="kanban-toolbar">
            <FilterInput
                value={s.query()}
                onInput={s.setQuery}
                placeholder="Find a task, #number, agent or repository"
                ariaLabel="Find a task"
            />
            <Show when={s.repos().length > 1}>
                <Select
                    density="compact"
                    ariaLabel="Repository"
                    value={s.repo()}
                    onChange={s.setRepo}
                    options={[
                        { value: "", label: "All repositories" },
                        ...s.repos().map((r) => ({ value: r, label: r })),
                    ]}
                />
            </Show>
            <ReportsToSelect value={s.lead()} onChange={s.setLead} />
            <label class="kanban-toolbar__switch" {...tip("One row of columns per repository")}>
                <Switch checked={s.lanes()} onChange={s.setLanes} density="compact" ariaLabel="Lanes by repository" />
                Lanes
            </label>
            <label
                class="kanban-toolbar__switch"
                {...tip("Agents working without a task show as dashed cards in In progress")}
            >
                <Switch
                    checked={s.showUntracked()}
                    onChange={s.setShowUntracked}
                    density="compact"
                    ariaLabel="Show untracked work"
                />
                Untracked
            </label>
            <label class="kanban-toolbar__switch" {...tip(KANBAN_REMINDERS_DESCRIPTION)}>
                <Switch
                    checked={kanbanRemindersOn(settingsAtom() as unknown as Record<string, unknown>)}
                    onChange={setKanbanReminders}
                    density="compact"
                    ariaLabel="Remind idle agents of their backlog"
                />
                Reminders
            </label>
            <Button density="compact" tone="accent" icon="plus" onClick={() => s.setAdding(true)}>
                New task
            </Button>
            <IconButton icon="rotate" label="Refresh" onClick={() => void s.model.refresh()} />
        </div>
    );
}

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

export function AgentStrip(props: Part): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.agents().length > 0}>
            <div class="kanban-strip" aria-label="Agents">
                <For each={s.agents()}>{(a) => <AgentStripChip agent={a} model={s.model} onOpenAgent={s.openAgent} />}</For>
            </div>
        </Show>
    );
}

/** The board over some tasks and untracked agents: the whole board, or one lane of it. */
function SectionBoard(props: Part & { tasks: KanbanTask[]; untracked: KanbanAgent[] }): JSX.Element {
    const s = props.s;
    return (
        <KanbanBoard
            tasks={props.tasks}
            untracked={props.untracked}
            agents={s.agents()}
            model={s.model}
            now={s.now()}
            holding={s.holding}
            onOpen={s.open}
            onOpenAgent={s.openAgent}
            reviewLimit={s.reviewLimit()}
            reviewTotal={s.reviewTotal()}
            colors={s.board()?.agent_colors}
            narrow={s.narrow()}
            collapsed={s.collapsed()}
            onToggleCollapsed={s.toggleColumn}
        />
    );
}

function LaneHead(props: Part & { repo: string; label: string; tasks: KanbanTask[] }): JSX.Element {
    const s = props.s;
    return (
        <Button
            density="compact"
            tone="quiet"
            icon={s.folded().has(props.repo) ? "chevron-right" : "chevron-down"}
            class="kanban-lane__head"
            aria-expanded={!s.folded().has(props.repo)}
            onClick={() => s.fold(props.repo)}
        >
            <span class="kanban-lane__name">{props.label}</span>
            <span class="warden-manager-dim">
                {props.tasks.filter((t) => !isFinishedColumn(columnOf(t))).length}{" "}
                open
                {" · "}
                {props.tasks.filter((t) => columnOf(t) === "needs").length} need
                you
            </span>
        </Button>
    );
}

/** One row of columns per repository, with the untracked work (which belongs to no task, so to no lane) first. */
function LanesView(props: Part): JSX.Element {
    const s = props.s;
    return (
        <div class="kanban-lanes">
            {/* Untracked work belongs to no task, so to no lane: its own row, In progress only. */}
            <Show when={s.untracked().length > 0}>
                <section class="kanban-lane" aria-label="Agents working without a task">
                    <div class="kanban-lane__head">
                        <span class="kanban-lane__name">Working without a task</span>
                        <span class="warden-manager-dim">{s.untracked().length}</span>
                    </div>
                    <SectionBoard s={s} tasks={[]} untracked={s.untracked()} />
                </section>
            </Show>
            {/* Keyed by repository (a string), so a lane stays mounted when the board refreshes. */}
            <For each={s.laneList().map((l) => l.repo)}>
                {(repo) => (
                    <Show when={s.laneList().find((l) => l.repo === repo)}>
                        {(lane) => (
                            <section class="kanban-lane" aria-label={repoLabel(repo)}>
                                <LaneHead s={s} repo={repo} label={lane().label} tasks={lane().tasks} />
                                <Show when={!s.folded().has(repo)}>
                                    <SectionBoard s={s} tasks={lane().tasks} untracked={[]} />
                                </Show>
                            </section>
                        )}
                    </Show>
                )}
            </For>
        </div>
    );
}

/** The board, or one lane each; a note while it loads. */
export function BoardArea(props: Part): JSX.Element {
    const s = props.s;
    return (
        <Show
            when={s.board()}
            fallback={<p class="warden-manager-dim">{s.model.error() ? "" : "Loading the Kanban…"}</p>}
        >
            <Show when={s.lanes()} fallback={<SectionBoard s={s} tasks={s.tasks()} untracked={s.untracked()} />}>
                <LanesView s={s} />
            </Show>
        </Show>
    );
}

export function OpenDrawer(props: Part): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.openTask()}>
            {(t) => (
                <KanbanDrawer
                    model={s.model}
                    task={t()}
                    agents={s.agents()}
                    now={s.now()}
                    onClose={() => s.setOpenId(null)}
                    onOpenAgent={s.openAgent}
                    onOpenTask={(id) => s.setOpenId(id)}
                />
            )}
        </Show>
    );
}
