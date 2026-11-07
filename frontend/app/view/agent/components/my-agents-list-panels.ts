// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The panel under a My agents tile: which row's panel is up and which of its views, the rename and fork-name
// states, opening / switching / attaching / forking, and the delete (with the sweep of the agent's panes).
// Split out of MyAgentsList.tsx (#13044 S6); MyAgentsRowPanel.tsx draws it.

import { errorDetail } from "@/app/notification/toast-debug";
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
import { batch, createEffect, createSignal, on, onCleanup, type Accessor, type Setter } from "solid-js";
import { requestShutdown } from "../shutdown/shut-down-agent";
import { createEnvDraft, type EnvDraft } from "./AgentEnvPanel";
import { createSeatForm, type SeatForm } from "./AgentSeatPanel";
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

/** What the panel's functions share: the list's props and data, the signals, and the drafts that outlive a re-render. */
interface RowPanelsState {
    props: MyAgentsListProps;
    data: Pick<MyAgentsData, "setRowsStore" | "setDeletingIds" | "sortedRows">;
    // Fork prompt state per row (keyed by definition_id)
    forkStates: Accessor<Map<string, ForkState>>;
    setForkStates: Setter<Map<string, ForkState>>;
    // Where the agents are open, read fresh when a panel opens (a pane can be in another window); until
    // then, and for a list that has no way to ask, the list's own map.
    freshOpen: Accessor<Map<string, string> | null>;
    setFreshOpen: Setter<Map<string, string> | null>;
    openMenuId: Accessor<string | null>;
    setOpenMenuId: Setter<string | null>;
    subView: Accessor<{ id: string; kind: SubView } | null>;
    setSubView: Setter<{ id: string; kind: SubView } | null>;
    // Rename — RenameAgentDefinitionTitleCommand already exists (used
    // today for fork/stack-tab double-click-rename); this is its first
    // My Agents row entry point. Keyed by definition_id like forkStates.
    renameStates: Accessor<Map<string, RenameState>>;
    setRenameStates: Setter<Map<string, RenameState>>;
    columns: Accessor<number>;
    setColumns: Setter<number>;
    // While the delete is on its way the panel can be neither confirmed again nor dismissed.
    deleteBusy: Accessor<boolean>;
    setDeleteBusy: Setter<boolean>;
    // What the Seat and Environment views hold, kept here and not in the views: a resize that changes the
    // column count moves the panel to the last tile of its row, which builds it again under that tile, and
    // the edits must come through. Dropped as soon as the panel is on another view or closed (so a new visit
    // starts from what is stored), not by the move, which leaves the view as it was.
    seatForms: Map<string, SeatForm>;
    envDrafts: Map<string, EnvDraft>;
    // The tiles by row id, for the panel's caret and its scroll.
    tileEls: Map<string, HTMLElement>;
    rootRef: HTMLDivElement | undefined;
}

function createRowPanelsState(props: MyAgentsListProps, data: RowPanelsState["data"]): RowPanelsState {
    const [forkStates, setForkStates] = createSignal<Map<string, ForkState>>(new Map());
    const [freshOpen, setFreshOpen] = createSignal<Map<string, string> | null>(null);
    // The panel under a tile (AgentRowPanel): one row's at a time, and the views of one row are
    // alternatives: the actions (the chevron), a rename, a fork name or the already-open choice (a tile
    // click), and Seat, Bundles, Environment and delete (reached from the actions). Opening one closes
    // the others, on this row and on every other. Like the New Agent panel it stays until it is closed
    // (its X, Esc, or the tile's chevron again), not on an outside click.
    const [openMenuId, setOpenMenuId] = createSignal<string | null>(null);
    const [subView, setSubView] = createSignal<{ id: string; kind: SubView } | null>(null);
    const [renameStates, setRenameStates] = createSignal<Map<string, RenameState>>(new Map());
    // The list's column count decides the tile the panel goes after: the last of the clicked tile's row, so
    // the row stays together.
    const [columns, setColumns] = createSignal(1);
    const [deleteBusy, setDeleteBusy] = createSignal(false);
    return {
        props,
        data,
        forkStates,
        setForkStates,
        freshOpen,
        setFreshOpen,
        openMenuId,
        setOpenMenuId,
        subView,
        setSubView,
        renameStates,
        setRenameStates,
        columns,
        setColumns,
        deleteBusy,
        setDeleteBusy,
        seatForms: new Map(),
        envDrafts: new Map(),
        tileEls: new Map(),
        rootRef: undefined,
    };
}

