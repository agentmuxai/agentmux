// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The panel under a My agents tile: which row's panel is up and which of its views, the rename and fork-name
// states, opening / switching / attaching / forking, and the delete (with the sweep of the agent's panes).
// Split out of MyAgentsList.tsx (#13044 S6); MyAgentsRowPanel.tsx draws it.

import { rowEnd } from "@/app/element/ui/CardGrid";
import { getOpenBlockIdsForDefinition } from "@/app/store/agent-pane-state-store";
import { hasProcess } from "@/app/store/agent-runtime";
import { pushNotification } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ObjectService } from "@/app/store/services";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { eventBelongsToPaneOf } from "@/util/focusutil";
import { Logger } from "@/util/logger";
import { batch, createSignal, onCleanup } from "solid-js";
import { requestShutdown } from "../shutdown/shut-down-agent";
import type { MyAgentsListProps } from "./MyAgentsList";
import type { MyAgentsData } from "./my-agents-list-data";

type ForkState =
    | { kind: "idle" }
    // name input expanded; `edited`: the user changed the suggested name, so a click outside no longer dismisses it
    | { kind: "naming"; label: string; loading: boolean; error: string | null; edited?: boolean };

export type SubView = "seat" | "bundles" | "env" | "delete";

export interface RenameState {
    label: string;
    loading: boolean;
    error: string | null;
}

export interface RowAction {
    label: string;
    icon: string;
    testId?: string;
    tone?: "danger";
    onSelect: () => void;
}

