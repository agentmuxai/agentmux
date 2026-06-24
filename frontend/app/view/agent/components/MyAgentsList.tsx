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
import { useTick } from "@/app/hook/useTick";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { ProviderLogo } from "@/element/ProviderLogo";
import { RuntimeBadge } from "./RuntimeBadge";

/** ms epoch → human-readable relative timestamp. Centralized here +
 * exported so unit tests can pin its boundary behavior. */
export function formatRelative(now: number, ms: number): string {
    if (!ms) return "";
    const delta = Math.max(0, now - ms);
    if (delta < 60_000) return "just now";
    if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
    if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
    return `${Math.floor(delta / 86_400_000)}d ago`;
}

/** Empty-state copy varies based on whether an identity filter was
 * applied — surfaced so the integration test can match it. */
export const EMPTY_GLOBAL =
    "No agents yet — pick a template below to create your first one.";
export const EMPTY_FILTERED = "No agents for this identity yet.";

export interface MyAgentsListProps {
    /** Optional reactive accessor for the identity filter. `null` /
     *  undefined / empty string = no filter (show every identity). */
    identityId?: Accessor<string | null | undefined>;
    /** Visible cap. Backend caps at 100; default is 20. */
    limit?: number;
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
}

type ForkState =
    | { kind: "idle" }
    | { kind: "prompt" }           // showing "Open new session / Switch" buttons
    | { kind: "naming"; label: string; loading: boolean; error: string | null }; // name input expanded

export const MyAgentsList = (props: MyAgentsListProps): JSX.Element => {
    const filterId = createMemo(() => {
        const raw = props.identityId?.();
        if (raw === null || raw === undefined) return "";
        return String(raw).trim();
    });

    const [rows, { refetch }] = createResource<RecentSessionRow[], string>(
        // Key the resource on the filter so changes refetch.
        filterId,
        async (id) => {
            try {
                return await RpcApi.ListRecentSessionsCommand(TabRpcClient, {
                    limit: props.limit ?? 20,
                    // Backend treats "" as "no filter" — see
                    // CommandListRecentSessionsData docs.
                    identity_id: id,
                });
            } catch {
                return [];
            }
        },
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

    const getForkState = (definitionId: string): ForkState =>
        forkStates().get(definitionId) ?? { kind: "idle" };

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
    // - rows undefined → still loading (skeleton hint)
    // - rows []        → empty state (filter-aware copy)
    // - rows non-empty → list
    const isLoading = () => rows() === undefined;
    const isEmpty = () => !isLoading() && (rows() ?? []).length === 0;

    return (
        <div class="agent-recent-sessions" data-testid="agent-my-agents-list">
            <div class="agent-recent-sessions-header">
                <span class="agent-recent-sessions-title">My Agents</span>
                <Show when={!isLoading() && (rows() ?? []).length > 0}>
                    <span class="agent-recent-sessions-count">
                        {(rows() ?? []).length}
                    </span>
                </Show>
            </div>
            <Show
                when={!isEmpty()}
                fallback={
                    <div
                        class="agent-recent-sessions-empty"
                        data-testid="agent-my-agents-empty"
                    >
                        {filterId() ? EMPTY_FILTERED : EMPTY_GLOBAL}
                    </div>
                }
            >
                <ul class="agent-recent-sessions-list">
                    <For each={rows() ?? []}>
                        {(row) => {
                            const isActive = () =>
                                (props.openDefinitions?.() ?? new Map()).has(row.definition_id);
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
                                        <ProviderLogo
                                            provider={row.provider}
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
                                                <Show when={row.agent_type === "host" || row.agent_type === "container"}>
                                                    <RuntimeBadge runtime={row.agent_type} size="sm" />
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
                                                <span class="agent-recent-sessions-preview">
                                                    {row.preview}
                                                </span>
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
                                                            {formatRelative(now(), row.agent_created_at)}
                                                        </span>
                                                    </span>
                                                </Show>
                                                <Show when={row.started_at > 0}>
                                                    <span class="agent-recent-sessions-ts">
                                                        <span class="agent-recent-sessions-ts-label">Last Launch</span>
                                                        <span class="agent-recent-sessions-ts-value">
                                                            {formatRelative(now(), row.started_at)}
                                                        </span>
                                                    </span>
                                                </Show>
                                                <Show when={row.has_snapshot && row.started_at > 0 && row.last_active_at > row.started_at}>
                                                    <span class="agent-recent-sessions-ts">
                                                        <span class="agent-recent-sessions-ts-label">Last Active</span>
                                                        <span class="agent-recent-sessions-ts-value">
                                                            {formatRelative(now(), row.last_active_at)}
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
                                                <strong>{row.instance_name || row.definition_name}</strong> is already open in another pane.
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
                                            <Show when={forkState().kind === "naming" ? (forkState() as Extract<ForkState, { kind: "naming" }>) : null}>
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
                                                                    if (e.key === "Escape") handleForkCancel(row.definition_id);
                                                                }}
                                                                ref={(el) => { createEffect(() => { if (!ns().loading) el.focus(); }); }}
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
