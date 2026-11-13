// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The History pane, Agents: every agent whose history AgentMux keeps,
// forks under the agent they were forked from, deleted agents included. A row
// opens the agent's sessions from its record, and its Agent History tab.

import { createEffect, createMemo, createSignal, For, Show, type Accessor, type JSX } from "solid-js";

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

type HistoryAgentsProps = { paneBlockId: string; visible: boolean };

/** What the list reads and does: the live store, the filter, the open sessions and the derived rows. */
interface AgentsList {
    agents: Accessor<HistoryAgent[] | null>;
    error: Accessor<string | null>;
    refreshing: Accessor<boolean>;
    refresh: () => Promise<void>;
    filter: Accessor<string>;
    setFilter: (query: string) => void;
    open: Accessor<Record<string, Sessions>>;
    toggleSessions: (id: string) => Promise<void>;
    openHistory: (a: HistoryAgent) => void;
    rows: Accessor<AgentRow[]>;
    rowIds: Accessor<string[]>;
    rowsById: Accessor<Map<string, AgentRow>>;
    totals: Accessor<{ agents: number; bytes: number; archives: number }>;
}

const now = () => Date.now();

function useAgentsList(props: HistoryAgentsProps): AgentsList {
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
    return { agents, error, refreshing, refresh, filter, setFilter, open, toggleSessions, openHistory, rows, rowIds, rowsById, totals };
}

const AgentsBar = (props: { list: AgentsList }): JSX.Element => (
    <div class="warden-history-bar">
        <FilterInput class="warden-history-filter" value={props.list.filter()} onInput={props.list.setFilter} placeholder="Filter agents" />
        <Button tone="quiet" density="compact" icon="rotate" busy={props.list.refreshing()} onClick={() => void props.list.refresh()}>
            Refresh
        </Button>
    </div>
);

/** The agent's name cell: the sessions toggle, the name (or that it was deleted), its short id and what it was forked from. */
const AgentNameCell = (props: { id: string; row: Accessor<AgentRow>; sessions: Accessor<Sessions | undefined>; list: AgentsList }): JSX.Element => {
    const a = () => props.row().agent;
    return (
        <td class="warden-history-cell-wide">
            <span class="warden-history-agent" style={{ "padding-left": `${props.row().depth * 1.25}em` }}>
                <IconButton
                    icon={props.sessions() ? "chevron-down" : "chevron-right"}
                    label={props.sessions() ? "Hide sessions" : "Show sessions"}
                    tone="quiet"
                    density="compact"
                    onClick={() => void props.list.toggleSessions(props.id)}
                />
                <Show when={!a().deleted} fallback={<span class="warden-manager-dim">Deleted agent</span>}>
                    <span>{a().name}</span>
                </Show>
                <span class="warden-manager-mono warden-manager-dim warden-history-id">{props.id.slice(0, 8)}</span>
                <Show when={a().fork_of}>
                    <span class="warden-manager-dim warden-history-fork">
                        fork of {a().fork_of_name || a().fork_of.slice(0, 8)}
                    </span>
                </Show>
            </span>
        </td>
    );
};

/** One agent's row, and its sessions row under it while they are open. */
const AgentRowItem = (props: { id: string; list: AgentsList }): JSX.Element => {
    // The last row stands while the id is on its way out of the list.
    const row = createMemo<AgentRow>((last) => props.list.rowsById().get(props.id) ?? last!);
    const a = () => row().agent;
    const sessions = () => props.list.open()[props.id];
    return (
        <>
            <tr data-agent={props.id} data-deleted={a().deleted ? "" : undefined}>
                <AgentNameCell id={props.id} row={row} sessions={sessions} list={props.list} />
                <td data-label="Provider">{a().provider || <span class="warden-manager-dim">—</span>}</td>
                <td class="warden-history-num" data-label="History">{formatBytes(a().current_bytes + a().archive_bytes)}</td>
                <td class="warden-history-num" data-label="Archived">{a().archives || <span class="warden-manager-dim">—</span>}</td>
                <td data-label="Last active">{a().last_active_ms ? `${formatAge(ageMs(a().last_active_ms, now()))} ago` : "—"}</td>
                <td class="warden-history-actions warden-history-cell-wide">
                    <Button tone="quiet" density="compact" onClick={() => props.list.openHistory(a())}>
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
};

const AgentsTable = (props: { list: AgentsList }): JSX.Element => (
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
            <For each={props.list.rowIds()}>{(id) => <AgentRowItem id={id} list={props.list} />}</For>
        </tbody>
    </table>
);

export const HistoryAgents = (props: HistoryAgentsProps): JSX.Element => {
    const list = useAgentsList(props);
    const totals = list.totals;

    return (
        <div class="warden-history-agents">
            <AgentsBar list={list} />
            <Show when={list.agents()}>
                <p class="warden-manager-summary">
                    {totals().agents} agents · {formatBytes(totals().bytes)} kept · {totals().archives} archived conversations
                </p>
            </Show>
            <Show when={list.error()}>
                <div class="warden-manager-error">⚠ {list.error()}</div>
            </Show>
            <Show
                when={list.rows().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {list.agents() === null && !list.error()
                            ? "Loading…"
                            : list.filter()
                              ? "No agent matches."
                              : "No agent history yet."}
                    </div>
                }
            >
                <AgentsTable list={list} />
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
                                            <td class="warden-manager-mono warden-history-cell-wide">{x.session_id.slice(0, 8)}</td>
                                            <td data-label="Started">{formatWhen(x.first_ms)}</td>
                                            <td data-label="Last">{formatWhen(x.last_ms)}</td>
                                            <td class="warden-history-num" data-label="Size">{formatBytes(x.bytes)}</td>
                                            <td data-label="Where">{x.archived ? "Archived" : "Current"}</td>
                                            <td
                                                class="warden-manager-mono warden-manager-dim warden-history-cwd warden-history-cell-wide"
                                                data-label="Folder"
                                                {...tip(x.cwd)}
                                            >
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