export function createRowPanels(
    props: MyAgentsListProps,
    data: Pick<MyAgentsData, "setRowsStore" | "setDeletingIds" | "sortedRows">
) {
    const { setRowsStore, setDeletingIds, sortedRows } = data;
    // Fork prompt state per row (keyed by definition_id)
    const [forkStates, setForkStates] = createSignal<Map<string, ForkState>>(new Map());

    const getForkState = (definitionId: string): ForkState => forkStates().get(definitionId) ?? { kind: "idle" };

    const setForkState = (definitionId: string, state: ForkState): void => {
        setForkStates((prev) => {
            const next = new Map(prev);
            next.set(definitionId, state);
            return next;
        });
    };

    // Where the agents are open, read fresh when a panel opens (a pane can be in another window); until
    // then, and for a list that has no way to ask, the list's own map.
    const [freshOpen, setFreshOpen] = createSignal<Map<string, string> | null>(null);
    const openMap = (): Map<string, string> => freshOpen() ?? props.openDefinitions?.() ?? new Map<string, string>();
    const runsWithoutPane = (definitionId: string): boolean => {
        const runtime = props.runtimeOf?.(definitionId);
        return !!props.onAttach && !!runtime && hasProcess(runtime.state) && !openMap().has(definitionId);
    };
    // A click anywhere on the tile, or on its chevron, opens the agent's panel or closes it. The panel holds
    // what the click used to decide (open it, switch to its pane, attach it, fork it) and the row's actions.
    const toggleRowPanel = (row: RecentSessionRow): void => {
        const id = row.definition_id;
        if (panelRowId() === id) {
            closePanels();
            return;
        }
        closePanels();
        setFreshOpen(null);
        setOpenMenuId(id);
        void props.resolveOpenDefinitions?.().then(
            (open) => {
                if (panelRowId() === id) setFreshOpen(open);
            },
            () => undefined
        );
    };
    // Open starts the agent. The panel's first look at where it is open can still be on its way (a pane in
    // another window), so Open waits for it and, if the agent turns out to be open, leaves the panel up to
    // offer Switch instead of starting a second one.
    // The read is made again at the click: a pane can open or close in another window while the panel is up.
    const readOpenNow = async (): Promise<Map<string, string> | null> => {
        if (!props.resolveOpenDefinitions) return null;
        const open = await props.resolveOpenDefinitions().catch(() => null);
        if (open) setFreshOpen(open);
        return open;
    };
    const handleOpen = async (row: RecentSessionRow): Promise<void> => {
        const open = await readOpenNow();
        if (open?.has(row.definition_id)) {
            return;
        }
        closePanels();
        props.onReattach(row);
    };
    const handleAttachNoPane = (row: RecentSessionRow) => {
        closePanels();
        props.onAttach?.(row);
    };

    const handleOpenNewSession = async (row: RecentSessionRow) => {
        setForkState(row.definition_id, { kind: "naming", label: "", loading: true, error: null });
        try {
            const result = await RpcApi.ForkAgentDefinitionSuggestCommand(TabRpcClient, {
                source_id: row.definition_id,
            });
            setForkState(row.definition_id, {
                kind: "naming",
                label: result.suggested_label,
                loading: false,
                error: null,
            });
        } catch {
            setForkState(row.definition_id, {
                kind: "naming",
                label: `${row.definition_name} #2`,
                loading: false,
                error: null,
            });
        }
    };

    const handleSwitchToExisting = async (row: RecentSessionRow): Promise<void> => {
        const blockId = ((await readOpenNow()) ?? openMap()).get(row.definition_id);
        closePanels();
        if (blockId) props.onSwitchToExisting?.(blockId);
    };

    const handleForkStart = async (row: RecentSessionRow) => {
        const fs = getForkState(row.definition_id);
        if (fs.kind !== "naming" || !fs.label.trim()) return;
        const label = fs.label.trim();
        setForkState(row.definition_id, { kind: "naming", label, loading: true, error: null, edited: fs.edited });
        try {
            await props.onFork?.(row, label);
            closePanels();
        } catch (err) {
            setForkState(row.definition_id, {
                kind: "naming",
                label,
                loading: false,
                error: err instanceof Error ? err.message : String(err),
                // A failed start keeps the panel (and its error) up whatever the name was.
                edited: true,
            });
        }
    };

    const handleForkCancel = (definitionId: string) => {
        setForkState(definitionId, { kind: "idle" });
    };

    // The panel under a tile (AgentRowPanel): one row's at a time, and the views of one row are
    // alternatives: the actions (the chevron), a rename, a fork name or the already-open choice (a tile
    // click), and Seat, Bundles, Environment and delete (reached from the actions). Opening one closes
    // the others, on this row and on every other. Like the New Agent panel it stays until it is closed
    // (its X, Esc, or the tile's chevron again), not on an outside click.
    const [openMenuId, setOpenMenuId] = createSignal<string | null>(null);
    const [subView, setSubView] = createSignal<{ id: string; kind: SubView } | null>(null);
    /** Close every panel view except those of `keepId` (none given: all). */
    const closePanels = (keepId?: string): void => {
        if (openMenuId() !== null && openMenuId() !== keepId) setOpenMenuId(null);
        const sub = subView();
        if (sub && sub.id !== keepId) setSubView(null);
        for (const id of [...renameStates().keys()]) if (id !== keepId) setRenameState(id, null);
        for (const [id, fs] of [...forkStates()])
            if (id !== keepId && fs.kind !== "idle") setForkState(id, { kind: "idle" });
    };
    const closeMenu = (definitionId: string): void => {
        setOpenMenuId((prev) => (prev === definitionId ? null : prev));
    };
    let rootRef: HTMLDivElement | undefined;
    const setRootRef = (el: HTMLDivElement): void => {
        rootRef = el;
    };
    const onDocumentKeyDown = (e: KeyboardEvent) => {
        // Escape in another pane must not close this pane's panel.
        if (e.key !== "Escape" || !eventBelongsToPaneOf(e, rootRef) || deleteBusy()) return;
        // A field inside the panel takes its own Escape (the name field cancels itself).
        const t = e.target;
        if (t instanceof HTMLElement && t.closest(".agent-row-panel") && t.matches("input, textarea")) return;
        closePanels();
    };
    // In the capture phase: a field's own Escape can change the view and take the field out of the panel
    // before this runs, and the check above needs to see it still inside.
    document.addEventListener("keydown", onDocumentKeyDown, true);
    onCleanup(() => {
        document.removeEventListener("keydown", onDocumentKeyDown, true);
    });

    // Seat, Bundles, Environment and delete are views of the row's panel (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3
    // for the seat). Each was a modal over the picker.
    // A change of view is one step: the open id must not be empty in between, or the panel is torn down and
    // built again (and replays its opening) instead of changing its body.
    const openSubView = (row: RecentSessionRow, kind: SubView): void =>
        batch(() => {
            closePanels();
            setSubView({ id: row.definition_id, kind });
        });
    const handleSeatOpen = (row: RecentSessionRow): void => openSubView(row, "seat");
    const handleBundlesOpen = (row: RecentSessionRow): void => openSubView(row, "bundles");
    const handleEnvOpen = (row: RecentSessionRow): void => openSubView(row, "env");
    // Duplicate — reuses the EXISTING fork-and-launch flow unchanged
    // (handleOpenNewSession/handleForkStart above, already wired to
    // props.onFork = AgentPicker.handleFork); its naming step is the panel's fork-name view.
    // Repo-owner-confirmed 2026-09-16: keep conversation history forking as-is
    // (forkSession: true, unchanged).
    const handleDuplicate = (row: RecentSessionRow): void =>
        batch(() => {
            closeMenu(row.definition_id);
            // Rename and Duplicate render two DIFFERENT panels, both anchored to
            // the same tile, from two independent state maps — so opening one while
            // the other was already open showed both name inputs stacked in one
            // row (ReAgent P2 on PR #3262). They are alternatives, not
            // companions: entering either closes the other.
            setRenameState(row.definition_id, null);
            void handleOpenNewSession(row);
        });

    // Rename — RenameAgentDefinitionTitleCommand already exists (used
    // today for fork/stack-tab double-click-rename); this is its first
    // My Agents row entry point. Keyed by definition_id like forkStates.
    const [renameStates, setRenameStates] = createSignal<Map<string, RenameState>>(new Map());
    const getRenameState = (definitionId: string): RenameState | null => renameStates().get(definitionId) ?? null;
    const setRenameState = (definitionId: string, state: RenameState | null): void => {
        setRenameStates((prev) => {
            const next = new Map(prev);
            if (state === null) next.delete(definitionId);
            else next.set(definitionId, state);
            return next;
        });
    };
    const handleRenameOpen = (row: RecentSessionRow): void =>
        batch(() => {
            closeMenu(row.definition_id);
            // The other half of `handleDuplicate`'s mutual exclusion — see the
            // comment there.
            setForkState(row.definition_id, { kind: "idle" });
            setRenameState(row.definition_id, {
                label: row.instance_name || row.definition_name,
                loading: false,
                error: null,
            });
        });
    const handleRenameCancel = (definitionId: string): void =>
        batch(() => {
            setRenameState(definitionId, null);
            setOpenMenuId(definitionId);
        });
    const handleRenameSubmit = async (row: RecentSessionRow): Promise<void> => {
        const rs = getRenameState(row.definition_id);
        if (!rs || !rs.label.trim()) return;
        const title = rs.label.trim();
        setRenameState(row.definition_id, { ...rs, loading: true, error: null });
        try {
            await RpcApi.RenameAgentDefinitionTitleCommand(TabRpcClient, { id: row.definition_id, title });
            // Backend broadcasts "agents:changed" (template.rs) — the
            // existing subscription in my-agents-list-data.ts already refetches
            // and picks up the new name; nothing else to update here.
            setRenameState(row.definition_id, null);
        } catch (err) {
            setRenameState(row.definition_id, {
                label: title,
                loading: false,
                error: err instanceof Error ? err.message : String(err),
            });
        }
    };

    /** The row whose panel is showing: its actions, a rename, a fork name or choice, or a sub-view. */
    const panelRowId = (): string | null => {
        const sub = subView();
        if (sub) return sub.id;
        const menu = openMenuId();
        if (menu !== null) return menu;
        const renaming = renameStates().keys().next();
        if (!renaming.done) return renaming.value;
        for (const [id, fs] of forkStates()) if (fs.kind !== "idle") return id;
        return null;
    };
    const isRowExpanded = (definitionId: string): boolean => panelRowId() === definitionId;

    // The tiles by row id, for the panel's caret and its scroll; and the list's column count, which decides
    // the tile the panel goes after: the last of the clicked tile's row, so the row stays together.
    const tileEls = new Map<string, HTMLElement>();
    const [columns, setColumns] = createSignal(1);
    const measureList = (ul: HTMLUListElement): void => {
        const measure = () =>
            setColumns(Math.max(1, getComputedStyle(ul).gridTemplateColumns.split(" ").filter(Boolean).length));
        measure();
        if (typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver(measure);
        ro.observe(ul);
        onCleanup(() => ro.disconnect());
    };
    /** The row whose panel goes right after row `id`'s tile, if that is where it goes. */
    const panelAfter = (id: string): RecentSessionRow | null => {
        const openId = panelRowId();
        if (openId === null) return null;
        const list = sortedRows();
        const openIndex = list.findIndex((r) => r.definition_id === openId);
        const here = list.findIndex((r) => r.definition_id === id);
        if (openIndex < 0 || here < 0) return null;
        return here === rowEnd(openIndex, columns(), list.length) ? list[openIndex] : null;
    };

    // View History — delegates to the parent; see onViewHistory's own doc
    // comment on MyAgentsListProps for why this can't be self-contained.
    const handleViewHistory = (row: RecentSessionRow): void => {
        closeMenu(row.definition_id);
        props.onViewHistory?.(row);
    };

    // Shut down: the one act that ends a running agent, with or without a pane.
    const shutdownBlock = (row: RecentSessionRow): string | undefined => {
        const runtime = props.runtimeOf?.(row.definition_id);
        return runtime && hasProcess(runtime.state) ? runtime.block_id : undefined;
    };
    const handleShutdown = (row: RecentSessionRow, blockId: string): void => {
        closeMenu(row.definition_id);
        void requestShutdown(blockId, row.instance_name || row.definition_name);
    };
    const handleDeleteOpen = (row: RecentSessionRow): void => openSubView(row, "delete");

    // The actions of the panel's first view.
    const rowActions = (row: RecentSessionRow, withDuplicate: boolean): RowAction[] => {
        const actions: RowAction[] = [
            { label: "Rename", icon: "pen", onSelect: () => handleRenameOpen(row) },
            ...(withDuplicate ? [{ label: "Duplicate", icon: "clone", onSelect: () => handleDuplicate(row) }] : []),
            { label: "View History", icon: "clock-rotate-left", onSelect: () => handleViewHistory(row) },
            { label: "Seat…", icon: "chair", testId: "agent-my-agents-seat", onSelect: () => handleSeatOpen(row) },
            {
                label: "Bundles…",
                icon: "layer-group",
                testId: "agent-my-agents-bundles",
                onSelect: () => handleBundlesOpen(row),
            },
            {
                label: "Environment…",
                icon: "terminal",
                testId: "agent-my-agents-env",
                onSelect: () => handleEnvOpen(row),
            },
        ];
        const blockId = shutdownBlock(row);
        if (blockId)
            actions.push({
                label: "Shut down",
                icon: "power-off",
                testId: "agent-my-agents-shutdown",
                onSelect: () => handleShutdown(row, blockId),
            });
        actions.push({
            label: "Delete",
            icon: "trash-can",
            testId: "agent-my-agents-delete",
            tone: "danger",
            onSelect: () => handleDeleteOpen(row),
        });
        return actions;
    };
    // While the delete is on its way the panel can be neither confirmed again nor dismissed.
    const [deleteBusy, setDeleteBusy] = createSignal(false);
    const confirmDelete = async (row: RecentSessionRow): Promise<void> => {
        if (deleteBusy()) return;
        setDeleteBusy(true);
        try {
            await RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id: row.definition_id });
        } catch (e: unknown) {
            setDeleteBusy(false);
            setSubView(null);
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Delete failed",
                message: e instanceof Error ? e.message : String(e),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
            return;
        }
        setDeleteBusy(false);
        setSubView(null);
        // Mark this row as a GENUINE delete before it leaves the store, so
        // the TransitionGroup's onExit handler (my-agents-list-exit.ts) knows to play the
        // real poof for it rather than the instant-removal path used for
        // ordinary filtering (codex P2 on PR #3280).
        setDeletingIds((prev) => new Set(prev).add(row.definition_id));
        // Optimistic local removal — don't wait on the "agents:changed"
        // broadcast round-trip before the exit animation can even start
        // (same "optimistic collapse" philosophy the menu-close above
        // already uses). Surgical `filter` on the STORE (not `rows()`
        // itself) so only this one row's identity is affected — the
        // eventual real refetch's reconcile pass is then a no-op for it.
        // See docs/specs/SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md §4.1.
        setRowsStore("list", (list) => list.filter((r) => r.definition_id !== row.definition_id));
        // Sweep EVERY pane open for this agent (spec §5.2). Not
        // `props.openDefinitions` — that map is keyed by definition, so an
        // agent open in two panes collapses to whichever block registered
        // last and the others would keep running against a deleted agent
        // (codex P2 on PR #3262). The map's shape is right for its other
        // caller (the fork prompt just needs *a* pane to switch to); a
        // sweep needs all of them.
        //
        // `slots` is this renderer's agent panes only, so srv's list of every
        // open agent pane (`agent.open-panes`) is added: a pane in another
        // window, and a terminal-tier agent's `term` pane, which has no slot
        // (SPEC_AGENT_DELETE_2026_09_16.md §5.2, #4911).
        //
        // Failures are reported, not swallowed: the agent and its
        // credentials are already gone by this point, so a pane that
        // wouldn't close is a live pane attached to a deleted agent, and
        // the user is the only one who can do anything about it.
        const sweep = new Set(getOpenBlockIdsForDefinition(row.definition_id));
        try {
            const open = await RpcApi.AgentOpenPanesCommand(TabRpcClient, {});
            for (const p of open?.panes ?? []) if (p.agent_id === row.definition_id) sweep.add(p.block_id);
        } catch (e) {
            // This renderer's own panes are still swept.
            Logger.warn("agent", "agent.open-panes failed during delete", { error: String(e) });
        }
        const failedBlockIds = (
            await Promise.all(
                [...sweep].map((blockId) =>
                    ObjectService.DeleteBlock(blockId).then(
                        () => null,
                        () => blockId
                    )
                )
            )
        ).filter((blockId): blockId is string => blockId !== null);
        if (failedBlockIds.length > 0) {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Agent deleted, but a pane stayed open",
                message:
                    `${failedBlockIds.length} pane${failedBlockIds.length === 1 ? "" : "s"} for ` +
                    `${row.instance_name || row.definition_name} could not be closed and ` +
                    `${failedBlockIds.length === 1 ? "is" : "are"} now running against a deleted ` +
                    `agent. Close ${failedBlockIds.length === 1 ? "it" : "them"} manually.`,
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 12000,
            });
        }
        // The row's removal above already triggers its exit animation; the
        // existing "agents:changed" refetch subscription (my-agents-list-data.ts) still
        // runs in the background and reconciles the rest of the list, but
        // is a no-op for this row since it's already gone from the store.
    };

    return {
        setRootRef,
        tileEls,
        measureList,
        panelAfter,
        isRowExpanded,
        toggleRowPanel,
        subView,
        setOpenMenuId,
        getRenameState,
        setRenameState,
        getForkState,
        setForkState,
        runsWithoutPane,
        openMap,
        deleteBusy,
        closePanels,
        handleOpen,
        handleAttachNoPane,
        handleOpenNewSession,
        handleSwitchToExisting,
        handleForkStart,
        handleForkCancel,
        handleRenameSubmit,
        handleRenameCancel,
        rowActions,
        confirmDelete,
    };
}

export type RowPanels = ReturnType<typeof createRowPanels>;
