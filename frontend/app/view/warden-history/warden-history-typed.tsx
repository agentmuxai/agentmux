// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — History section, Typed: what the user typed to an agent, newest
// first, from the history srv keeps with the agent (the composer's ArrowUp and
// Ctrl+R read the same; docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md).
// Forgetting an entry, or an agent's whole history, is asked twice.

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, type JSX } from "solid-js";

import { Button, FilterInput, Select } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";
import { formatWhen, typedAgents, typedTags } from "./warden-history-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-history.scss";

/** Entries read at a time (srv's default page). */
const PAGE = 200;
/** How long a search waits after the last keystroke. */
const SEARCH_DELAY_MS = 250;

export const WardenHistoryTyped = (props: { visible: boolean }): JSX.Element => {
    const [agents, setAgents] = createSignal<HistoryAgent[] | null>(null);
    const [agentId, setAgentId] = createSignal("");
    const [query, setQuery] = createSignal("");
    const [entries, setEntries] = createSignal<InputHistoryItem[]>([]);
    const [more, setMore] = createSignal(false);
    const [loading, setLoading] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    // The entry asked "forget this?", or the agent asked "forget everything?".
    const [confirmId, setConfirmId] = createSignal<number | null>(null);
    const [confirmClear, setConfirmClear] = createSignal(false);
    // A read overtaken by another (another agent, another search) must not land.
    let readSeq = 0;

    const list = createMemo(() => typedAgents(agents() ?? []));
    const agentName = () => list().find((a) => a.definition_id === agentId())?.name || agentId().slice(0, 8);

    const loadAgents = async () => {
        try {
            const res = await RpcApi.HistoryAgentsCommand(TabRpcClient, {});
            setAgents(res.agents);
            const ids = typedAgents(res.agents).map((a) => a.definition_id);
            if (!ids.includes(agentId())) setAgentId(ids[0] ?? "");
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    };

    /** The newest page for the agent and search shown, or the page before the oldest shown. */
    const read = async (older: boolean) => {
        const id = agentId();
        const seq = ++readSeq;
        if (!id) {
            setEntries([]);
            setMore(false);
            setLoading(false);
            return;
        }
        const q = query().trim();
        setLoading(true);
        try {
            const before = older ? entries()[entries().length - 1]?.id : undefined;
            const res = q
                ? await RpcApi.AgentInputHistorySearchCommand(TabRpcClient, { agent_id: id, query: q, limit: PAGE })
                : await RpcApi.AgentInputHistoryCommand(TabRpcClient, { agent_id: id, before_id: before, limit: PAGE });
            if (seq !== readSeq) return;
            setEntries(older ? [...entries(), ...res.entries] : res.entries);
            setMore(!q && res.entries.length >= PAGE);
            setError(null);
        } catch (e) {
            if (seq === readSeq) setError(String(e));
        } finally {
            if (seq === readSeq) setLoading(false);
        }
    };

    // Loads the first time the section is shown; Refresh reads it again.
    createEffect(() => {
        if (props.visible && agents() === null) void loadAgents();
    });
    // A different agent: its newest page. Typing in the search: after a pause.
    createEffect(
        on([agentId, query, () => props.visible], ([, , visible]) => {
            setConfirmId(null);
            setConfirmClear(false);
            if (!visible) return;
            const t = setTimeout(() => void read(false), query() ? SEARCH_DELAY_MS : 0);
            onCleanup(() => clearTimeout(t));
        })
    );

    const forget = async (item: InputHistoryItem) => {
        setConfirmId(null);
        try {
            await RpcApi.AgentInputHistoryDeleteCommand(TabRpcClient, { id: item.id });
            setEntries(entries().filter((e) => e.id !== item.id));
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    };
    const clearAll = async () => {
        setConfirmClear(false);
        try {
            await RpcApi.AgentInputHistoryClearCommand(TabRpcClient, { agent_id: agentId() });
            setEntries([]);
            setMore(false);
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    };

    const tags = (item: InputHistoryItem): string =>
        [...typedTags(item.kind, item.source), ...(item.sends > 1 ? [`sent ${item.sends} times in a row`] : [])].join(" · ");

    return (
        <div class="warden-history-typed">
            <div class="warden-history-bar">
                <Select
                    ariaLabel="Agent"
                    density="compact"
                    value={agentId()}
                    onChange={setAgentId}
                    disabled={list().length === 0}
                    options={list().map((a) => ({ value: a.definition_id, label: a.name || a.definition_id.slice(0, 8) }))}
                />
                <FilterInput value={query()} onInput={setQuery} placeholder="Search what was typed" />
                <Button
                    tone="quiet"
                    density="compact"
                    icon="rotate"
                    busy={loading()}
                    onClick={() => {
                        void loadAgents();
                        void read(false);
                    }}
                >
                    Refresh
                </Button>
                <Show when={entries().length > 0 && !query()}>
                    <Show
                        when={confirmClear()}
                        fallback={
                            <Button tone="quiet" density="compact" onClick={() => setConfirmClear(true)}>
                                Clear…
                            </Button>
                        }
                    >
                        <Button tone="danger" density="compact" onClick={() => void clearAll()}>
                            {`Forget everything typed to ${agentName()}`}
                        </Button>
                        <Button tone="quiet" density="compact" onClick={() => setConfirmClear(false)}>
                            Cancel
                        </Button>
                    </Show>
                </Show>
            </div>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={entries().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {agents() === null || loading()
                            ? "Loading…"
                            : list().length === 0
                              ? "No agent to show."
                              : query()
                                ? "Nothing typed to this agent matches."
                                : "Nothing typed to this agent yet."}
                    </div>
                }
            >
                <table class="warden-manager-table warden-history-table warden-history-typed-table">
                    <thead>
                        <tr>
                            <th>When</th>
                            <th>Typed</th>
                            <th />
                        </tr>
                    </thead>
                    <tbody>
                        <For each={entries()}>
                            {(item) => (
                                <tr data-entry={item.id}>
                                    <td class="warden-history-typed-when">{formatWhen(item.sent_at_ms)}</td>
                                    <td>
                                        <div class="warden-history-typed-text" title={item.text}>
                                            {item.text}
                                        </div>
                                        <Show when={tags(item)}>
                                            <div class="warden-manager-dim warden-history-typed-tags">{tags(item)}</div>
                                        </Show>
                                    </td>
                                    <td class="warden-history-actions">
                                        <Show
                                            when={confirmId() === item.id}
                                            fallback={
                                                <Button tone="quiet" density="compact" onClick={() => setConfirmId(item.id)}>
                                                    Forget
                                                </Button>
                                            }
                                        >
                                            <Button tone="danger" density="compact" onClick={() => void forget(item)}>
                                                Forget this
                                            </Button>
                                            <Button tone="quiet" density="compact" onClick={() => setConfirmId(null)}>
                                                Cancel
                                            </Button>
                                        </Show>
                                    </td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
                <Show when={more()}>
                    <Button tone="quiet" density="compact" busy={loading()} onClick={() => void read(true)}>
                        Show older
                    </Button>
                </Show>
            </Show>
            <div class="warden-manager-footnote">
                What was typed to an agent is kept with the agent, up to its limit (the agent:inputhistory:max setting), and
                goes when the agent is deleted. Forgetting is permanent.
            </div>
        </div>
    );
};

WardenHistoryTyped.displayName = "WardenHistoryTyped";
