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
 * Renamed from RecentSessionsList.tsx (the cascade follow-up file
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
 */

import { useTick } from "@/app/hook/useTick";
import {
    createEffect,
    createMemo,
    createResource,
    createSignal,
    For,
    onCleanup,
    Show,
    type Accessor,
    type JSX,
} from "solid-js";

import { pushNotification } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getOpenBlockIdsForDefinition } from "@/app/store/agent-pane-state-store";
import { waveEventSubscribe } from "@/app/store/wps";
import { ConfirmModal } from "@/element/modal";
import { DualProviderLogo } from "@/element/DualProviderLogo";
import { ObjectService } from "@/app/store/services";
import { formatTimeAgo } from "@/util/format-time";
import { Logger } from "@/util/logger";
import { resolveEffectiveVendor } from "../providers/catalog";
import type { AgentSortOption } from "./AgentPickerFilterBar";
import { RuntimeBadge } from "./RuntimeBadge";

/** "type" sort groups Host before Sandbox (Container) before anything
 *  unrecognized, matching `RuntimeBadge`'s own known-runtime ordering —
 *  not alphabetical ("container" < "host" would put Sandbox first, which
 *  reads backwards next to the badge's own HOST/SANDBOX vocabulary).
 *
 *  Codex P2, PR #2789: `"standalone"` is the LEGACY default `agent_type`
 *  (`default_agent_type()`, `backend/storage/agents.rs`) for definitions
 *  predating the container feature, and every non-`"container"` value is
 *  treated as the host controller at launch (`agent_open.rs`'s
 *  `controller_type = if agent.agent_type == "container" {...} else {...}`)
 *  — "standalone" IS a host agent, effectively, just under an older name.
 *  Without this, legacy definitions would rank as "unknown" (last),
 *  splitting them from genuinely host-labeled agents instead of grouping
 *  correctly with them. */
const TYPE_SORT_RANK: Record<string, number> = { host: 0, standalone: 0, container: 1 };
const typeSortRank = (agentType: string): number => TYPE_SORT_RANK[agentType] ?? 2;

function compareRows(sort: AgentSortOption, a: RecentSessionRow, b: RecentSessionRow): number {
    switch (sort) {
        case "name":
            return (a.instance_name || a.definition_name).localeCompare(b.instance_name || b.definition_name, undefined, {
                sensitivity: "base",
            });
        case "type": {
            const rankDiff = typeSortRank(a.agent_type) - typeSortRank(b.agent_type);
            if (rankDiff !== 0) return rankDiff;
            return (a.instance_name || a.definition_name).localeCompare(b.instance_name || b.definition_name, undefined, {
                sensitivity: "base",
            });
        }
        case "recent":
        default:
            return b.started_at - a.started_at;
    }
}

/** Empty-state copy varies based on whether an identity filter was
 * applied — surfaced so the integration test can match it. */
export const EMPTY_GLOBAL = "No agents yet — pick a template below to create your first one.";
export const EMPTY_FILTERED = "No agents for this identity yet.";
/** Shown when ListRecentSessionsCommand itself failed — distinct from
 * EMPTY_GLOBAL/EMPTY_FILTERED so a backend error never looks identical
 * to "you genuinely have zero agents" (retro
 * docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md §4/§9
 * rec 2 — this exact ambiguity is what let a real regression, PR #2296,
 * go unnoticed as "expected empty state" the first time it happened). */
export const FETCH_ERROR = "Couldn't load your agents — check the connection and try again.";
/** Copy for "the fetch succeeded, but the name filter matched nothing" —
 * distinct from EMPTY_GLOBAL/EMPTY_FILTERED (both mean "there is nothing
 * to filter"), so a user narrowing a real, non-empty list to zero visible
 * rows isn't told they have no agents at all.
 * SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md. */
export const noMatchText = (query: string): string => `No agents match "${query}".`;

