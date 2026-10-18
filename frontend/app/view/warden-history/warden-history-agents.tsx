// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — History section, Agents: every agent whose history AgentMux keeps,
// forks under the agent they were forked from, deleted agents included. A row
// opens the agent's sessions from its record, and its Agent History tab.

import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { Button, FilterInput, IconButton } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openOrFocusHistoryTab } from "@/app/view/agent/open-history-tab";
import { ageMs, formatAge } from "@/app/view/warden-shared/warden-shared";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import type { HistoryAgentSessions } from "@/types/rpc/HistoryAgentSessions";
import { formatBytes } from "@/util/format-bytes";
import { agentRows, formatWhen, historyTotals } from "./warden-history-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-history.scss";

type Sessions = { loading: true } | { loading: false; data?: HistoryAgentSessions; error?: string };

export const WardenHistoryAgents = (props: { wardenBlockId: string; visible: boolean }): JSX.Element => {
    const [agents, setAgents] = createSignal<HistoryAgent[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(false);
    const [filter, setFilter] = createSignal("");
    const [open, setOpen] = createSignal<Record<string, Sessions>>({});

    const refresh = async () => {
        setLoading(true);
        try {
            const res = await RpcApi.HistoryAgentsCommand(TabRpcClient, {});
            setAgents(res.agents);
            setError(null);
            // Sessions shown before a refresh may be out of date: read them again when reopened.
            setOpen({});
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    // Loads the first time the section is shown; Refresh reads it again.
    createEffect(() => {
        if (props.visible && agents() === null && !loading()) void refresh();
    });

    const toggleSessions = async (id: string) => {
        if (open()[id]) {
            const { [id]: _, ...rest } = open();
            setOpen(rest);
            return;
        }
        setOpen({ ...open(), [id]: { loading: true } });
        try {
            const data = await RpcApi.HistoryAgentSessionsCommand(TabRpcClient, { definition_id: id });
            if (open()[id]) setOpen({ ...open(), [id]: { loading: false, data } });
        } catch (e) {
            if (open()[id]) setOpen({ ...open(), [id]: { loading: false, error: String(e) } });
        }
    };

    const openHistory = (a: HistoryAgent) =>
        void openOrFocusHistoryTab({
            currentBlockId: props.wardenBlockId,
            agentId: a.definition_id,
            provider: a.provider,
            agentName: a.name || a.definition_id,
        });

    const rows = createMemo(() => agentRows(agents() ?? [], filter()));
    const totals = createMemo(() => historyTotals(agents() ?? []));
    const now = () => Date.now();

    return (
        <div class="warden-history-agents">
            <div class="warden-history-bar">
                <FilterInput value={filter()} onInput={setFilter} placeholder="Filter agents" />
                <Button tone="quiet" density="compact" icon="rotate" busy={loading()} onClick={() => void refresh()}>
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
                        {loading() || agents() === null ? "Loading…" : filter() ? "No agent matches." : "No agent history yet."}
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
                        <For each={rows()}>
                            {(row) => {
                                const a = row.agent;
                                const id = a.definition_id;
                                const sessions = () => open()[id];
                                return (
                                    <>
                                        <tr data-agent={id} data-deleted={a.deleted ? "" : undefined}>
                                            <td>
                                                <span class="warden-history-agent" style={{ "padding-left": `${row.depth * 1.25}em` }}>
                                                    <IconButton
                                                        icon={sessions() ? "chevron-down" : "chevron-right"}
                                                        label={sessions() ? "Hide sessions" : "Show sessions"}
                                                        tone="quiet"
                                                        density="compact"
                                                        onClick={() => void toggleSessions(id)}
                                                    />
                                                    <Show
                                                        when={!a.deleted}
                                                        fallback={<span class="warden-manager-dim">Deleted agent</span>}
                                                    >
                                                        <span>{a.name}</span>
                                                    </Show>
                                                    <span class="warden-manager-mono warden-manager-dim warden-history-id">{id.slice(0, 8)}</span>
                                                    <Show when={a.fork_of}>
                                                        <span class="warden-manager-dim warden-history-fork">
                                                            fork of {a.fork_of_name || a.fork_of.slice(0, 8)}
                                                        </span>
                                                    </Show>
                                                </span>
                                            </td>
                                            <td>{a.provider || <span class="warden-manager-dim">—</span>}</td>
                                            <td class="warden-history-num">{formatBytes(a.current_bytes + a.archive_bytes)}</td>
                                            <td class="warden-history-num">{a.archives || <span class="warden-manager-dim">—</span>}</td>
                                            <td>{a.last_active_ms ? `${formatAge(ageMs(a.last_active_ms, now()))} ago` : "—"}</td>
                                            <td class="warden-history-actions">
                                                <Button tone="quiet" density="compact" onClick={() => openHistory(a)}>
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
                                            <td class="warden-manager-mono warden-manager-dim warden-history-cwd" title={x.cwd}>
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

WardenHistoryAgents.displayName = "WardenHistoryAgents";