const getForkState = (s: RowPanelsState, definitionId: string): ForkState =>
    s.forkStates().get(definitionId) ?? { kind: "idle" };

const setForkState = (s: RowPanelsState, definitionId: string, state: ForkState): void => {
    s.setForkStates((prev) => {
        const next = new Map(prev);
        next.set(definitionId, state);
        return next;
    });
};

const openMap = (s: RowPanelsState): Map<string, string> =>
    s.freshOpen() ?? s.props.openDefinitions?.() ?? new Map<string, string>();

const runsWithoutPane = (s: RowPanelsState, definitionId: string): boolean => {
    const runtime = s.props.runtimeOf?.(definitionId);
    return !!s.props.onAttach && !!runtime && hasProcess(runtime.state) && !openMap(s).has(definitionId);
};

// A click anywhere on the tile, or on its chevron, opens the agent's panel or closes it. The panel holds
// what the click used to decide (open it, switch to its pane, attach it, fork it) and the row's actions.
function toggleRowPanel(s: RowPanelsState, row: RecentSessionRow): void {
    const id = row.definition_id;
    if (panelRowId(s) === id) {
        closePanels(s);
        return;
    }
    closePanels(s);
    s.setFreshOpen(null);
    s.setOpenMenuId(id);
    void s.props.resolveOpenDefinitions?.().then(
        (open) => {
            if (panelRowId(s) === id) s.setFreshOpen(open);
        },
        () => undefined
    );
}

// Open starts the agent. The panel's first look at where it is open can still be on its way (a pane in
// another window), so Open waits for it and, if the agent turns out to be open, leaves the panel up to
// offer Switch instead of starting a second one.
// The read is made again at the click: a pane can open or close in another window while the panel is up.
async function readOpenNow(s: RowPanelsState): Promise<Map<string, string> | null> {
    if (!s.props.resolveOpenDefinitions) return null;
    const open = await s.props.resolveOpenDefinitions().catch(() => null);
    if (open) s.setFreshOpen(open);
    return open;
}

async function handleOpen(s: RowPanelsState, row: RecentSessionRow): Promise<void> {
    const open = await readOpenNow(s);
    if (open?.has(row.definition_id)) {
        return;
    }
    closePanels(s);
    s.props.onReattach(row);
}

function handleAttachNoPane(s: RowPanelsState, row: RecentSessionRow): void {
    closePanels(s);
    s.props.onAttach?.(row);
}

async function handleOpenNewSession(s: RowPanelsState, row: RecentSessionRow): Promise<void> {
    setForkState(s, row.definition_id, { kind: "naming", label: "", loading: true, error: null });
    try {
        const result = await RpcApi.ForkAgentDefinitionSuggestCommand(TabRpcClient, {
            source_id: row.definition_id,
        });
        setForkState(s, row.definition_id, {
            kind: "naming",
            label: result.suggested_label,
            loading: false,
            error: null,
        });
    } catch {
        setForkState(s, row.definition_id, {
            kind: "naming",
            label: `${row.definition_name} #2`,
            loading: false,
            error: null,
        });
    }
}

async function handleSwitchToExisting(s: RowPanelsState, row: RecentSessionRow): Promise<void> {
    const blockId = ((await readOpenNow(s)) ?? openMap(s)).get(row.definition_id);
    closePanels(s);
    if (blockId) s.props.onSwitchToExisting?.(blockId);
}