/**
 * Copy for a row with no preview text and no snapshot, split into its
 * three genuinely distinct causes — collapsing them into one generic
 * "(no conversation snapshot)" made a real backend error and a
 * cross-channel row both read as "this agent has no history," which
 * isn't true for either. See
 * docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md
 * §5.
 *
 * - `snapshot_check_failed` — the filestore lookup itself errored
 *   (transient I/O/lock/DB failure); the real state is unknown, not
 *   confirmed empty. Checked first: a stat() error on a cross-channel
 *   row can't actually happen (session.rs skips the call entirely when
 *   `block_id_hint` is empty), but ordering this first keeps the two
 *   conditions from ever silently depending on which one wins.
 * - `block_id_hint === ""` — a synthetic cross-channel row (no local
 *   SQLite instance backs it); this channel's filestore genuinely has
 *   nothing, but the real conversation may exist in another version.
 * - otherwise — a genuine, confirmed `Ok(None)`: the block really never
 *   wrote a snapshot.
 */
export const noSnapshotText = (row: Pick<RecentSessionRow, "snapshot_check_failed" | "block_id_hint">): string => {
    if (row.snapshot_check_failed) return "(couldn't check for history)";
    if (!row.block_id_hint) return "(history may exist in another version)";
    return "(no conversation snapshot)";
};
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
}

type ForkState =
    | { kind: "idle" }
    | { kind: "prompt" } // showing "Open new session / Switch" buttons
    | { kind: "naming"; label: string; loading: boolean; error: string | null }; // name input expanded

interface NamePromptProps {
    /** Prompt above the input, e.g. "Rename to:". */
    label: string;
    placeholder: string;
    value: string;
    loading: boolean;
    error: string | null;
    /** Text on the confirm button; swapped for "…" while loading. */
    submitLabel: string;
    inputTestid: string;
    submitTestid: string;
    onInput: (value: string) => void;
    onSubmit: () => void;
    onCancel: () => void;
}

/**
 * The "type a name, Enter to confirm, Escape to back out" sub-panel, shared
 * by the fork prompt's session naming and the row menu's Rename
 * (docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.3 — Rename was specified to
 * "reuse the existing fork-prompt naming sub-pattern", which until now meant
 * a second copy of the same markup, autofocus effect and key handling).
 * Purely presentational: every piece of state lives in the caller's own
 * per-row map.
 */
