// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MyAgentsList — the top section of the two-tier AgentPicker
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md, Phase 1). Lists the user's
 * own agents (Maks, AgentY, etc.) — formerly known as "Recent sessions"
 * before the rename. Templates (Claude Code, Codex CLI, …) live in the
 * sibling Templates section below this one and are not surfaced here.
 *
 * Why "Recent sessions" became "My Agents" — under Option E session
 * zones are anchored to the agent definition, so a user agent has
 * exactly one "current" session by construction. The list is really
 * "your agents, with their current state", not "sessions across all
 * agents". Naming follows the new model.
 *
 * Renamed from RecentSessionsList (the cascade follow-up file
 * shipped in PR #977 + #1008). The reattach mechanism, data source, and
 * row UI are unchanged from that file — only the labels move. The data
 * source stays `ListRecentSessionsCommand` because the row UI uses
 * per-instance preview + node_count + last_active_at, which are only on
 * `RecentSessionRow` (NamedAgentRow doesn't carry them). Switching the
 * RPC would require backend changes and is out of Phase 1 scope.
 *
 * Reattach mechanism (unchanged): each entry triggers a normal
 * definition launch through `AgentViewModel.launchAgentDefinition` with
 * `continueOfInstanceId` + `workDirOverride` set from the row. The new
 * pane spawns the CLI in the prior working directory, so Claude's
 * `--continue` (and equivalents) resumes the session and the new pane's
 * `output.state.json` snapshot path picks up the conversation history
 * on render.
 *
 * Fork prompt (feat/agent-session-fork): when a row's definition is
 * already open in another pane, clicking it shows an inline prompt
 * instead of immediately reattaching. The user can either fork into a
 * new named session or switch focus to the existing pane.
 *
 * An agent that runs with no pane (always-available, parked) gets the same kind of
 * choice: "Open it" attaches it to this pane, "Fork" makes a named copy. Its row has a
 * dashed border and a "no pane" tag, so it doesn't look like any idle card.
 */

import { useTick } from "@/app/hook/useTick";
import { hasProcess } from "@/app/store/agent-runtime";
import { requestShutdown } from "../shutdown/shut-down-agent";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { eventBelongsToPaneOf } from "@/util/focusutil";
import {
    createEffect,
    createMemo,
    createResource,
    createSignal,
    For,
    Match,
    onCleanup,
    Show,
    Switch,
    type Accessor,
    type JSX,
} from "solid-js";
import clsx from "clsx";
import { createStore, reconcile } from "solid-js/store";
import { TransitionGroup } from "solid-transition-group";

import { Button, IconButton, Pill } from "@/app/element/ui";
import { rowEnd } from "@/app/element/ui/CardGrid";
import { AgentBundlesTab } from "./AgentBundlesTab";
import { AgentEnvPanel } from "./AgentEnvPanel";
import { AgentRowPanel } from "./AgentRowPanel";
import { AgentSeatPanel } from "./AgentSeatPanel";
import { NamePrompt } from "./NamePrompt";
import { getOpenBlockIdsForDefinition } from "@/app/store/agent-pane-state-store";
import { shouldReduceMotion } from "@/app/motion/motion";
import { pushNotification } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ObjectService } from "@/app/store/services";
import { literalFirstSearch } from "@/app/util/fuzzysearch";
import { DualProviderLogo } from "@/element/DualProviderLogo";
import { formatTimeAgo } from "@/util/format-time";
import { Logger } from "@/util/logger";
import type { OpenAgentLocation } from "../open-agent-panes";
import { resolveEffectiveVendor } from "../providers/catalog";
import { seatBadgeText, seatReportsLine, seatTooltipLine } from "../seat-format";
import type { AgentSortOption } from "./AgentPickerFilterBar";
import { AgentRuntimeChip } from "./AgentRuntimeChip";
import { RuntimeBadge } from "./RuntimeBadge";

import { WpsEvent } from "@/app/store/mps-events";
import { compareRows, EMPTY_FILTERED, EMPTY_GLOBAL, FETCH_ERROR, noMatchText, noPreviewText } from "./my-agents-list-text";
export { EMPTY_FILTERED, EMPTY_GLOBAL, FETCH_ERROR, noMatchText };
/** Backend hard cap (`CommandListRecentSessionsData`, instance.rs) — the
 * limit MyAgentsList requests once a name filter is active, so filtering
 * covers the full available set instead of just the default first page. */
const SEARCH_LIMIT = 100;

export interface MyAgentsListProps {
    /** Optional reactive accessor for the identity filter. `null` /
     *  undefined / empty string = no filter (show every identity). */
    identityId?: Accessor<string | null | undefined>;
    /** Visible cap. Backend caps at 100; default is 20. */
    limit?: number;
    /** Optional reactive accessor for a name filter (matched against
     *  `instance_name || definition_name`, case-insensitive substring).
     *  `null` / undefined / empty string = no filter (show every row).
     *  When non-empty, the fetch limit is bumped to `SEARCH_LIMIT` so the
     *  filter searches the full backend-capped set, not just the default
     *  page — see SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md. */
    nameFilter?: Accessor<string | null | undefined>;
    /** Optional reactive accessor for the sort order — client-side, over
     *  the already-fetched/filtered row set (no backend RPC change).
     *  Defaults to `"recent"` (today's implicit backend ordering) when
     *  omitted. See docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §3. */
    sortBy?: Accessor<AgentSortOption>;
    /** Called when the user clicks an entry that is NOT currently open
     *  in another pane. The parent (AgentPicker) hands this to
     *  `AgentViewModel.launchAgentDefinition` with the continuation
     *  overrides — see "Reattach mechanism" above. */
    onReattach: (row: RecentSessionRow) => void;
    /**
     * Map of definition_id → blockId for definitions currently open in
     * another pane. When a row's definition_id is in this map, clicking
     * the row shows the fork prompt instead of calling onReattach.
     */
    openDefinitions?: Accessor<Map<string, string>>;
    /**
     * definition_id → where that open pane is (this window or another),
     * for the active badge's tooltip and the "already open" prompt.
     * Optional: without it the wording stays "in another pane".
     */
    openLocations?: Accessor<Map<string, OpenAgentLocation>>;
    /**
     * A fresh, instance-wide read of `openDefinitions`, awaited on row click
     * before deciding between the "already open" prompt and a reattach — the
     * rendered map may not have srv's answer yet. Optional: without it the
     * click reads `openDefinitions` as before.
     */
    resolveOpenDefinitions?: () => Promise<Map<string, string>>;
    /**
     * Called after the user confirms a fork: fork has been created and
     * the new definition should be launched. Receives the new AgentDefinition.
     */
    onFork?: (row: RecentSessionRow, branchLabel: string) => Promise<void>;
    /**
     * Called when the user clicks "Switch to existing" in the fork
     * prompt. Receives the blockId of the already-open pane.
     */
    onSwitchToExisting?: (blockId: string) => void;
    /**
     * Called exactly once, the first time this component's own
     * ListRecentSessionsCommand resource resolves (success OR failure —
     * `rows()` transitions away from `undefined` either way). Lets a parent
     * that also has its own async gate (AgentPicker's `agents()` list) hold
     * a single combined loading overlay until BOTH are ready, instead of
     * this component's own empty-`<ul>`-while-loading state flashing
     * separately underneath. Never re-fires on background refetches
     * (visibility regain, agents:changed) — those correctly keep showing
     * the already-loaded list via Solid's stale-while-revalidate and have
     * nothing to do with "first load."
     */
    onFirstLoad?: () => void;
    /**
     * Row-menu "View History" action — opens a read-only history tab for
     * the row's agent. Delegates up to the parent (AgentPicker) because
     * `openOrFocusHistoryTab` needs the PICKER's OWN blockId (the tab this
     * list is rendered inside) to push the history tab onto, which this
     * component has no reason to hold itself.
     * docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.3.
     */
    onViewHistory?: (row: RecentSessionRow) => void;
    /** The agent's runtime state (store/agent-runtime.ts), for the chip on its row. */
    runtimeOf?: (definitionId: string) => AgentRuntime | undefined;
    /**
     * Called when the user clicks an agent that runs with no pane shown anywhere: it attaches to the
     * process that is running, where `onReattach` would start another one.
     */
    onAttach?: (row: RecentSessionRow) => void;
}

type ForkState =
    | { kind: "idle" }
    // name input expanded; `edited`: the user changed the suggested name, so a click outside no longer dismisses it
    | { kind: "naming"; label: string; loading: boolean; error: string | null; edited?: boolean };

export const MyAgentsList = (props: MyAgentsListProps): JSX.Element => {
    const filterId = createMemo(() => {
        const raw = props.identityId?.();
        if (raw === null || raw === undefined) return "";
        return String(raw).trim();
    });

    // Trimmed query, kept in its original case for display (the no-match
    // message echoes back what the user typed). `nameQuery` below is the
    // lowercased form used for matching.
    const rawQuery = createMemo(() => {
        const raw = props.nameFilter?.();
        if (raw === null || raw === undefined) return "";
        return String(raw).trim();
    });
    const nameQuery = createMemo(() => rawQuery().toLowerCase());
    const isSearching = createMemo(() => nameQuery().length > 0);

    // Compound resource key: a plain string so Solid's resource-source
    // memoization (value equality) only triggers a refetch when the
    // identity filter changes OR searching flips on/off — NOT on every
    // keystroke while already searching. An object key would defeat this
    // (a fresh object each recomputation is never `Object.is`-equal to the
    // last one), so the two parts are joined into one primitive instead of
    // passed as `{ id, searching }`.
    const resourceKey = createMemo(() => `${filterId()} ${isSearching() ? "1" : "0"}`);

    // Separate from the resource's own value: `rows()` still resolves to
    // `[]` on a failed fetch (see the catch below) so nothing here needs
    // to special-case Solid's throw-on-read error-state accessor — this
    // signal is the ONLY thing that distinguishes "backend call failed"
    // from "genuinely zero agents" for the render below. Cleared at the
    // start of every fetch so a stale error doesn't survive into a fresh
    // attempt's loading state.
    const [fetchError, setFetchError] = createSignal(false);
    // Guards the two `setFetchError(true)` calls below against a stale
    // (superseded) fetch's late resolution overwriting a NEWER fetch's
    // already-settled state — e.g. rapid identity-filter switching, or
    // clicking Retry again before the first attempt has finished, could
    // otherwise let an old failure paint the error panel over valid,
    // already-loaded data from the fetch that actually matters now
    // (reagent P1 on PR #2327's re-review). Incremented synchronously at
    // the start of each fetcher call, so invocation order is always
    // correct even though resolution order isn't.
    let fetchGeneration = 0;

    const [rows, { refetch }] = createResource<RecentSessionRow[], string>(
        // Key the resource on identity + whether a name search is active
        // (see resourceKey's own comment) so changes to either refetch.
        resourceKey,
        async () => {
            // Read the current values directly rather than parsing
            // `resourceKey`'s string — the key's only job is to give
            // Solid's resource-source memoization a primitive to dedupe
            // on (see resourceKey's own comment); by the time this
            // fetcher runs, `filterId()`/`isSearching()` already reflect
            // whatever change just triggered it.
            const id = filterId();
            const myGeneration = ++fetchGeneration;
            setFetchError(false);
            try {
                const result = await RpcApi.ListRecentSessionsCommand(TabRpcClient, {
                    limit: isSearching() ? SEARCH_LIMIT : (props.limit ?? 20),
                    // Backend treats "" as "no filter" — see
                    // CommandListRecentSessionsData docs.
                    identity_id: id,
                });
                // Zero rows AND a reported degradation means a backend data
                // source failed and we got nothing back — NOT a trustworthy
                // "you have no agents." A healthy call with genuinely zero
                // agents never populates `degraded` (session.rs's six
                // sources only degrade on their own error, never on "found
                // nothing"). Partial degradation alongside real rows (e.g.
                // identity lookups failing but the registry/defs succeeding)
                // is left alone here — those rows still render, just with
                // the account line's own fallback ("No auth" or no line).
                if (result.rows.length === 0 && result.degraded.length > 0) {
                    Logger.error("agent", "MyAgentsList: listrecentsessions reported degraded sources with zero rows", {
                        degraded: result.degraded,
                        identityId: id,
                    });
                    if (myGeneration === fetchGeneration) setFetchError(true);
                    return [];
                }
                return result.rows;
            } catch (e) {
                // Was a silent `catch { return []; }` — indistinguishable from
                // "genuinely no sessions" in the UI and left zero trace
                // anywhere (not even a console entry), which is exactly what
                // made a real regression here look like expected empty state.
                // `Error` objects serialize to `{}` via structured/JSON clone
                // (message/stack aren't own-enumerable) — pull them out explicitly.
                const errInfo =
                    e instanceof Error ? { name: e.name, message: e.message, stack: e.stack } : { value: String(e) };
                Logger.error("agent", "MyAgentsList: ListRecentSessionsCommand failed", {
                    error: errInfo,
                    identityId: id,
                });
                if (myGeneration === fetchGeneration) setFetchError(true);
                return [];
            }
        }
    );

    // Mirror `rows()` into a keyed store so `<For>`/`<TransitionGroup>` can
    // tell "one row removed" from "list replaced" across a refetch.
    // `createResource`'s own signal hands back a brand-new array of
    // brand-new objects on every refetch, which `<For>`'s reference-based
    // reconciliation can't diff against the previous array at all — in
    // practice every row would unmount/remount on every "agents:changed"
    // refetch, not just the deleted one, which also defeats any per-row
    // exit animation. `reconcile(..., { key: "definition_id" })` is the
    // same idiom `drone-model.ts`'s `setDraft` already uses for its store.
    // See docs/specs/SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md §4.1.
    const [rowsStore, setRowsStore] = createStore<{ list: RecentSessionRow[] }>({ list: [] });
    createEffect(() => {
        setRowsStore("list", reconcile(rows() ?? [], { key: "definition_id" }));
    });

    // definition_ids genuinely being deleted right now (added in
    // confirmDelete, before the row leaves rowsStore.list) — NOT every row
    // that happens to drop out of `sortedRows()`. A name-filter narrowing
    // the visible set, or a background refetch reconciling away a row for
    // some other reason, also removes it from the array `<TransitionGroup>`
    // watches; without this distinction the poof animation would fire for
    // ordinary filtering too, misrepresenting it as a delete (codex P2 on
    // PR #3280). Read by the TransitionGroup's onExit handler below (§4.2/
    // §4.3 of SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md) to decide
    // whether an exiting row gets the real animation or an instant removal,
    // and by `hasPendingDeleteExit` to keep the list mounted (reagent P1 +
    // codex P2 on the same PR: the empty/no-match `<Show>` gate used to
    // unmount the whole `<ul>`/`TransitionGroup` — killing the animation
    // outright — the instant the last visible row's optimistic removal
    // made the list look empty).
    const [deletingIds, setDeletingIds] = createSignal<Set<string>>(new Set());
    const hasPendingDeleteExit = () => deletingIds().size > 0;

    // Re-poll on visibility regain so a session that just ended in
    // another pane shows up at the top without the user having to
    // re-open the picker. createEffect runs after first render too,
    // so the initial subscribe doesn't double-fetch.
    const onVisible = () => {
        if (document.visibilityState === "visible") void refetch();
    };
    document.addEventListener("visibilitychange", onVisible);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisible));

    // Refetch when a new agent definition is created (e.g. via agent.define)
    // so the stub instance appears immediately without needing a restart.
    const unsubAgents = onMuxEvent(WpsEvent.AgentsChanged, () => void refetch());
    onCleanup(unsubAgents);

    const minuteTick = useTick(60_000);
    const now = createMemo(() => (minuteTick(), Date.now()));

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
    const handleOpen = async (row: RecentSessionRow): Promise<void> => {
        let open = freshOpen();
        if (!open && props.resolveOpenDefinitions) open = await props.resolveOpenDefinitions().catch(() => null);
        if (open?.has(row.definition_id)) {
            setFreshOpen(open);
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

    const handleSwitchToExisting = (row: RecentSessionRow) => {
        const blockId = openMap().get(row.definition_id);
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
    type SubView = "seat" | "bundles" | "env" | "delete";
    const [openMenuId, setOpenMenuId] = createSignal<string | null>(null);
    const [subView, setSubView] = createSignal<{ id: string; kind: SubView } | null>(null);
    const isMenuOpen = (definitionId: string): boolean => openMenuId() === definitionId;
    /** Close every panel view except those of `keepId` (none given: all). */
    const closePanels = (keepId?: string): void => {
        if (openMenuId() !== null && openMenuId() !== keepId) setOpenMenuId(null);
        const sub = subView();
        if (sub && sub.id !== keepId) setSubView(null);
        for (const id of [...renameStates().keys()]) if (id !== keepId) setRenameState(id, null);
        for (const [id, fs] of [...forkStates()]) if (id !== keepId && fs.kind !== "idle") setForkState(id, { kind: "idle" });
    };
    const closeMenu = (definitionId: string): void => {
        setOpenMenuId((prev) => (prev === definitionId ? null : prev));
    };
    let rootRef: HTMLDivElement | undefined;
    const onDocumentKeyDown = (e: KeyboardEvent) => {
        // Escape in another pane must not close this pane's panel.
        if (e.key !== "Escape" || !eventBelongsToPaneOf(e, rootRef)) return;
        // A field inside the panel takes its own Escape (the name field cancels itself).
        const t = e.target;
        if (t instanceof HTMLElement && t.closest(".agent-row-panel") && t.matches("input, textarea")) return;
        closePanels();
    };
    document.addEventListener("keydown", onDocumentKeyDown);
    onCleanup(() => {
        document.removeEventListener("keydown", onDocumentKeyDown);
    });

    // Seat, Bundles, Environment and delete are views of the row's panel (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3
    // for the seat). Each was a modal over the picker.
    const openSubView = (row: RecentSessionRow, kind: SubView): void => {
        closePanels();
        setSubView({ id: row.definition_id, kind });
    };
    const handleSeatOpen = (row: RecentSessionRow): void => openSubView(row, "seat");
    const handleBundlesOpen = (row: RecentSessionRow): void => openSubView(row, "bundles");
    const handleEnvOpen = (row: RecentSessionRow): void => openSubView(row, "env");
    // Duplicate — reuses the EXISTING fork-and-launch flow unchanged
    // (handleOpenNewSession/handleForkStart above, already wired to
    // props.onFork = AgentPicker.handleFork). The naming UI that flow
    // drives (the `agent-fork-prompt` block below) is already rendered
    // per-row regardless of what triggered it; only its "already open in
    // another pane" message needed to become conditional (see JSX below),
    // since Duplicate reaches "naming" state directly without going
    // through "prompt" first. Repo-owner-confirmed 2026-09-16: keep
    // conversation history forking as-is (forkSession: true, unchanged).
    const handleDuplicate = (row: RecentSessionRow): void => {
        closeMenu(row.definition_id);
        // Rename and Duplicate render two DIFFERENT panels, both anchored to
        // the same tile, from two independent state maps — so opening one while
        // the other was already open showed both name inputs stacked in one
        // row (ReAgent P2 on PR #3262). They are alternatives, not
        // companions: entering either closes the other.
        setRenameState(row.definition_id, null);
        void handleOpenNewSession(row);
    };

    // Rename — RenameAgentDefinitionTitleCommand already exists (used
    // today for fork/stack-tab double-click-rename); this is its first
    // My Agents row entry point. Keyed by definition_id like forkStates.
    interface RenameState {
        label: string;
        loading: boolean;
        error: string | null;
    }
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
    const handleRenameOpen = (row: RecentSessionRow): void => {
        closeMenu(row.definition_id);
        // The other half of `handleDuplicate`'s mutual exclusion — see the
        // comment there.
        setForkState(row.definition_id, { kind: "idle" });
        setRenameState(row.definition_id, {
            label: row.instance_name || row.definition_name,
            loading: false,
            error: null,
        });
    };
    const handleRenameCancel = (definitionId: string): void => {
        setRenameState(definitionId, null);
        setOpenMenuId(definitionId);
    };
    const handleRenameSubmit = async (row: RecentSessionRow): Promise<void> => {
        const rs = getRenameState(row.definition_id);
        if (!rs || !rs.label.trim()) return;
        const title = rs.label.trim();
        setRenameState(row.definition_id, { ...rs, loading: true, error: null });
        try {
            await RpcApi.RenameAgentDefinitionTitleCommand(TabRpcClient, { id: row.definition_id, title });
            // Backend broadcasts "agents:changed" (template.rs) — the
            // existing subscription above (line ~334) already refetches
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

    type PanelView = "actions" | "rename" | "fork-name" | SubView;
    const PANEL_SUBTITLE: Record<PanelView, string | undefined> = {
        actions: undefined,
        rename: "Rename",
        "fork-name": "New session",
        seat: "Seat",
        bundles: "Bundles",
        env: "Environment",
        delete: "Delete",
    };
    // The choices are the answer to a click on the tile, so they have nothing to go back to.
    const PANEL_HAS_BACK: ReadonlySet<PanelView> = new Set(["rename", "fork-name", "seat", "bundles", "env", "delete"]);

    const renderRowPanel = (row: RecentSessionRow): JSX.Element => {
        const id = row.definition_id;
        const name = row.instance_name || row.definition_name;
        const view = (): PanelView => {
            const sub = subView();
            if (sub && sub.id === id) return sub.kind;
            if (getRenameState(id)) return "rename";
            if (getForkState(id).kind === "naming") return "fork-name";
            return "actions";
        };
        const isActive = () => openMap().has(id);
        const withoutPane = () => runsWithoutPane(id);
        const openWhere = () => props.openLocations?.().get(id)?.label ?? "open in another pane";
        // What the panel says about the agent before its choices.
        const note = (): string | undefined =>
            isActive() ? `is already ${openWhere()}.` : withoutPane() ? "is running with no pane." : undefined;
        const naming = () => {
            const fs = getForkState(id);
            return fs.kind === "naming" ? fs : null;
        };
        const toActions = (): void => {
            closePanels();
            setOpenMenuId(id);
        };
        return (
            <AgentRowPanel
                title={name}
                subtitle={PANEL_SUBTITLE[view()]}
                viewKey={view()}
                tile={() => tileEls.get(id)}
                onClose={() => closePanels()}
                onBack={PANEL_HAS_BACK.has(view()) ? toActions : undefined}
            >
                <Switch>
                    <Match when={view() === "actions"}>
                        <div class="agent-row-panel-actions-view" data-testid="agent-row-actions">
                            <Show when={note()}>
                                {(text) => (
                                    <p class="agent-row-panel-note">
                                        <strong>{name}</strong> {text()}
                                    </p>
                                )}
                            </Show>
                            <div class="agent-row-panel-actions" data-testid="agent-row-open">
                                <Switch>
                                    <Match when={isActive()}>
                                        <Button
                                            tone="accent"
                                            icon="right-left"
                                            data-testid="agent-fork-switch"
                                            onClick={() => handleSwitchToExisting(row)}
                                        >
                                            Switch to existing
                                        </Button>
                                        <Button icon="code-fork" data-testid="agent-fork-open-new" onClick={() => void handleOpenNewSession(row)}>
                                            Fork…
                                        </Button>
                                    </Match>
                                    <Match when={withoutPane()}>
                                        <Button
                                            tone="accent"
                                            icon="arrow-right-to-bracket"
                                            data-testid="agent-fork-attach"
                                            onClick={() => handleAttachNoPane(row)}
                                        >
                                            Open it
                                        </Button>
                                        <Button icon="code-fork" data-testid="agent-fork-fork" onClick={() => void handleOpenNewSession(row)}>
                                            Fork…
                                        </Button>
                                    </Match>
                                    <Match when={true}>
                                        <Button tone="accent" icon="arrow-right-to-bracket" data-testid="agent-my-agents-open" onClick={() => void handleOpen(row)}>
                                            Open
                                        </Button>
                                    </Match>
                                </Switch>
                            </div>
                            <div class="agent-row-panel-actions" data-testid="agent-row-more">
                                <For each={rowActions(row, !isActive() && !withoutPane())}>
                                    {(action) => (
                                        <Button icon={action.icon} tone={action.tone} onClick={action.onSelect} data-testid={action.testId}>
                                            {action.label}
                                        </Button>
                                    )}
                                </For>
                            </div>
                        </div>
                    </Match>
                    <Match when={view() === "rename"}>
                        <Show when={getRenameState(id)}>
                            {(rs) => (
                                <div data-testid="agent-rename-prompt">
                                    <NamePrompt
                                        label="Rename to:"
                                        placeholder="Agent name"
                                        value={rs().label}
                                        loading={rs().loading}
                                        error={rs().error}
                                        submitLabel="Save"
                                        inputTestid="agent-rename-input"
                                        submitTestid="agent-rename-save"
                                        onInput={(label) => setRenameState(id, { ...rs(), label })}
                                        onSubmit={() => void handleRenameSubmit(row)}
                                        onCancel={() => handleRenameCancel(id)}
                                    />
                                </div>
                            )}
                        </Show>
                    </Match>
                    <Match when={view() === "fork-name"}>
                        <Show when={naming()}>
                            {(ns) => (
                                <div data-testid="agent-fork-prompt">
                                    <Show when={isActive()}>
                                        <p class="agent-row-panel-note">
                                            <strong>{name}</strong> is already {openWhere()}.
                                        </p>
                                    </Show>
                                    <NamePrompt
                                        label="Name for new session:"
                                        placeholder="Session name"
                                        value={ns().label}
                                        loading={ns().loading}
                                        error={ns().error}
                                        submitLabel="Start"
                                        inputTestid="agent-fork-name-input"
                                        submitTestid="agent-fork-start"
                                        onInput={(label) =>
                                            setForkState(id, { kind: "naming", label, loading: false, error: null, edited: true })
                                        }
                                        onSubmit={() => void handleForkStart(row)}
                                        onCancel={() => {
                                            handleForkCancel(id);
                                            setOpenMenuId(id);
                                        }}
                                    />
                                </div>
                            )}
                        </Show>
                    </Match>
                    <Match when={view() === "seat"}>
                        <div class="agent-row-panel-embed">
                            <AgentSeatPanel agentId={id} agentName={name} onClose={toActions} />
                        </div>
                    </Match>
                    <Match when={view() === "bundles"}>
                        <div class="agent-row-panel-embed">
                            <AgentBundlesTab agentId={id} onClose={toActions} />
                        </div>
                    </Match>
                    <Match when={view() === "env"}>
                        <div class="agent-row-panel-embed">
                            <AgentEnvPanel agentId={id} agentName={name} onClose={toActions} />
                        </div>
                    </Match>
                    <Match when={view() === "delete"}>
                        <div data-testid="agent-delete-confirm">
                            <p class="agent-row-panel-note">
                                This permanently deletes the agent and its credentials, skills, and activity history. Its bundle and any
                                manually-saved transcripts are not affected. This cannot be undone.
                            </p>
                            <div class="agent-row-panel-actions">
                                <Button tone="danger" icon="trash-can" data-testid="agent-delete-confirm-yes" onClick={() => void confirmDelete(row)}>
                                    Delete {name}
                                </Button>
                                <Button onClick={toActions}>Cancel</Button>
                            </div>
                        </div>
                    </Match>
                </Switch>
            </AgentRowPanel>
        );
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

    interface RowAction {
        label: string;
        icon: string;
        testId?: string;
        tone?: "danger";
        onSelect: () => void;
    }
    // The actions of the panel's first view.
    const rowActions = (row: RecentSessionRow, withDuplicate: boolean): RowAction[] => {
        const actions: RowAction[] = [
            { label: "Rename", icon: "pen", onSelect: () => handleRenameOpen(row) },
            ...(withDuplicate ? [{ label: "Duplicate", icon: "clone", onSelect: () => handleDuplicate(row) }] : []),
            { label: "View History", icon: "clock-rotate-left", onSelect: () => handleViewHistory(row) },
            { label: "Seat…", icon: "chair", testId: "agent-my-agents-seat", onSelect: () => handleSeatOpen(row) },
            { label: "Bundles…", icon: "layer-group", testId: "agent-my-agents-bundles", onSelect: () => handleBundlesOpen(row) },
            { label: "Environment…", icon: "terminal", testId: "agent-my-agents-env", onSelect: () => handleEnvOpen(row) },
        ];
        const blockId = shutdownBlock(row);
        if (blockId) actions.push({ label: "Shut down", icon: "power-off", testId: "agent-my-agents-shutdown", onSelect: () => handleShutdown(row, blockId) });
        actions.push({ label: "Delete", icon: "trash-can", testId: "agent-my-agents-delete", tone: "danger", onSelect: () => handleDeleteOpen(row) });
        return actions;
    };
    const confirmDelete = async (row: RecentSessionRow): Promise<void> => {
        try {
            await RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id: row.definition_id });
        } catch (e: unknown) {
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
        setSubView(null);
        // Mark this row as a GENUINE delete before it leaves the store, so
        // the TransitionGroup's onExit handler (below) knows to play the
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
        // existing "agents:changed" refetch subscription (line ~334) still
        // runs in the background and reconciles the rest of the list, but
        // is a no-op for this row since it's already gone from the store.
    };

    // Surfacing rules:
    // - rows.loading              → still loading (skeleton hint) — uses the
    //                                RESOURCE's own loading flag, not
    //                                `rows() === undefined`: Solid's
    //                                createResource keeps the previous
    //                                value visible while a refetch is in
    //                                flight (stale-while-revalidate), so
    //                                after Retry on a failed fetch, `rows()`
    //                                is still `[]` from the failed attempt
    //                                even though a fresh request is
    //                                pending. Checking `rows() === undefined`
    //                                only catches the very first load — a
    //                                retry would fall straight through to
    //                                the empty branch below for its entire
    //                                in-flight duration, flashing "No agents
    //                                yet" and recreating the exact
    //                                error/empty ambiguity this fix exists
    //                                to remove (codex P2 on PR #2327's
    //                                post-merge re-review).
    // - fetchError()              → error state (retry affordance), never
    //                                confused with a genuinely empty list
    // - rows [] (fetch succeeded) → empty state (filter-aware copy)
    // - rows non-empty            → list
    //
    // `rowsStore.list`, not `rows()`, for the length checks below — reagent
    // P1 on PR #3280: confirmDelete's optimistic removal (§4.1 of
    // SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md) empties
    // `rowsStore.list` immediately, but `rows()` itself stays stale until
    // the background "agents:changed" refetch resolves. Reading `rows()`
    // here mismatched isEmpty/isNoMatch against what filteredRows/
    // sortedRows (already on rowsStore.list) actually render — deleting
    // the last agent showed "No agents match" instead of the real empty
    // state for that window.
    const isLoading = () => rows.loading;
    const isEmpty = () => !isLoading() && !fetchError() && rowsStore.list.length === 0;

    // Report the first resolution up to the parent (see onFirstLoad's own
    // doc comment on MyAgentsListProps). `once` guards against a stale
    // closure re-firing `props.onFirstLoad` if it ever changed identity —
    // not expected from AgentPicker's call site today, but cheap insurance
    // since this must only ever fire once regardless.
    let firstLoadReported = false;
    createEffect(() => {
        if (rows() !== undefined && !firstLoadReported) {
            firstLoadReported = true;
            props.onFirstLoad?.();
        }
    });

    // Client-side name filter over the already-fetched page (bumped to
    // SEARCH_LIMIT while searching — see resourceKey/fetcher above).
    // Matches instance_name first, falling back to definition_name for
    // rows without a custom instance name (e.g. freshly-created agents).
    // SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md. Literal first: names that
    // contain what was typed are listed, and only when none does does the
    // shared fuzzy search (SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §5) step in
    // for a typo. Fuzzy alone listed "agenty" for "agentx", which is wrong when
    // the user types a name they know. No `keywords` concept here (agent names
    // are free text, not a fixed vocabulary), so no synonym matching.
    const filteredRows = createMemo(() => {
        const q = nameQuery();
        // `rowsStore.list`, not `rows()` — see the reconcile effect above;
        // this is what gives `<For>`/`<TransitionGroup>` stable per-row
        // identity across a refetch.
        const all = rowsStore.list;
        if (!q) return all;
        const nameOf = (r: RecentSessionRow) => r.instance_name || r.definition_name;
        return literalFirstSearch(all, q, nameOf, { keys: [{ name: "searchName", getFn: nameOf }] });
    });
    // Distinct from `isEmpty()`: the fetch found real rows, but the
    // filter narrowed them all away — not "you have no agents."
    // `rowsStore.list`, not `rows()` — same reagent P1 fix as isEmpty above:
    // this must agree with what filteredRows() itself is derived from, or
    // an optimistic delete leaves this reading the stale pre-delete count.
    const isNoMatch = () => !isLoading() && !fetchError() && rowsStore.list.length > 0 && filteredRows().length === 0;

    // Sort applied AFTER filtering, over whatever's already in memory — no
    // backend RPC change (§3 of the audit report above). `.slice()` before
    // `.sort()` since Array.prototype.sort mutates in place and
    // `filteredRows()` may be the SAME array reference `rows()` returned
    // when there's no active name filter (see filteredRows's own `return
    // all` fast path) — sorting that in place would mutate the resource's
    // cached value out from under Solid's stale-while-revalidate.
    const sortedRows = createMemo(() => {
        const sort = props.sortBy?.() ?? "recent";
        return filteredRows()
            .slice()
            .sort((a, b) => compareRows(sort, a, b));
    });

    return (
        <div class="agent-recent-sessions" data-testid="agent-my-agents-list" ref={rootRef}>
            <div class="agent-recent-sessions-header">
                <span class="agent-recent-sessions-title">My Agents</span>
                {/* No `!isLoading()` guard here (reagent P2 on PR #2328):
                    that check was already redundant even before isLoading
                    switched to rows.loading — if rows() were undefined,
                    `.length > 0` is already false — but now that isLoading
                    also covers BACKGROUND refetches (visibility regain,
                    agents:changed events), keeping the guard would hide
                    the count on every one of those instead of just the
                    very first load, which is a real, visible flicker
                    regression this component never had before. Solid's
                    stale-while-revalidate means `rows()` keeps showing the
                    last real count during a background refetch anyway —
                    exactly what should render. */}
                <Show when={filteredRows().length > 0}>
                    <span class="agent-recent-sessions-count" data-testid="agent-my-agents-count">
                        {filteredRows().length}
                    </span>
                </Show>
            </div>
            <Show
                // `|| hasPendingDeleteExit()`: don't unmount the list (and
                // with it, the <TransitionGroup> mid-animation) just
                // because deleting the last visible row's optimistic
                // removal made it LOOK empty/no-match before the poof has
                // actually finished playing — reagent P1 + codex P2 on PR
                // #3280.
                when={(!isEmpty() && !fetchError() && !isNoMatch()) || hasPendingDeleteExit()}
                fallback={
                    <Show
                        when={fetchError()}
                        fallback={
                            <Show
                                when={isNoMatch()}
                                fallback={
                                    <div class="agent-recent-sessions-empty" data-testid="agent-my-agents-empty">
                                        {filterId() ? EMPTY_FILTERED : EMPTY_GLOBAL}
                                    </div>
                                }
                            >
                                <div class="agent-recent-sessions-empty" data-testid="agent-my-agents-no-match">
                                    {noMatchText(rawQuery())}
                                </div>
                            </Show>
                        }
                    >
                        <div class="agent-recent-sessions-error" data-testid="agent-my-agents-error">
                            <span class="agent-recent-sessions-error-msg">{FETCH_ERROR}</span>
                            <Button
                                tone="danger"
                                class="agent-recent-sessions-retry"
                                onClick={() => void refetch()}
                                data-testid="agent-my-agents-retry"
                            >
                                Retry
                            </Button>
                        </div>
                    </Show>
                }
            >
                <ul class="agent-recent-sessions-list" ref={measureList}>
                    {/* Exit ("poof") + move (rubbery grid reflow) animation
                        on row delete. FLIP-based (measures rects, inverts,
                        plays) — see
                        docs/specs/SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md
                        §4.2/§4.3. No enter animation (non-goal, §3 of that
                        spec) — enter* classes are explicitly empty.

                        Exit is handled IMPERATIVELY (onBeforeExit/onExit),
                        not via the declarative exit*Class props (left
                        empty) — codex P2 on PR #3280: `<TransitionGroup>`
                        treats ANY row dropping out of `sortedRows()` as an
                        "exit," including one that just got filtered out by
                        a name-search query, not only a real delete. Only a
                        `definition_id` present in `deletingIds` (set by
                        confirmDelete, BEFORE the row leaves rowsStore.list)
                        gets the real poof; everything else calls `done()`
                        immediately for an instant removal — the same
                        behavior this list always had for filtering, before
                        this animation existed. A single class
                        (`agent-row-exit-active`) is enough since the poof
                        is one self-contained `@keyframes` `animation`, not
                        a Vue-style enter/active/to transition dance.

                        `onBeforeExit` pins the exiting row's on-screen
                        width/height as inline styles before that class
                        (which switches it to `position: absolute`, pulling
                        it out of the CSS grid's flow so surviving rows can
                        immediately reflow) applies — without this the row
                        would collapse to its content's intrinsic width
                        instead of holding its original grid-cell size
                        while it animates. When the motion policy reduces
                        `transition` (dormant today), both handlers bail to
                        the instant-removal path; the stylesheet's reduced
                        treatment is a backstop. */}
                    <TransitionGroup
                        enterClass=""
                        enterActiveClass=""
                        enterToClass=""
                        moveClass={shouldReduceMotion("transition") ? "" : "agent-row-move"}
                        onBeforeExit={(el) => {
                            const id = (el as HTMLElement).dataset.definitionId;
                            const isGenuineDelete = id !== undefined && deletingIds().has(id);
                            if (shouldReduceMotion("transition") || !isGenuineDelete) return;
                            const r = (el as HTMLElement).getBoundingClientRect();
                            (el as HTMLElement).style.width = `${r.width}px`;
                            (el as HTMLElement).style.height = `${r.height}px`;
                        }}
                        onExit={(el, done) => {
                            const id = (el as HTMLElement).dataset.definitionId;
                            const isGenuineDelete = id !== undefined && deletingIds().has(id);
                            const clearDeleting = () => {
                                if (id === undefined) return;
                                setDeletingIds((prev) => {
                                    if (!prev.has(id)) return prev;
                                    const next = new Set(prev);
                                    next.delete(id);
                                    return next;
                                });
                            };
                            if (!isGenuineDelete || shouldReduceMotion("transition")) {
                                clearDeleting();
                                done();
                                return;
                            }
                            const htmlEl = el as HTMLElement;
                            htmlEl.classList.add("agent-row-exit-active");
                            htmlEl.addEventListener(
                                "animationend",
                                () => {
                                    clearDeleting();
                                    done();
                                },
                                { once: true }
                            );
                        }}
                    >
                        <For each={sortedRows()}>
                            {(row) => {
                                const isActive = () => (props.openDefinitions?.() ?? new Map()).has(row.definition_id);
                                const noPane = () => {
                                    const runtime = props.runtimeOf?.(row.definition_id);
                                    return !!runtime && hasProcess(runtime.state) && !isActive();
                                };


                                // The tile is the `<li>` (the grid item): it owns the border,
                                // background and hover, so the chevron and the panels are placed
                                // against the box the user sees. See
                                // docs/reports/REPORT_MY_AGENTS_TILE_ACTIONS_ALIGNMENT_2026_10_05.md.
                                const openWhere = () =>
                                    props.openLocations?.().get(row.definition_id)?.label ?? "open in another pane";
                                onCleanup(() => tileEls.delete(row.definition_id));

                                return (
                                    <>
                                    <li
                                        ref={(el) => tileEls.set(row.definition_id, el)}
                                        class="agent-recent-sessions-row"
                                        classList={{
                                            "is-expanded": isRowExpanded(row.definition_id),
                                            "agent-recent-sessions-row--active": isActive(),
                                            "agent-recent-sessions-row--no-pane": noPane(),
                                        }}
                                        data-definition-id={row.definition_id}
                                    >
                                        <DualProviderLogo
                                            harness={row.provider}
                                            vendor={resolveEffectiveVendor(row.provider, row.model_vendor_base_url)}
                                            size={40}
                                            class="agent-recent-sessions-icon"
                                        />
                                        <span class="agent-recent-sessions-body">
                                            <span class="agent-recent-sessions-line1">
                                                {/* The one control that opens the agent. Its ::after
                                                covers the whole tile (the "stretched button"
                                                pattern), so a click anywhere on the tile opens it
                                                while the text stays plain text and the chevron
                                                stays a separate control: nested buttons are not
                                                valid HTML. */}
                                                <Button
                                                    variant="row"
                                                    density="compact"
                                                    class="agent-recent-sessions-entry"
                                                    onClick={() => toggleRowPanel(row)}
                                                    aria-expanded={isRowExpanded(row.definition_id)}
                                                    aria-label={row.instance_name || row.definition_name}
                                                    data-testid="agent-my-agents-entry"
                                                >
                                                    <span class="agent-recent-sessions-name">
                                                        {row.instance_name || row.definition_name}
                                                    </span>
                                                </Button>
                                                <Show when={props.runtimeOf?.(row.definition_id)}>
                                                    {(runtime) => (
                                                        <AgentRuntimeChip state={runtime().state} detail={runtime().detail} />
                                                    )}
                                                </Show>
                                                <Show when={noPane()}>
                                                    <span
                                                        class="agent-no-pane-tag"
                                                        data-testid="agent-no-pane-tag"
                                                    >
                                                        no pane
                                                    </span>
                                                </Show>
                                                <Show when={isActive()}>
                                                    <span
                                                        class="agent-active-badge"
                                                        title={
                                                            openWhere().charAt(0).toUpperCase() + openWhere().slice(1)
                                                        }
                                                        aria-label="Active"
                                                    />
                                                </Show>
                                                <Show
                                                    when={row.agent_type === "host" || row.agent_type === "container"}
                                                >
                                                    {/* size="tag" — same NATIVE/CONTAINER wording + white/
                                                    yellow styling as AgentComposerStrip's runtime
                                                    tag, so the badge reads as one consistent
                                                    vocabulary instead of two ("Container" here vs.
                                                    "CONTAINER" in the pane the row launches into). */}
                                                    <RuntimeBadge runtime={row.agent_type} size="tag" />
                                                </Show>
                                                <Show when={row.seat}>
                                                    {(seat) => (
                                                        <Pill class="agent-seat-badge" title={seatTooltipLine(seat())}>
                                                            {seatBadgeText(seat())}
                                                        </Pill>
                                                    )}
                                                </Show>
                                            </span>
                                            {/* Own line, not squeezed onto line1 via flex:1 next to
                                            the name — the account is as important as the name
                                            itself for a user managing multiple accounts, and
                                            needs reliable space rather than competing ellipsis
                                            with it. See
                                            docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §2. */}
                                            {/* "No auth", or the bound account; empty (so no line) when
                                            the backend could not tell
                                            (SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md §5.2). */}
                                            <Show when={row.identity_name}>
                                                <span class="agent-recent-sessions-account">{row.identity_name}</span>
                                            </Show>
                                            <Show when={row.seat}>
                                                {(seat) => (
                                                    <span class="agent-recent-sessions-seat" data-testid="agent-my-agents-seat-line">
                                                        {seatReportsLine(seat())}
                                                    </span>
                                                )}
                                            </Show>
                                            <Show
                                                when={row.preview}
                                                fallback={
                                                    <span class="agent-recent-sessions-preview agent-recent-sessions-preview--empty">
                                                        {noPreviewText(row)}
                                                    </span>
                                                }
                                            >
                                                <span class="agent-recent-sessions-preview">{row.preview}</span>
                                            </Show>
                                            <Show when={row.node_count > 0}>
                                                <span class="agent-recent-sessions-line3">
                                                    <span class="agent-recent-sessions-nodes">
                                                        {row.node_count} message
                                                        {row.node_count === 1 ? "" : "s"}
                                                    </span>
                                                </span>
                                            </Show>
                                        </span>
                                        {/* Footer row: the timestamps and the chevron share one row at
                                        the bottom of the tile. Most-relevant-first, not
                                        chronological (Created → Launch → Active): a picker exists to
                                        answer "what did I last touch," which is Last Active (or Last
                                        Launch if never active) — not when the agent was originally
                                        created, which is the least actionable fact on the row. See
                                        docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §2. */}
                                        <span class="agent-recent-sessions-timestamps">
                                            <Show
                                                when={
                                                    row.has_snapshot &&
                                                    row.started_at > 0 &&
                                                    row.last_active_at > row.started_at
                                                }
                                            >
                                                <span class="agent-recent-sessions-ts">
                                                    <span class="agent-recent-sessions-ts-label">Last Active</span>
                                                    <span class="agent-recent-sessions-ts-value">
                                                        {formatTimeAgo(row.last_active_at, now())}
                                                    </span>
                                                </span>
                                            </Show>
                                            <Show when={row.started_at > 0}>
                                                <span class="agent-recent-sessions-ts">
                                                    <span class="agent-recent-sessions-ts-label">Last Launch</span>
                                                    <span class="agent-recent-sessions-ts-value">
                                                        {formatTimeAgo(row.started_at, now())}
                                                    </span>
                                                </span>
                                            </Show>
                                            <Show when={row.agent_created_at > 0}>
                                                <span class="agent-recent-sessions-ts">
                                                    <span class="agent-recent-sessions-ts-label">Created</span>
                                                    <span class="agent-recent-sessions-ts-value">
                                                        {formatTimeAgo(row.agent_created_at, now())}
                                                    </span>
                                                </span>
                                            </Show>
                                        </span>

                                        {/* Row actions menu toggle. A sibling of the entry button, not
                                        nested inside it (nested buttons are invalid HTML). See
                                        docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.1. */}
                                        <IconButton
                                            icon="chevron-down"
                                            label={`Actions for ${row.instance_name || row.definition_name}`}
                                            tooltip={false}
                                            density="compact"
                                            class={clsx("agent-recent-sessions-menu-toggle", isRowExpanded(row.definition_id) && "is-open")}
                                            aria-expanded={isRowExpanded(row.definition_id)}
                                            onClick={(e) => {
                                                e.stopPropagation();
                                                toggleRowPanel(row);
                                            }}
                                            data-testid="agent-my-agents-menu-toggle"
                                        />

                                    </li>
                                    <Show when={panelAfter(row.definition_id)} keyed>
                                        {(open) => renderRowPanel(open)}
                                    </Show>
                                    </>
                                );
                            }}
                        </For>
                    </TransitionGroup>
                </ul>
            </Show>
        </div>
    );
};

MyAgentsList.displayName = "MyAgentsList";