async function handleForkStart(s: RowPanelsState, row: RecentSessionRow): Promise<void> {
    const fs = getForkState(s, row.definition_id);
    if (fs.kind !== "naming" || !fs.label.trim()) return;
    const label = fs.label.trim();
    setForkState(s, row.definition_id, { kind: "naming", label, loading: true, error: null, edited: fs.edited });
    try {
        await s.props.onFork?.(row, label);
        closePanels(s);
    } catch (err) {
        setForkState(s, row.definition_id, {
            kind: "naming",
            label,
            loading: false,
            error: err instanceof Error ? err.message : String(err),
            // A failed start keeps the panel (and its error) up whatever the name was.
            edited: true,
        });
    }
}

const handleForkCancel = (s: RowPanelsState, definitionId: string): void => {
    setForkState(s, definitionId, { kind: "idle" });
};

const seatFormOf = (s: RowPanelsState, id: string): SeatForm => {
    let form = s.seatForms.get(id);
    if (!form) s.seatForms.set(id, (form = createSeatForm()));
    return form;
};

const envDraftOf = (s: RowPanelsState, id: string): EnvDraft => {
    let draft = s.envDrafts.get(id);
    if (!draft) s.envDrafts.set(id, (draft = createEnvDraft()));
    return draft;
};

function watchSubViewDrafts(s: RowPanelsState): void {
    createEffect(
        on(
            s.subView,
            (sub) => {
                for (const id of [...s.seatForms.keys()])
                    if (!(sub?.kind === "seat" && sub.id === id)) s.seatForms.delete(id);
                for (const id of [...s.envDrafts.keys()])
                    if (!(sub?.kind === "env" && sub.id === id)) s.envDrafts.delete(id);
            },
            { defer: true }
        )
    );
}

/** Close every panel view except those of `keepId` (none given: all). */
function closePanels(s: RowPanelsState, keepId?: string): void {
    if (s.openMenuId() !== null && s.openMenuId() !== keepId) s.setOpenMenuId(null);
    const sub = s.subView();
    if (sub && sub.id !== keepId) s.setSubView(null);
    for (const id of [...s.renameStates().keys()]) if (id !== keepId) setRenameState(s, id, null);
    for (const [id, fs] of [...s.forkStates()])
        if (id !== keepId && fs.kind !== "idle") setForkState(s, id, { kind: "idle" });
}

const closeMenu = (s: RowPanelsState, definitionId: string): void => {
    s.setOpenMenuId((prev) => (prev === definitionId ? null : prev));
};

function onDocumentKeyDown(s: RowPanelsState, e: KeyboardEvent): void {
    // Escape in another pane must not close this pane's panel.
    if (e.key !== "Escape" || !eventBelongsToPaneOf(e, s.rootRef) || s.deleteBusy()) return;
    // A field inside the panel takes its own Escape (the name field cancels itself).
    const t = e.target;
    if (t instanceof HTMLElement && t.closest(".agent-row-panel") && t.matches("input, textarea")) return;
    closePanels(s);
}

function watchEscape(s: RowPanelsState): void {
    const onKeyDown = (e: KeyboardEvent) => onDocumentKeyDown(s, e);
    // In the capture phase: a field's own Escape can change the view and take the field out of the panel
    // before this runs, and the check above needs to see it still inside.
    document.addEventListener("keydown", onKeyDown, true);
    onCleanup(() => {
        document.removeEventListener("keydown", onKeyDown, true);
    });
}

// Seat, Bundles, Environment and delete are views of the row's panel (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3
// for the seat). Each was a modal over the picker.
// A change of view is one step: the open id must not be empty in between, or the panel is torn down and
// built again (and replays its opening) instead of changing its body.
const openSubView = (s: RowPanelsState, row: RecentSessionRow, kind: SubView): void =>
    batch(() => {
        closePanels(s);
        s.setSubView({ id: row.definition_id, kind });
    });

// Duplicate — reuses the EXISTING fork-and-launch flow unchanged
// (handleOpenNewSession/handleForkStart above, already wired to
// props.onFork = AgentPicker.handleFork); its naming step is the panel's fork-name view.
// Repo-owner-confirmed 2026-09-16: keep conversation history forking as-is
// (forkSession: true, unchanged).
const handleDuplicate = (s: RowPanelsState, row: RecentSessionRow): void =>
    batch(() => {
        closeMenu(s, row.definition_id);
        // Rename and Duplicate render two DIFFERENT panels, both anchored to
        // the same tile, from two independent state maps — so opening one while
        // the other was already open showed both name inputs stacked in one
        // row (ReAgent P2 on PR #3262). They are alternatives, not
        // companions: entering either closes the other.
        setRenameState(s, row.definition_id, null);
        void handleOpenNewSession(s, row);
    });