const NamePrompt = (props: NamePromptProps): JSX.Element => (
    <div class="agent-fork-naming">
        <label class="agent-fork-label">{props.label}</label>
        <div class="agent-fork-input-row">
            <input
                type="text"
                class="agent-fork-input"
                value={props.value}
                disabled={props.loading}
                placeholder={props.placeholder}
                data-testid={props.inputTestid}
                onInput={(e) => props.onInput(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Enter") props.onSubmit();
                    if (e.key === "Escape") props.onCancel();
                }}
                ref={(el) => {
                    createEffect(() => {
                        if (!props.loading) el.focus();
                    });
                }}
            />
            <button
                type="button"
                class="agent-fork-btn agent-fork-btn--primary"
                disabled={props.loading || !props.value.trim()}
                onClick={() => props.onSubmit()}
                data-testid={props.submitTestid}
            >
                {props.loading ? "…" : props.submitLabel}
            </button>
            <button
                type="button"
                class="agent-fork-btn agent-fork-btn--ghost"
                onClick={() => props.onCancel()}
                aria-label="Cancel"
            >
                ✕
            </button>
        </div>
        <Show when={props.error}>
            <span class="agent-fork-error">{props.error}</span>
        </Show>
    </div>
);

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
                // the existing "(missing account)"-style fallback text.
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
    const unsubAgents = waveEventSubscribe({
        eventType: "agents:changed",
        handler: () => void refetch(),
    });
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

    const handleRowClick = async (row: RecentSessionRow) => {
        const openMap = props.openDefinitions?.() ?? new Map<string, string>();
        const existingBlockId = openMap.get(row.definition_id);
        if (existingBlockId) {
            // Already open — show fork prompt
            setForkState(row.definition_id, { kind: "prompt" });
            return;
        }
        props.onReattach(row);
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
        const openMap = props.openDefinitions?.() ?? new Map<string, string>();
        const blockId = openMap.get(row.definition_id);
        if (blockId) props.onSwitchToExisting?.(blockId);
        setForkState(row.definition_id, { kind: "idle" });
    };

    const handleForkStart = async (row: RecentSessionRow) => {
        const fs = getForkState(row.definition_id);
        if (fs.kind !== "naming" || !fs.label.trim()) return;
        const label = fs.label.trim();
        setForkState(row.definition_id, { kind: "naming", label, loading: true, error: null });
        try {
            await props.onFork?.(row, label);
            setForkState(row.definition_id, { kind: "idle" });
        } catch (err) {
            setForkState(row.definition_id, {
                kind: "naming",
                label,
                loading: false,
                error: err instanceof Error ? err.message : String(err),
            });
        }
    };

    const handleForkCancel = (definitionId: string) => {
        setForkState(definitionId, { kind: "idle" });
    };

    // Row actions menu (chevron-toggled, per row) — docs/specs/
    // SPEC_AGENT_DELETE_2026_09_16.md §4.
    //
    // Exactly one row's menu can be open, so this is a single id rather
    // than the keyed Map/Set idiom `forkStates` uses: "open another row's
    // menu closes this one" is then the data model rather than a rule the
    // setters have to maintain, and the "blur every OTHER row" spotlight
    // below always has exactly one row excluded by construction.
    const [openMenuId, setOpenMenuId] = createSignal<string | null>(null);
    const isMenuOpen = (definitionId: string): boolean => openMenuId() === definitionId;
    const toggleMenu = (definitionId: string): void => {
        setOpenMenuId((prev) => {
            if (prev === definitionId) return null;
            // The menu and the inline panels all render at `top: 100%` of
            // the same row, so two open at once would overlap. They are
            // alternatives — reopening the chevron drops a half-typed
            // rename/duplicate name, which is the predictable reading of
            // "go back to the menu".
            setRenameState(definitionId, null);
            setForkState(definitionId, { kind: "idle" });
            return definitionId;
        });
    };
    const closeMenu = (definitionId: string): void => {
        setOpenMenuId((prev) => (prev === definitionId ? null : prev));
    };

    // "Click elsewhere collapses it" (spec §4.2), plus Escape. The toggle
    // and the panel are excluded: the toggle owns its own close (letting
    // both fire on one click would close then immediately reopen), and a
    // menu item closes as part of the action it runs. Capture phase so a
    // click that lands on something calling stopPropagation still counts as
    // "elsewhere" — the blurred rows set `pointer-events: none`, so those
    // clicks reach the list container rather than a row.
    const onDocumentPointerDown = (e: MouseEvent) => {
        if (openMenuId() === null) return;
        const target = e.target as Element | null;
        if (target?.closest?.(".agent-row-menu, .agent-recent-sessions-menu-toggle")) return;
        setOpenMenuId(null);
    };
    const onDocumentKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") setOpenMenuId(null);
    };
    document.addEventListener("pointerdown", onDocumentPointerDown, true);
    document.addEventListener("keydown", onDocumentKeyDown);
    onCleanup(() => {
        document.removeEventListener("pointerdown", onDocumentPointerDown, true);
        document.removeEventListener("keydown", onDocumentKeyDown);
    });

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
        // Rename and Duplicate render two DIFFERENT inline panels into the
        // same <li>, from two independent state maps — so opening one while
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
        setRenameState(row.definition_id, { label: row.instance_name || row.definition_name, loading: false, error: null });
    };
    const handleRenameCancel = (definitionId: string): void => setRenameState(definitionId, null);
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

    /**
     * A row is "expanded" while ANY of its inline panels is showing — the
     * actions menu, the rename input, or the fork/duplicate prompt.
     *
     * The spotlight (blur every other row) and the overlay positioning are
     * keyed to this rather than to the menu alone. Keying them to the menu
     * meant clicking Rename or Duplicate — which closes the menu on its way
     * to opening a panel — dropped the row out of the effect, so the
     * neighbours un-blurred and the panel went back to pushing them down.
     *
     * A plain accessor, not a `createMemo`: a memo evaluates eagerly at
     * creation, and `renameStates` is declared below this point, so it
     * would be read inside its own temporal dead zone.
     */
    const isRowExpanded = (definitionId: string): boolean =>
        isMenuOpen(definitionId) ||
        getRenameState(definitionId) !== null ||
        getForkState(definitionId).kind !== "idle";
    const anyRowExpanded = (): boolean =>
        openMenuId() !== null ||
        renameStates().size > 0 ||
        // `setForkState(id, {kind: "idle"})` leaves the entry in place
        // rather than deleting it, so size alone would stay true forever
        // after the first fork prompt was cancelled.
        [...forkStates().values()].some((s) => s.kind !== "idle");

    // View History — delegates to the parent; see onViewHistory's own doc
    // comment on MyAgentsListProps for why this can't be self-contained.
    const handleViewHistory = (row: RecentSessionRow): void => {
        closeMenu(row.definition_id);
        props.onViewHistory?.(row);
    };

    // Delete — single confirm-modal instance (not per-row: only one can be
    // open at a time), reusing the already-hardened backend path
    // (agent_def_delete purges every dependent table — see
    // docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §5.1).
    const [deleteConfirmRow, setDeleteConfirmRow] = createSignal<RecentSessionRow | null>(null);
    const handleDeleteOpen = (row: RecentSessionRow): void => {
        closeMenu(row.definition_id);
        setDeleteConfirmRow(row);
    };
    const confirmDelete = async (): Promise<void> => {
        const row = deleteConfirmRow();
        if (!row) return;
        try {
            await RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id: row.definition_id });
        } catch (e: unknown) {
            setDeleteConfirmRow(null);
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
        setDeleteConfirmRow(null);
        // Sweep EVERY pane open for this agent (spec §5.2). Not
        // `props.openDefinitions` — that map is keyed by definition, so an
        // agent open in two panes collapses to whichever block registered
        // last and the others would keep running against a deleted agent
        // (codex P2 on PR #3262). The map's shape is right for its other
        // caller (the fork prompt just needs *a* pane to switch to); a
        // sweep needs all of them.
        //
        // KNOWN LIMITATION, this renderer only: `slots` is module-local, so
        // a pane for this agent in ANOTHER window (or a floating-pane
        // window) is not swept and keeps running against a deleted agent
        // (codex P2, second round). Closing it needs a backend-global block
        // query or a cross-window broadcast — real work, not a wider
        // `filter` here — so it is called out rather than silently implied
        // to be handled. Tracked in SPEC_AGENT_DELETE_2026_09_16.md §5.2.
        //
        // Failures are reported, not swallowed: the agent and its
        // credentials are already gone by this point, so a pane that
        // wouldn't close is a live pane attached to a deleted agent, and
        // the user is the only one who can do anything about it.
        const failedBlockIds = (
            await Promise.all(
                getOpenBlockIdsForDefinition(row.definition_id).map((blockId) =>
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
        // The row itself disappears via the existing "agents:changed"
        // refetch subscription (line ~334) — no manual list mutation here.
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
    const isLoading = () => rows.loading;
    const isEmpty = () => !isLoading() && !fetchError() && (rows() ?? []).length === 0;

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
    // SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md.
    const filteredRows = createMemo(() => {
        const q = nameQuery();
        const all = rows() ?? [];
        if (!q) return all;
        return all.filter((r) => (r.instance_name || r.definition_name).toLowerCase().includes(q));
    });
    // Distinct from `isEmpty()`: the fetch found real rows, but the
    // filter narrowed them all away — not "you have no agents."
    const isNoMatch = () => !isLoading() && !fetchError() && (rows() ?? []).length > 0 && filteredRows().length === 0;

    // Sort applied AFTER filtering, over whatever's already in memory — no
    // backend RPC change (§3 of the audit report above). `.slice()` before
    // `.sort()` since Array.prototype.sort mutates in place and
    // `filteredRows()` may be the SAME array reference `rows()` returned
    // when there's no active name filter (see filteredRows's own `return
    // all` fast path) — sorting that in place would mutate the resource's
    // cached value out from under Solid's stale-while-revalidate.
    const sortedRows = createMemo(() => {
        const sort = props.sortBy?.() ?? "recent";
        return filteredRows().slice().sort((a, b) => compareRows(sort, a, b));
    });

    return (
        <div class="agent-recent-sessions" data-testid="agent-my-agents-list">
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
                when={!isEmpty() && !fetchError() && !isNoMatch()}
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
                            <button
                                type="button"
                                class="agent-recent-sessions-retry"
                                onClick={() => void refetch()}
                                data-testid="agent-my-agents-retry"
                            >
                                Retry
                            </button>
                        </div>
                    </Show>
                }
            >
                <ul class="agent-recent-sessions-list" classList={{ "has-expanded-row": anyRowExpanded() }}>
                    <For each={sortedRows()}>
                        {(row) => {
                            const isActive = () => (props.openDefinitions?.() ?? new Map()).has(row.definition_id);
                            const forkState = () => getForkState(row.definition_id);

                            return (
                                <li
                                    class="agent-recent-sessions-row"
                                    classList={{ "is-expanded": isRowExpanded(row.definition_id) }}
                                >
                                    <button
                                        type="button"
                                        class={`agent-recent-sessions-entry${isActive() ? " agent-recent-sessions-entry--active" : ""}`}
                                        onClick={() => handleRowClick(row)}
                                        aria-label={`Continue ${row.instance_name}`}
                                        data-testid="agent-my-agents-entry"
                                    >
                                        <DualProviderLogo
                                            harness={row.provider}
                                            vendor={resolveEffectiveVendor(row.provider, row.model_vendor_base_url)}
                                            size={24}
                                            class="agent-recent-sessions-icon"
                                        />
                                        <span class="agent-recent-sessions-body">
                                            <span class="agent-recent-sessions-line1">
                                                <span class="agent-recent-sessions-name">
                                                    {row.instance_name || row.definition_name}
                                                </span>
                                                <Show when={isActive()}>
                                                    <span
                                                        class="agent-active-badge"
                                                        title="Open in another pane"
                                                        aria-label="Active"
                                                    />
                                                </Show>
                                                <Show
                                                    when={row.agent_type === "host" || row.agent_type === "container"}
                                                >
                                                    {/* size="tag" — same HOST/SANDBOX wording + white/
                                                        yellow styling as AgentComposerStrip's runtime
                                                        tag, so the badge reads as one consistent
                                                        vocabulary instead of two ("Container" here vs.
                                                        "SANDBOX" in the pane the row launches into). */}
                                                    <RuntimeBadge runtime={row.agent_type} size="tag" />
                                                </Show>
                                            </span>
                                            {/* Own line, not squeezed onto line1 via flex:1 next to
                                                the name — the account is as important as the name
                                                itself for a user managing multiple accounts, and
                                                needs reliable space rather than competing ellipsis
                                                with it. See
                                                docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §2. */}
                                            <span class="agent-recent-sessions-account">
                                                {row.identity_name || "(ambient creds)"}
                                            </span>
                                            <Show
                                                when={row.preview}
                                                fallback={
                                                    <span class="agent-recent-sessions-preview agent-recent-sessions-preview--empty">
                                                        {row.has_snapshot ? "(no user message yet)" : noSnapshotText(row)}
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
                                            {/* Most-relevant-first, not chronological (Created →
                                                Launch → Active): a picker exists to answer "what did
                                                I last touch," which is Last Active (or Last Launch if
                                                never active) — not when the agent was originally
                                                created, which is the least actionable fact on the
                                                row. See
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
                                        </span>
                                    </button>

                                    {/* Row actions menu — chevron toggle + inline expand.
                                        A SIBLING of the entry button above, not nested inside
                                        it (nested buttons are invalid HTML, and the entry
                                        button's own click would fire first regardless). See
                                        docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.1. */}
                                    <button
                                        type="button"
                                        class="agent-recent-sessions-menu-toggle"
                                        classList={{ "is-open": isMenuOpen(row.definition_id) }}
                                        aria-label={`Actions for ${row.instance_name || row.definition_name}`}
                                        aria-expanded={isMenuOpen(row.definition_id)}
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            toggleMenu(row.definition_id);
                                        }}
                                        data-testid="agent-my-agents-menu-toggle"
                                    >
                                        <i class="fa-sharp fa-solid fa-chevron-down" aria-hidden="true" />
                                    </button>
                                    <Show when={isMenuOpen(row.definition_id)}>
                                        <div class="agent-row-menu" data-testid="agent-row-menu">
                                            <button
                                                type="button"
                                                class="agent-row-menu-item"
                                                onClick={() => handleRenameOpen(row)}
                                            >
                                                <i class="fa-sharp fa-solid fa-pen" aria-hidden="true" /> Rename
                                            </button>
                                            <button
                                                type="button"
                                                class="agent-row-menu-item"
                                                onClick={() => handleDuplicate(row)}
                                            >
                                                <i class="fa-sharp fa-solid fa-clone" aria-hidden="true" /> Duplicate
                                            </button>
                                            <button
                                                type="button"
                                                class="agent-row-menu-item"
                                                onClick={() => handleViewHistory(row)}
                                            >
                                                <i class="fa-sharp fa-solid fa-clock-rotate-left" aria-hidden="true" /> View
                                                History
                                            </button>
                                            <button
                                                type="button"
                                                class="agent-row-menu-item agent-row-menu-item--danger"
                                                onClick={() => handleDeleteOpen(row)}
                                                data-testid="agent-my-agents-delete"
                                            >
                                                <i class="fa-sharp fa-solid fa-trash-can" aria-hidden="true" /> Delete
                                            </button>
                                        </div>
                                    </Show>

                                    {/* Inline rename input — same sibling-panel pattern as
                                        the fork prompt below. */}
                                    <Show when={getRenameState(row.definition_id)}>
                                        {(rs) => (
                                            <div class="agent-fork-prompt" data-testid="agent-rename-prompt">
                                                <NamePrompt
                                                    label="Rename to:"
                                                    placeholder="Agent name"
                                                    value={rs().label}
                                                    loading={rs().loading}
                                                    error={rs().error}
                                                    submitLabel="Save"
                                                    inputTestid="agent-rename-input"
                                                    submitTestid="agent-rename-save"
                                                    onInput={(label) =>
                                                        setRenameState(row.definition_id, { ...rs(), label })
                                                    }
                                                    onSubmit={() => void handleRenameSubmit(row)}
                                                    onCancel={() => handleRenameCancel(row.definition_id)}
                                                />
                                            </div>
                                        )}
                                    </Show>

                                    {/* Fork prompt — inline below the row */}
                                    <Show when={forkState().kind !== "idle"}>
                                        <div class="agent-fork-prompt" data-testid="agent-fork-prompt">
                                            {/* Only true when reached via the "already open" row
                                                click (kind === "prompt") — Duplicate (this spec's
                                                new entry point) jumps straight to "naming" without
                                                that being the reason, so the message must not
                                                assume it. Still shown for "naming" too when the row
                                                genuinely happens to be open elsewhere. */}
                                            <Show when={forkState().kind === "prompt" || isActive()}>
                                                <span class="agent-fork-prompt-msg">
                                                    <strong>{row.instance_name || row.definition_name}</strong> is
                                                    already open in another pane.
                                                </span>
                                            </Show>
                                            <Show when={forkState().kind === "prompt"}>
                                                <div class="agent-fork-prompt-actions">
                                                    <button
                                                        type="button"
                                                        class="agent-fork-btn agent-fork-btn--primary"
                                                        onClick={() => handleOpenNewSession(row)}
                                                        data-testid="agent-fork-open-new"
                                                    >
                                                        Open new session
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="agent-fork-btn agent-fork-btn--secondary"
                                                        onClick={() => handleSwitchToExisting(row)}
                                                        data-testid="agent-fork-switch"
                                                    >
                                                        Switch to existing
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="agent-fork-btn agent-fork-btn--ghost"
                                                        onClick={() => handleForkCancel(row.definition_id)}
                                                        aria-label="Cancel"
                                                    >
                                                        ✕
                                                    </button>
                                                </div>
                                            </Show>
                                            <Show
                                                when={
                                                    forkState().kind === "naming"
                                                        ? (forkState() as Extract<ForkState, { kind: "naming" }>)
                                                        : null
                                                }
                                            >
                                                {(ns) => (
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
                                                            setForkState(row.definition_id, {
                                                                kind: "naming",
                                                                label,
                                                                loading: false,
                                                                error: null,
                                                            })
                                                        }
                                                        onSubmit={() => void handleForkStart(row)}
                                                        onCancel={() => handleForkCancel(row.definition_id)}
                                                    />
                                                )}
                                            </Show>
                                        </div>
                                    </Show>
                                </li>
                            );
                        }}
                    </For>
                </ul>
            </Show>
            <ConfirmModal
                open={deleteConfirmRow() !== null}
                title={`Delete ${deleteConfirmRow()?.instance_name || deleteConfirmRow()?.definition_name}?`}
                description="This permanently deletes the agent and its credentials, skills, and activity history. Its bundle and any manually-saved transcripts are not affected. This cannot be undone."
                confirmLabel="Delete"
                destructive
                onConfirm={confirmDelete}
                onCancel={() => setDeleteConfirmRow(null)}
            />
        </div>
    );
};

MyAgentsList.displayName = "MyAgentsList";
