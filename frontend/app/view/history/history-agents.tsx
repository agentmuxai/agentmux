// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The History pane, Agents: every agent whose history AgentMux keeps,
// forks under the agent they were forked from, deleted agents included. A row
// opens the agent's sessions from its record, and its Agent History tab.

import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { Button, FilterInput, IconButton } from "@/app/element/ui";
import { createDomainStore } from "@/app/store/domain-store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openOrFocusHistoryTab } from "@/app/view/agent/open-history-tab";
import { ageMs, formatAge } from "@/app/view/warden-shared/warden-shared";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import { formatBytes } from "@/util/format-bytes";
import { agentRows, formatWhen, historyTotals, type AgentRow } from "./history-shared";
import { useOpenSessions, type Sessions } from "./history-sessions";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./history.scss";
import { tip } from "@/app/element/tip";

/** srv announces a change at most every two seconds while a transcript grows; a write by another AgentMux sharing the store is not announced, so the list also reads again this often. */
const SAFETY_POLL_MS = 30_000;

export const HistoryAgents = (props: { paneBlockId: string; visible: boolean }): JSX.Element => {
    // Live while the section is shown: a new session, a growing transcript or a renamed agent updates it.
    const store = createDomainStore<HistoryAgent[] | null>({
        initial: null,
        load: async () => (await RpcApi.HistoryAgentsCommand(TabRpcClient, {})).agents,
        events: ["AgentsChanged", "HistorySessionsChanged"],
        debounceMs: 500,
        safetyPollMs: SAFETY_POLL_MS,
    });
    const agents = store.data;
    const error = store.error;
    const [refreshing, setRefreshing] = createSignal(false);
    const [filter, setFilter] = createSignal("");
    const { open, closeAll, toggleSessions } = useOpenSessions(agents);

    // Refresh reads the list again and drops the sessions shown, which are read again when reopened.
    const refresh = async () => {
        setRefreshing(true);
        try {
            await store.refresh();
            closeAll();
        } finally {
            setRefreshing(false);
        }
    };

    // Follows while the section is shown (the first load is the follow's), and stops when it is hidden.
    createEffect(() => {
        if (props.visible) store.follow();
    });

    const openHistory = (a: HistoryAgent) =>
        void openOrFocusHistoryTab({
            currentBlockId: props.paneBlockId,
            agentId: a.definition_id,
            provider: a.provider,
            agentName: a.name || a.definition_id,
        });

    const rows = createMemo(() => agentRows(agents() ?? [], filter()));
    // The list is keyed by agent id, so a reload that changes one agent's numbers updates its cells in
    // place and leaves the other rows, and any open sessions, alone.
    const rowIds = createMemo(() => rows().map((r) => r.agent.definition_id));
    const rowsById = createMemo(() => new Map(rows().map((r) => [r.agent.definition_id, r])));
    const totals = createMemo(() => historyTotals(agents() ?? []));
    const now = () => Date.now();

    return (
        <div class="warden-history-agents">
            <div class="warden-history-bar">
                <FilterInput value={filter()} onInput={setFilter} placeholder="Filter agents" />
                <Button tone="quiet" density="compact" icon="rotate" busy={refreshing()} onClick={() => void refresh()}>
                    Refresh
                </Button>
            </div>
            <Show when={agents()}>
                <p class="warden-manager-summary">
                    {totals().agents} agents · {formatBytes(totals().bytes)} kept · {totals().archives} archived conversations
                </p>
            </Show>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={rows().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {agents() === null && !error() ? "Loading…" : filter() ? "No agent matches." : "No agent history yet."}
                    </div>
                }
            >
                <table class="warden-manager-table warden-history-table">
                    <thead>
                        <tr>
                            <th>Agent</th>
                            <th>Provider</th>
                            <th class="warden-history-num">History</th>
                            <th class="warden-history-num">Archived</th>
                            <th>Last active</th>
                            <th />
                        </tr>
                    </thead>
                    <tbody>
                        <For each={rowIds()}>
                            {(id) => {
                                // The last row stands while the id is on its way out of the list.
                                const row = createMemo<AgentRow>((last) => rowsById().get(id) ?? last!);
                                const a = () => row().agent;
                                const sessions = () => open()[id];
                                return (
                                    <>
                                        <tr data-agent={id} data-deleted={a().deleted ? "" : undefined}>
                                            <td>
                                                <span class="warden-history-agent" style={{ "padding-left": `${row().depth * 1.25}em` }}>
                                                    <IconButton
                                                        icon={sessions() ? "chevron-down" : "chevron-right"}
                                                        label={sessions() ? "Hide sessions" : "Show sessions"}
                                                        tone="quiet"
                                                        density="compact"
                                                        onClick={() => void toggleSessions(id)}
                                                    />
                                                    <Show
                                                        when={!a().deleted}
                                                        fallback={<span class="warden-manager-dim">Deleted agent</span>}
                                                    >
                                                        <span>{a().name}</span>
                                                    </Show>
                                                    <span class="warden-manager-mono warden-manager-dim warden-history-id">{id.slice(0, 8)}</span>
                                                    <Show when={a().fork_of}>
                                                        <span class="warden-manager-dim warden-history-fork">
                                                            fork of {a().fork_of_name || a().fork_of.slice(0, 8)}
                                                        </span>
                                                    </Show>
                                                </span>
                                            </td>
                                            <td>{a().provider || <span class="warden-manager-dim">—</span>}</td>
                                            <td class="warden-history-num">{formatBytes(a().current_bytes + a().archive_bytes)}</td>
                                            <td class="warden-history-num">{a().archives || <span class="warden-manager-dim">—</span>}</td>
                                            <td>{a().last_active_ms ? `${formatAge(ageMs(a().last_active_ms, now()))} ago` : "—"}</td>
                                            <td class="warden-history-actions">
                                                <Button tone="quiet" density="compact" onClick={() => openHistory(a())}>
                                                    Open history
                                                </Button>
                                            </td>
                                        </tr>
                                        <Show when={sessions()}>
                                            {(s) => (
                                                <tr class="warden-history-sessions-row">
                                                    <td colSpan={6}>
                                                        <SessionList sessions={s()} />
                                                    </td>
                                                </tr>
                                            )}
                                        </Show>
                                    </>
                                );
                            }}
                        </For>
                    </tbody>
                </table>
            </Show>
            <div class="warden-manager-footnote">
                Kept forever. "Open history" shows an agent's whole recorded conversation in this pane.
            </div>
        </div>
    );
};