const getRenameState = (s: RowPanelsState, definitionId: string): RenameState | null =>
    s.renameStates().get(definitionId) ?? null;

const setRenameState = (s: RowPanelsState, definitionId: string, state: RenameState | null): void => {
    s.setRenameStates((prev) => {
        const next = new Map(prev);
        if (state === null) next.delete(definitionId);
        else next.set(definitionId, state);
        return next;
    });
};

const handleRenameOpen = (s: RowPanelsState, row: RecentSessionRow): void =>
    batch(() => {
        closeMenu(s, row.definition_id);
        // The other half of `handleDuplicate`'s mutual exclusion — see the
        // comment there.
        setForkState(s, row.definition_id, { kind: "idle" });
        setRenameState(s, row.definition_id, {
            label: row.instance_name || row.definition_name,
            loading: false,
            error: null,
        });
    });

const handleRenameCancel = (s: RowPanelsState, definitionId: string): void =>
    batch(() => {
        setRenameState(s, definitionId, null);
        s.setOpenMenuId(definitionId);
    });

async function handleRenameSubmit(s: RowPanelsState, row: RecentSessionRow): Promise<void> {
    const rs = getRenameState(s, row.definition_id);
    if (!rs || !rs.label.trim()) return;
    const title = rs.label.trim();
    setRenameState(s, row.definition_id, { ...rs, loading: true, error: null });
    try {
        await RpcApi.RenameAgentDefinitionTitleCommand(TabRpcClient, { id: row.definition_id, title });
        // Backend broadcasts "agents:changed" (template.rs) — the
        // existing subscription in my-agents-list-data.ts already refetches
        // and picks up the new name; nothing else to update here.
        setRenameState(s, row.definition_id, null);
    } catch (err) {
        setRenameState(s, row.definition_id, {
            label: title,
            loading: false,
            error: err instanceof Error ? err.message : String(err),
        });
    }
}

/** The row whose panel is showing: its actions, a rename, a fork name or choice, or a sub-view. */
const panelRowId = (s: RowPanelsState): string | null => {
    const sub = s.subView();
    if (sub) return sub.id;
    const menu = s.openMenuId();
    if (menu !== null) return menu;
    const renaming = s.renameStates().keys().next();
    if (!renaming.done) return renaming.value;
    for (const [id, fs] of s.forkStates()) if (fs.kind !== "idle") return id;
    return null;
};

const isRowExpanded = (s: RowPanelsState, definitionId: string): boolean => panelRowId(s) === definitionId;

const measureList = (s: RowPanelsState, ul: HTMLUListElement): void => {
    const measure = () =>
        s.setColumns(Math.max(1, getComputedStyle(ul).gridTemplateColumns.split(" ").filter(Boolean).length));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(ul);
    onCleanup(() => ro.disconnect());
};

/** The row whose panel goes right after row `id`'s tile, if that is where it goes. */
function panelAfter(s: RowPanelsState, id: string): RecentSessionRow | null {
    const openId = panelRowId(s);
    if (openId === null) return null;
    const list = s.data.sortedRows();
    const openIndex = list.findIndex((r) => r.definition_id === openId);
    const here = list.findIndex((r) => r.definition_id === id);
    if (openIndex < 0 || here < 0) return null;
    return here === rowEnd(openIndex, s.columns(), list.length) ? list[openIndex] : null;
}

// View History — delegates to the parent; see onViewHistory's own doc
// comment on MyAgentsListProps for why this can't be self-contained.
const handleViewHistory = (s: RowPanelsState, row: RecentSessionRow): void => {
    closeMenu(s, row.definition_id);
    s.props.onViewHistory?.(row);
};

