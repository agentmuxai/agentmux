// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * RecentSessionsList — the AgentPicker's "Recent sessions" surface.
 *
 * Cascade follow-up (2026-05-23) — see
 * `docs/recovery/MAKS_CONVERSATION_2026_05_23.md` and
 * `docs/retro/retro-agent-pane-cascade-replacechild-2026-05-23.md`.
 * Before this surface, a renderer crash that killed an agent pane left
 * the conversation file orphaned in filestore with no UI path back —
 * the user had to know the blockId and force a remount. With this
 * list, the user picks the session from the picker and the existing
 * continuation flow (PR #977's continueOfInstanceId + workDirOverride)
 * brings the working directory + identity + memory back; the prior
 * conversation history reloads from filestore on the new pane's first
 * paint via the standard `useHistoryPagination` snapshot path.
 *
 * Reattach mechanism: this surface **does NOT mint a new "ghost"
 * block referring to the original blockId.** Each entry triggers a
 * normal definition launch through `AgentViewModel.launchAgentDefinition`
 * with `continueOfInstanceId` + `workDirOverride` set from the row.
 * The new pane spawns the CLI in the prior working directory, so
 * Claude's `--continue` (and equivalents on other CLIs) resumes the
 * session and the new pane's `output.state.json` snapshot path
 * picks up the conversation history on render. Per spec — using the
 * existing continueOfId plumbing keeps surface area small and reuses
 * the audited launch flow.
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
export const EMPTY_GLOBAL = "No recent sessions yet";
export const EMPTY_FILTERED = "No recent sessions for this identity";

export interface RecentSessionsListProps {
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

export const RecentSessionsList = (props: RecentSessionsListProps): JSX.Element => {
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
        <div class="agent-recent-sessions" data-testid="agent-recent-sessions">
            <div class="agent-recent-sessions-header">
                <span class="agent-recent-sessions-title">Recent sessions</span>
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
                        data-testid="agent-recent-sessions-empty"
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
                                    aria-label={`Reattach to ${row.instance_name}`}
                                    data-testid="agent-recent-sessions-entry"
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
                                        <span class="agent-recent-sessions-line3">
                                            <Show when={row.node_count > 0}>
                                                <span class="agent-recent-sessions-nodes">
                                                    {row.node_count} message
                                                    {row.node_count === 1 ? "" : "s"}
                                                </span>
                                            </Show>
                                            <Show when={row.last_active_at > 0}>
                                                <span class="agent-recent-sessions-when">
                                                    {formatRelative(now(), row.last_active_at)}
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

RecentSessionsList.displayName = "RecentSessionsList";
