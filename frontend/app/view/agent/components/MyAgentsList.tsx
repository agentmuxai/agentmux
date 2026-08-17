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

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { DualProviderLogo } from "@/element/DualProviderLogo";
import { formatTimeAgo } from "@/util/format-time";
import { Logger } from "@/util/logger";
import { resolveEffectiveVendor } from "../providers/catalog";
import { RuntimeBadge } from "./RuntimeBadge";

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
}

type ForkState =
    | { kind: "idle" }
    | { kind: "prompt" } // showing "Open new session / Switch" buttons
    | { kind: "naming"; label: string; loading: boolean; error: string | null }; // name input expanded

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
                <ul class="agent-recent-sessions-list">
                    <For each={filteredRows()}>
                        {(row) => {
                            const isActive = () => (props.openDefinitions?.() ?? new Map()).has(row.definition_id);
                            const forkState = () => getForkState(row.definition_id);

                            return (
                                <li class="agent-recent-sessions-row">
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
                                                <span class="agent-recent-sessions-meta">
                                                    {row.identity_name || "(ambient creds)"}
                                                </span>
                                            </span>
                                            <Show
                                                when={row.preview}
                                                fallback={
                                                    <span class="agent-recent-sessions-preview agent-recent-sessions-preview--empty">
                                                        {row.has_snapshot
                                                            ? "(no user message yet)"
                                                            : "(no conversation snapshot)"}
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
                                            <span class="agent-recent-sessions-timestamps">
                                                <Show when={row.agent_created_at > 0}>
                                                    <span class="agent-recent-sessions-ts">
                                                        <span class="agent-recent-sessions-ts-label">Created</span>
                                                        <span class="agent-recent-sessions-ts-value">
                                                            {formatTimeAgo(row.agent_created_at, now())}
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
                                            </span>
                                        </span>
                                    </button>

                                    {/* Fork prompt — inline below the row */}
                                    <Show when={forkState().kind !== "idle"}>
                                        <div class="agent-fork-prompt" data-testid="agent-fork-prompt">
                                            <span class="agent-fork-prompt-msg">
                                                <strong>{row.instance_name || row.definition_name}</strong> is already
                                                open in another pane.
                                            </span>
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
                                                    <div class="agent-fork-naming">
                                                        <label class="agent-fork-label">Name for new session:</label>
                                                        <div class="agent-fork-input-row">
                                                            <input
                                                                type="text"
                                                                class="agent-fork-input"
                                                                value={ns().label}
                                                                disabled={ns().loading}
                                                                placeholder="Session name"
                                                                data-testid="agent-fork-name-input"
                                                                onInput={(e) =>
                                                                    setForkState(row.definition_id, {
                                                                        kind: "naming",
                                                                        label: e.currentTarget.value,
                                                                        loading: false,
                                                                        error: null,
                                                                    })
                                                                }
                                                                onKeyDown={(e) => {
                                                                    if (e.key === "Enter") void handleForkStart(row);
                                                                    if (e.key === "Escape")
                                                                        handleForkCancel(row.definition_id);
                                                                }}
                                                                ref={(el) => {
                                                                    createEffect(() => {
                                                                        if (!ns().loading) el.focus();
                                                                    });
                                                                }}
                                                            />
                                                            <button
                                                                type="button"
                                                                class="agent-fork-btn agent-fork-btn--primary"
                                                                disabled={ns().loading || !ns().label.trim()}
                                                                onClick={() => handleForkStart(row)}
                                                                data-testid="agent-fork-start"
                                                            >
                                                                {ns().loading ? "…" : "Start"}
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
                                                        <Show when={ns().error}>
                                                            <span class="agent-fork-error">{ns().error}</span>
                                                        </Show>
                                                    </div>
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
        </div>
    );
};

MyAgentsList.displayName = "MyAgentsList";