// Shut down: the one act that ends a running agent, with or without a pane.
const shutdownBlock = (s: RowPanelsState, row: RecentSessionRow): string | undefined => {
    const runtime = s.props.runtimeOf?.(row.definition_id);
    return runtime && hasProcess(runtime.state) ? runtime.block_id : undefined;
};

const handleShutdown = (s: RowPanelsState, row: RecentSessionRow, blockId: string): void => {
    closeMenu(s, row.definition_id);
    void requestShutdown(blockId, row.instance_name || row.definition_name, s.props.paneBlockId);
};

// The actions of the panel's first view.
function rowActions(s: RowPanelsState, row: RecentSessionRow, withDuplicate: boolean): RowAction[] {
    const actions: RowAction[] = [
        { label: "Rename", icon: "pen", onSelect: () => handleRenameOpen(s, row) },
        ...(withDuplicate ? [{ label: "Duplicate", icon: "clone", onSelect: () => handleDuplicate(s, row) }] : []),
        { label: "View History", icon: "clock-rotate-left", onSelect: () => handleViewHistory(s, row) },
        {
            label: "Seat…",
            icon: "chair",
            testId: "agent-my-agents-seat",
            onSelect: () => openSubView(s, row, "seat"),
        },
        {
            label: "Bundles…",
            icon: "layer-group",
            testId: "agent-my-agents-bundles",
            onSelect: () => openSubView(s, row, "bundles"),
        },
        {
            label: "Environment…",
            icon: "terminal",
            testId: "agent-my-agents-env",
            onSelect: () => openSubView(s, row, "env"),
        },
    ];
    const blockId = shutdownBlock(s, row);
    if (blockId)
        actions.push({
            label: "Shut down",
            icon: "power-off",
            testId: "agent-my-agents-shutdown",
            onSelect: () => handleShutdown(s, row, blockId),
        });
    actions.push({
        label: "Delete",
        icon: "trash-can",
        testId: "agent-my-agents-delete",
        tone: "danger",
        onSelect: () => openSubView(s, row, "delete"),
    });
    return actions;
}

function notifyDeleteFailed(row: RecentSessionRow, e: unknown): void {
    pushNotification({
        icon: "fa-triangle-exclamation",
        title: "Delete failed",
        message: e instanceof Error ? e.message : String(e),
        timestamp: new Date().toISOString(),
        type: "error",
        expiration: Date.now() + 8000,
        debug: { agentId: row.definition_id, agentName: row.instance_name || row.definition_name, code: "agent-delete", detail: errorDetail(e) },
    });
}

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
// Returns the panes that would not close.
async function closeAgentPanes(row: RecentSessionRow): Promise<string[]> {
    const sweep = new Set(getOpenBlockIdsForDefinition(row.definition_id));
    try {
        const open = await RpcApi.AgentOpenPanesCommand(TabRpcClient, {});
        for (const p of open?.panes ?? []) if (p.agent_id === row.definition_id) sweep.add(p.block_id);
    } catch (e) {
        // This renderer's own panes are still swept.
        Logger.warn("agent", "agent.open-panes failed during delete", { error: String(e) });
    }
    return (
        await Promise.all(
            [...sweep].map((blockId) =>
                ObjectService.DeleteBlock(blockId).then(
                    () => null,
                    () => blockId
                )
            )
        )
    ).filter((blockId): blockId is string => blockId !== null);
}

