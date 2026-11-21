// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's stages over a set of tasks (SPEC_WARDEN_KANBAN_2026_10_20.md §6.3;
// SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §4; SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §8.3): the
// whole board, or one repository's lane of it (§6.11). A stage shows some of the eight sources the board
// computes (Backlog … Released); the default board has one stage per source. A stage of waiting sources
// groups its cards by the agent each task waits for, each group a drop target; untracked work sits in the
// stage that shows In progress; Review's load in the stage that shows Review; cancelled tasks folded under
// the stage that shows Merged.

import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX, type Setter } from "solid-js";

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { Button, IconButton } from "@/app/element/ui";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { tip } from "@/app/element/tip";
import { AgentChip, agentStateWords, KanbanCard, UntrackedCard } from "./kanban-card";
import { backlogGroups, columnStats, rankBetween, rankOf, reviewLoad, type BacklogGroup, type ColumnStats } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import {
    defaultStages,
    isFinishedStage,
    isWaitingStage,
    parseCollapsedStages,
    stageTasks,
    type KanbanStage,
} from "./kanban-stages";
import type { KanbanAgent, KanbanTask } from "./kanban-types";
import { WindowedList } from "./kanban-window";

import "./kanban-stage-colours.scss";

/** A drop target for Kanban cards; `hover` shows it will take the card. */
export function useCardDrop(
    el: () => HTMLElement | undefined,
    onDrop: (id: string) => void,
    enabled: () => boolean = () => true
): () => boolean {
    const [hover, setHover] = createSignal(false);
    onMount(() => {
        const element = el();
        if (!element) return;
        onCleanup(
            dropTargetForElements({
                element,
                canDrop: ({ source }) => source.data.type === kanbanCardItemType && enabled(),
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
            <WindowedList
                items={props.group.tasks}
                keyOf={(t) => t.id}
                each={(t) => (
                    <KanbanCard
                        task={t}
                        now={props.now}
                        agents={props.agents}
                        colors={props.colors}
                        onOpen={props.onOpen}
                    />
                )}
            />
        </div>
    );
}

/** A collapsed column: a thin strip with the column's name written down it and its stats, one button
 *  that opens the column again. A waiting column's strip still takes a dropped card. */
function CollapsedColumn(props: {
    label: string;
    /** Whether the strip takes a dropped card (a stage of waiting sources). */
    drops: boolean;
    stats: ColumnStats;
    onExpand: () => void;
    onDrop: (id: string) => void;
}): JSX.Element {
    let el: HTMLButtonElement | undefined;
    const hover = useCardDrop(
        () => el,
        props.onDrop,
        () => props.drops
    );
    const words = () => {
        const s = props.stats;
        const parts = [`${s.count} ${s.count === 1 ? "task" : "tasks"}`];
        if (s.held > 0) parts.push(`${s.held} on hold`);
        if (s.oldestMs > 0) parts.push(`oldest ${formatAge(s.oldestMs)}`);
        return parts.join(", ");
    };
    return (
        <Button
            ref={el}
            density="compact"
            tone="quiet"
            class={hover() ? "kanban-collapsed kanban-drop--hover" : "kanban-collapsed"}
            aria-label={`${props.label}: ${words()}. Expand`}
            aria-expanded={false}
            title={`${props.label}: ${words()}. Click to expand`}
            onClick={props.onExpand}
        >
            <i class="fa-solid fa-chevron-right kanban-collapsed__chevron" aria-hidden="true" />
            <span class="kanban-collapsed__count">{props.stats.count}</span>
            <span class="kanban-collapsed__name">{props.label}</span>
            <Show when={props.stats.held > 0}>
                <span class="kanban-collapsed__stat">{props.stats.held} held</span>
            </Show>
            <Show when={props.stats.oldestMs > 0}>
                <span class="kanban-collapsed__stat">oldest {formatAge(props.stats.oldestMs)}</span>
            </Show>
        </Button>
    );
}

interface BoardProps {
    tasks: KanbanTask[];
    untracked: KanbanAgent[];
    agents: KanbanAgent[];
    model: KanbanModel;
    now: number;
    holding: (agent: string) => KanbanTask | undefined;
    onOpen: (t: KanbanTask) => void;
    onOpenAgent: (blockId: string) => void;
    /** The board's stages, in the order they are drawn (the person's own); the default eight when absent. */
    stages?: readonly KanbanStage[];
    /** Review's soft limit (`kanban:reviewlimit`): past it the header says so. 0: none. */
    reviewLimit: number;
    /** How many tasks the whole board has in Review, which the limit is about (a lane holds some). */
    reviewTotal: number;
    /** The colours of agents not running here, by uid (`agent_colors`). */
    colors?: Record<string, string>;
    /** The narrow layout (§8): one list, the finished stages folded until opened. */
    narrow?: boolean;
    /** The stages collapsed to a strip, by id (wide layout only; the narrow list folds its own). */
    collapsed?: readonly string[];
    /** Collapse or expand a stage; without it the board has no collapse control. */
    onToggleCollapsed?: (id: string) => void;
}

/** What the board's stages share: the props, which stages are folded or collapsed, Review's load, and
 *  whether the cancelled tasks under Merged are shown. */
interface BoardState {
    props: BoardProps;
    groups: (stage: KanbanStage) => BacklogGroup[];
    foldable: (stage: KanbanStage) => boolean;
    folded: (stage: KanbanStage) => boolean;
    toggle: (id: string) => void;
    stripped: (id: string) => boolean;
    gridColumns: () => string | undefined;
    review: () => ReturnType<typeof reviewLoad>;
    over: () => boolean;
    showCancelled: () => boolean;
    setShowCancelled: Setter<boolean>;
}

const DEFAULT_STAGES = defaultStages();

/** The stages a board draws. */
const stagesOf = (props: BoardProps): readonly KanbanStage[] => props.stages ?? DEFAULT_STAGES;

function createBoardState(props: BoardProps): BoardState {
    const groups = (stage: KanbanStage) => backlogGroups(props.tasks, props.agents, stage.takes);
    // In the narrow list, the finished stages (Merged and Released by default) start folded: one header line each.
    const [unfolded, setUnfolded] = createSignal<ReadonlySet<string>>(new Set());
    const foldable = (stage: KanbanStage) => !!props.narrow && isFinishedStage(stage);
    const folded = (stage: KanbanStage) => foldable(stage) && !unfolded().has(stage.id);
    const toggle = (id: string) =>
        setUnfolded((s) => {
            const next = new Set(s);
            if (!next.delete(id)) next.add(id);
            return next;
        });
    const collapsedSet = createMemo(
        () => new Set(props.narrow || !props.onToggleCollapsed ? [] : parseCollapsedStages(props.collapsed, stagesOf(props)))
    );
    const stripped = (id: string) => collapsedSet().has(id);
    // Collapsed stages take a strip's width; the rest share what is left, as they did.
    const gridColumns = () =>
        collapsedSet().size === 0
            ? undefined
            : stagesOf(props).map((s) => (stripped(s.id) ? "var(--kanban-strip-width)" : "minmax(160px, 1fr)")).join(" ");
    const review = createMemo(() => reviewLoad(props.tasks, props.now));
    const over = () => props.reviewLimit > 0 && props.reviewTotal > props.reviewLimit;
    const [showCancelled, setShowCancelled] = createSignal(false);
    return {
        props,
        groups,
        foldable,
        folded,
        toggle,
        stripped,
        gridColumns,
        review,
        over,
        showCancelled,
        setShowCancelled,
    };
}

export function KanbanBoard(props: BoardProps): JSX.Element {
    const s = createBoardState(props);
    return (
        <div class="kanban-board" style={{ "grid-template-columns": s.gridColumns() }}>
            <For each={stagesOf(props)}>{(stage) => <ColumnSection s={s} stage={stage} />}</For>
        </div>
    );
}

function ColumnSection({ s, stage }: { s: BoardState; stage: KanbanStage }) {
    const { props, folded, stripped } = s;
    const tasks = () => stageTasks(props.tasks, stage);
    const holdsMerged = stage.takes.includes("merged");
    const holdsProgress = stage.takes.includes("progress");
    const finished = () => (holdsMerged ? tasks().filter((t) => t.state !== "cancelled") : tasks());
    const cancelled = () => (holdsMerged ? tasks().filter((t) => t.state === "cancelled") : []);
    // A stage that takes backlog stays open when empty, so a card can be dropped on it.
    const takesBacklogDrops = isWaitingStage(stage) && stage.takes.includes("backlog");
    return (
        <section
            class="kanban-column"
            data-stage-colour={stage.colour}
            classList={{
                [`kanban-column--${stage.id}`]: true,
                "kanban-column--has-needs": stage.takes.includes("needs"),
                "kanban-column--empty":
                    finished().length + cancelled().length === 0 &&
                    !(holdsProgress && props.untracked.length > 0) &&
                    !takesBacklogDrops,
                "kanban-column--folded": folded(stage),
                "kanban-column--collapsed": stripped(stage.id),
            }}
            aria-label={stage.name}
        >
            <Show when={!stripped(stage.id)} fallback={<ColumnStrip s={s} stage={stage} tasks={tasks} />}>
                {columnHead(s, stage, finished().length + (holdsProgress ? props.untracked.length : 0))}
                <Show when={!isWaitingStage(stage)} fallback={<WaitingList s={s} stage={stage} />}>
                    <ColumnList s={s} stage={stage} finished={finished} cancelled={cancelled} />
                </Show>
            </Show>
        </section>
    );
}

function ColumnStrip({ s, stage, tasks }: { s: BoardState; stage: KanbanStage; tasks: () => KanbanTask[] }) {
    const { props } = s;
    return (
        <CollapsedColumn
            label={stage.name}
            drops={isWaitingStage(stage)}
            stats={columnStats(tasks(), props.now, stage.takes.includes("progress") ? props.untracked.length : 0)}
            onExpand={() => props.onToggleCollapsed?.(stage.id)}
            onDrop={(id) => {
                // Dropped on a waiting stage's strip: the task waits for anyone, at
                // the end of the column.
                const last = tasks()[tasks().length - 1];
                void props.model.act({
                    action: "move",
                    id,
                    agent: "",
                    rank: rankBetween(last ? rankOf(last) : undefined, undefined),
                });
            }}
        />
    );
}

function columnHead(s: BoardState, stage: KanbanStage, count: number) {
    const { props, foldable, folded, toggle } = s;
    return (
        <div
            class="kanban-column__head"
            classList={{ "kanban-column__head--foldable": foldable(stage) }}
            role={foldable(stage) ? "button" : undefined}
            tabindex={foldable(stage) ? 0 : undefined}
            aria-expanded={foldable(stage) ? !folded(stage) : undefined}
            onClick={() => foldable(stage) && toggle(stage.id)}
            onKeyDown={(e) => {
                if (foldable(stage) && (e.key === "Enter" || e.key === " ")) {
                    e.preventDefault();
                    toggle(stage.id);
                }
            }}
        >
            <span class="kanban-column__label">
                <Show when={foldable(stage)}>
                    <i class={`fa-solid fa-chevron-${folded(stage) ? "right" : "down"}`} aria-hidden="true" />
                </Show>
                {stage.name}
            </span>
            <Show when={!props.narrow && props.onToggleCollapsed}>
                {(toggleCollapsed) => (
                    <IconButton
                        icon="chevron-left"
                        label={`Collapse ${stage.name}`}
                        class="kanban-column__collapse"
                        aria-expanded={true}
                        onClick={(e) => {
                            e.stopPropagation();
                            toggleCollapsed()(stage.id);
                        }}
                    />
                )}
            </Show>
            <ColumnCount s={s} stage={stage} count={count} />
        </div>
    );
}

function ColumnCount({ s, stage, count }: { s: BoardState; stage: KanbanStage; count: number }) {
    const { props, review, over } = s;
    // Review's load is said on the stage that shows Review, whatever it is called.
    const hasReview = stage.takes.includes("review");
    return (
        <span
            class="kanban-column__count"
            classList={{ "kanban-column__count--over": hasReview && over() }}
            {...tip(hasReview && over() ? `More than ${props.reviewLimit} waiting for review` : undefined)}
        >
            {count}
            <Show when={hasReview && review().specs > 0}>
                {" · "}
                {review().specs} spec{review().specs === 1 ? "" : "s"}
            </Show>
            <Show when={hasReview && review().count > 0}> · oldest {formatAge(review().oldestMs)}</Show>
            <Show when={hasReview && over()}> · over {props.reviewLimit}</Show>
        </span>
    );
}

function card(s: BoardState, t: KanbanTask) {
    const { props } = s;
    return <KanbanCard task={t} now={props.now} agents={props.agents} colors={props.colors} onOpen={props.onOpen} />;
}

/** A waiting stage: one group per agent, each a drop target. A stage that shows Backlog always shows Anyone,
 *  so a card can be given to anyone; one that shows only Spec'd has only the groups it has. */
function WaitingList({ s, stage }: { s: BoardState; stage: KanbanStage }) {
    const { props, groups } = s;
    return (
        <div class="kanban-column__list">
            {/* Keyed by the group's agent name (a string), so a group, and its drop target, stays mounted
                when the board refreshes. */}
            <For each={groups(stage).map((g) => g.key)}>
                {(key) => (
                    <Show when={groups(stage).find((g) => g.key === key)}>
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
            <Show when={stage.takes.includes("backlog") && !groups(stage).some((g) => g.key === "")}>
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
}

function ColumnList(listProps: {
    s: BoardState;
    stage: KanbanStage;
    finished: () => KanbanTask[];
    cancelled: () => KanbanTask[];
}) {
    const { s, stage, finished, cancelled } = listProps;
    const { props, showCancelled, setShowCancelled } = s;
    return (
        <div class="kanban-column__list">
            <Show when={stage.takes.includes("progress")}>
                <For each={props.untracked}>
                    {(a) => (
                        <UntrackedCard
                            agent={a}
                            onTrack={(ag) => void props.model.act({ action: "track", block_id: ag.block_id })}
                            onOpenAgent={(ag) => props.onOpenAgent(ag.block_id)}
                        />
                    )}
                </For>
            </Show>
            <WindowedList items={finished()} keyOf={(t) => t.id} each={(t) => card(s, t)} />
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
                    <WindowedList items={cancelled()} keyOf={(t) => t.id} each={(t) => card(s, t)} />
                </Show>
            </Show>
        </div>
    );
}
