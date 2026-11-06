// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban section's state (kanban-section.tsx): the model, the filters, the lanes and collapsed
// columns, the task that is open and the narrow layout, and what the board is made of from them. The
// hooks run in the order the section always ran them: the order they subscribe and register effects in
// is part of what the section does.

import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";

import { useTick } from "@/app/hook/useTick";
import { useAccountEpisodes } from "@/app/store/account-availability";
import { settingsAtom } from "@/app/store/config-signals";
import { revealBlock } from "@/app/util/reveal-block";

import {
    columnOf,
    KANBAN_NARROW_PX,
    matchesQuery,
    needsCountOf,
    parseCollapsed,
    repoLanes,
    reviewLimitOf,
    toggleCollapsed,
    untrackedAgents,
    type ColumnId,
} from "./kanban-columns";
import { KanbanModel, taskForOpenRequest } from "./kanban-model";
import type { KanbanSectionProps } from "./kanban-props";
import { reportsTo, seatOf, taskReportsTo, useSeatIndex } from "./kanban-seats";
import type { KanbanTask } from "./kanban-types";

/** What the Kanban pane passes (kanban-props.ts); all but `visible` are optional, so the board also mounts alone. */
export type SectionProps = Pick<KanbanSectionProps, "visible"> & Partial<Omit<KanbanSectionProps, "visible">>;

/** The model, followed while the pane is shown, and the Needs-you count it gives the pane. */
function useBoardModel(props: SectionProps): KanbanModel {
    const model = new KanbanModel();
    onCleanup(() => model.dispose());
    createEffect(() => model.setVisible(props.visible));
    // An account episode counts once (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2).
    const accountEpisodes = useAccountEpisodes();
    createEffect(() => props.onNeedsCount?.(needsCountOf(model.board()?.tasks ?? [], accountEpisodes()) + accountEpisodes().length));
    return model;
}

/**
 * The narrow layout (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §8): this section's own width
 * at KANBAN_NARROW_PX or less, as `kanban.scss`'s container query; the board folds Merged and Released.
 * The section is the named container (`.kanban-container`), so the pane needs no knowledge of it.
 */
function useNarrow() {
    let container: HTMLDivElement | undefined;
    const [narrow, setNarrow] = createSignal(false);
    onMount(() => {
        if (!container) return;
        const observer = new ResizeObserver(([entry]) => setNarrow(entry.contentRect.width <= KANBAN_NARROW_PX));
        observer.observe(container);
        onCleanup(() => observer.disconnect());
    });
    return { bindContainer: (el: HTMLDivElement) => (container = el), narrow };
}

/**
 * Columns collapsed to a strip: the pane's (it survives a restart), or this section's own when it
 * mounts alone. A choice shows at once and holds until the pane's meta says the same (or a few seconds
 * pass), so two clicks in a row, or a slow write, don't undo each other.
 */
function useCollapsedColumns(props: SectionProps) {
    const [ownCollapsed, setOwnCollapsed] = createSignal<ColumnId[]>([]);
    const [pending, setPending] = createSignal<ColumnId[] | null>(null);
    const collapsed = () => pending() ?? parseCollapsed(props.collapsed ?? ownCollapsed());
    createEffect(() => {
        const mine = pending();
        if (mine && parseCollapsed(props.collapsed).join() === mine.join()) setPending(null);
    });
    let pendingTimer: number | undefined;
    onCleanup(() => window.clearTimeout(pendingTimer));
    const toggleColumn = (id: ColumnId) => {
        const next = toggleCollapsed(collapsed(), id);
        if (!props.onCollapsed) return setOwnCollapsed(next);
        setPending(next);
        window.clearTimeout(pendingTimer);
        pendingTimer = window.setTimeout(() => setPending(null), 3000);
        props.onCollapsed(next);
    };
    return { collapsed, toggleColumn };
}

/** The lanes folded to their head, by repository. */
function useFoldedLanes() {
    const [folded, setFolded] = createSignal<ReadonlySet<string>>(new Set());
    const fold = (repo: string) =>
        setFolded((f) => {
            const next = new Set(f);
            if (!next.delete(repo)) next.add(repo);
            return next;
        });
    return { folded, fold };
}

/** The Kanban section's state: everything its markup reads, and what it does. */
export function createKanbanSection(props: SectionProps) {
    const model = useBoardModel(props);
    const tick = useTick(10_000);
    const { bindContainer, narrow } = useNarrow();
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
    const { collapsed, toggleColumn } = useCollapsedColumns(props);
    const { folded, fold } = useFoldedLanes();
    const reviewLimit = () => reviewLimitOf(settingsAtom() as unknown as Record<string, unknown>);

    // Opened from elsewhere: show the task once the board has it, then clear the request.
    createEffect(() => {
        const request = props.openRequest;
        // A task's id, or its number as the operator sees it ("12": a link, agentmux://task/12).
        const task = request ? taskForOpenRequest(model.board()?.tasks ?? [], request) : undefined;
        if (!task) return;
        setOpenId(task.id);
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
    // "Go to this agent": the pane's owner may place it (`onOpenAgent`); on its own, the agent is revealed where it is.
    const openAgent = (blockId: string) =>
        props.onOpenAgent ? props.onOpenAgent(blockId) : void revealBlock(blockId, { source: "kanban" });

    return {
        model, bindContainer, narrow, now,
        query, setQuery, repo, setRepo, lead, setLead, showUntracked, setShowUntracked,
        adding, setAdding, setOpenId, lanes, setLanes, collapsed, toggleColumn, folded, fold, reviewLimit,
        board, agents, repos, tasks, openTask, holding, untracked, laneList, reviewTotal, open, openAgent,
    };
}

/** What the section's markup reads. */
export type KanbanSectionState = ReturnType<typeof createKanbanSection>;