const SessionList = (props: { sessions: Sessions }): JSX.Element => (
    <Show when={!props.sessions.loading} fallback={<div class="warden-section-stub">Reading the record…</div>}>
        {(() => {
            const s = props.sessions as Extract<Sessions, { loading: false }>;
            return (
                <>
                    <Show when={s.error}>
                        <div class="warden-manager-error">⚠ {s.error}</div>
                    </Show>
                    <For each={s.data?.problems ?? []}>{(p) => <div class="warden-manager-error">⚠ {p}</div>}</For>
                    <Show
                        when={(s.data?.sessions.length ?? 0) > 0}
                        fallback={<Show when={!s.error}><div class="warden-section-stub">No sessions in the record.</div></Show>}
                    >
                        <table class="warden-manager-table warden-history-sessions">
                            <thead>
                                <tr>
                                    <th>Session</th>
                                    <th>Started</th>
                                    <th>Last</th>
                                    <th class="warden-history-num">Size</th>
                                    <th>Where</th>
                                    <th>Folder</th>
                                </tr>
                            </thead>
                            <tbody>
                                <For each={s.data!.sessions}>
                                    {(x) => (
                                        <tr data-session={x.session_id}>
                                            <td class="warden-manager-mono">{x.session_id.slice(0, 8)}</td>
                                            <td>{formatWhen(x.first_ms)}</td>
                                            <td>{formatWhen(x.last_ms)}</td>
                                            <td class="warden-history-num">{formatBytes(x.bytes)}</td>
                                            <td>{x.archived ? "Archived" : "Current"}</td>
                                            <td class="warden-manager-mono warden-manager-dim warden-history-cwd" {...tip(x.cwd)}>
                                                {x.cwd || "—"}
                                            </td>
                                        </tr>
                                    )}
                                </For>
                            </tbody>
                        </table>
                    </Show>
                </>
            );
        })()}
    </Show>
);

HistoryAgents.displayName = "HistoryAgents";