// Failures are reported, not swallowed: the agent and its
// credentials are already gone by this point, so a pane that
// wouldn't close is a live pane attached to a deleted agent, and
// the user is the only one who can do anything about it.
function notifyPanesStayedOpen(row: RecentSessionRow, failedBlockIds: string[]): void {
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

async function confirmDelete(s: RowPanelsState, row: RecentSessionRow): Promise<void> {
    if (s.deleteBusy()) return;
    s.setDeleteBusy(true);
    try {
        await RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id: row.definition_id });
    } catch (e: unknown) {
        s.setDeleteBusy(false);
        s.setSubView(null);
        notifyDeleteFailed(row, e);
        return;
    }
    s.setDeleteBusy(false);
    s.setSubView(null);
    // Mark this row as a GENUINE delete before it leaves the store, so
    // the TransitionGroup's onExit handler (my-agents-list-exit.ts) knows to play the
    // real poof for it rather than the instant-removal path used for
    // ordinary filtering (codex P2 on PR #3280).
    s.data.setDeletingIds((prev) => new Set(prev).add(row.definition_id));
    // Optimistic local removal — don't wait on the "agents:changed"
    // broadcast round-trip before the exit animation can even start
    // (same "optimistic collapse" philosophy the menu-close above
    // already uses). Surgical `filter` on the STORE (not `rows()`
    // itself) so only this one row's identity is affected — the
    // eventual real refetch's reconcile pass is then a no-op for it.
    // See docs/specs/SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md §4.1.
    s.data.setRowsStore("list", (list) => list.filter((r) => r.definition_id !== row.definition_id));
    const failedBlockIds = await closeAgentPanes(row);
    if (failedBlockIds.length > 0) notifyPanesStayedOpen(row, failedBlockIds);
    // The row's removal above already triggers its exit animation; the
    // existing "agents:changed" refetch subscription (my-agents-list-data.ts) still
    // runs in the background and reconciles the rest of the list, but
    // is a no-op for this row since it's already gone from the store.
}

export function createRowPanels(
    props: MyAgentsListProps,
    data: Pick<MyAgentsData, "setRowsStore" | "setDeletingIds" | "sortedRows">
) {
    const s = createRowPanelsState(props, data);
    watchSubViewDrafts(s);
    watchEscape(s);

    return {
        setRootRef: (el: HTMLDivElement): void => {
            s.rootRef = el;
        },
        tileEls: s.tileEls,
        measureList: (ul: HTMLUListElement): void => measureList(s, ul),
        panelAfter: (id: string): RecentSessionRow | null => panelAfter(s, id),
        isRowExpanded: (definitionId: string): boolean => isRowExpanded(s, definitionId),
        toggleRowPanel: (row: RecentSessionRow): void => toggleRowPanel(s, row),
        subView: s.subView,
        seatFormOf: (id: string): SeatForm => seatFormOf(s, id),
        envDraftOf: (id: string): EnvDraft => envDraftOf(s, id),
        setOpenMenuId: s.setOpenMenuId,
        getRenameState: (definitionId: string): RenameState | null => getRenameState(s, definitionId),
        setRenameState: (definitionId: string, state: RenameState | null): void =>
            setRenameState(s, definitionId, state),
        getForkState: (definitionId: string): ForkState => getForkState(s, definitionId),
        setForkState: (definitionId: string, state: ForkState): void => setForkState(s, definitionId, state),
        runsWithoutPane: (definitionId: string): boolean => runsWithoutPane(s, definitionId),
        openMap: (): Map<string, string> => openMap(s),
        deleteBusy: s.deleteBusy,
        closePanels: (keepId?: string): void => closePanels(s, keepId),
        handleOpen: (row: RecentSessionRow): Promise<void> => handleOpen(s, row),
        handleAttachNoPane: (row: RecentSessionRow): void => handleAttachNoPane(s, row),
        handleOpenNewSession: (row: RecentSessionRow): Promise<void> => handleOpenNewSession(s, row),
        handleSwitchToExisting: (row: RecentSessionRow): Promise<void> => handleSwitchToExisting(s, row),
        handleForkStart: (row: RecentSessionRow): Promise<void> => handleForkStart(s, row),
        handleForkCancel: (definitionId: string): void => handleForkCancel(s, definitionId),
        handleRenameSubmit: (row: RecentSessionRow): Promise<void> => handleRenameSubmit(s, row),
        handleRenameCancel: (definitionId: string): void => handleRenameCancel(s, definitionId),
        rowActions: (row: RecentSessionRow, withDuplicate: boolean): RowAction[] => rowActions(s, row, withDuplicate),
        confirmDelete: (row: RecentSessionRow): Promise<void> => confirmDelete(s, row),
    };
}

export type RowPanels = ReturnType<typeof createRowPanels>;
