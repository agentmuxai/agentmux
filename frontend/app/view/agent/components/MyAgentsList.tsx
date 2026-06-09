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
 */

import {
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
import { ProviderLogo } from "@/element/ProviderLogo";

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
    /** Called when the user clicks an entry. The parent (AgentPicker)
     *  hands this to `AgentViewModel.launchAgentDefinition` with the
     *  continuation overrides — see "Reattach mechanism" above. */
    onReattach: (row: RecentSessionRow) => void;
}

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

    // The Date.now() snapshot updates every minute so "5m ago" rolls
    // forward without the user having to re-render. createSignal ticks
    // are cheaper than re-running the createResource.
    const [now, setNow] = createSignal(Date.now());
    const tick = setInterval(() => setNow(Date.now()), 60_000);
    onCleanup(() => clearInterval(tick));

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
                        {(row) => (
                            <li class="agent-recent-sessions-row">
                                <button
                                    type="button"
                                    class="agent-recent-sessions-entry"
                                    onClick={() => props.onReattach(row)}
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
                                            <Show when={row.has_snapshot && row.last_active_at > row.started_at}>
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
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </div>
    );
};

MyAgentsList.displayName = "MyAgentsList";
