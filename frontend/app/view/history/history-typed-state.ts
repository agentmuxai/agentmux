// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Typed pane's state (history-typed.tsx): the agents it can show, the page of entries read for the agent and
// search shown, the agent's trash, and forgetting, clearing and restoring. The effects are registered in the order
// the pane always registered them.

import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor, type Setter } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { clearInputHistory, forgetInputHistoryEntry, restoreInputHistory } from "@/app/view/agent/agent-input-history";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";
import type { InputHistoryTrashItem } from "@/types/rpc/InputHistoryTrashItem";
import { typedAgents, type TypedAgent } from "./history-shared";

/** Entries read at a time (srv's default page). */
const PAGE = 200;
/** How long a search waits after the last keystroke. */
const SEARCH_DELAY_MS = 250;

type SetError = Setter<string | null>;

/** The agents the pane lists, and which one is shown. */
function createRoster(setError: SetError) {
    const [agents, setAgents] = createSignal<TypedAgent[] | null>(null);
    const [agentId, setAgentId] = createSignal("");
    const list = createMemo(() => agents() ?? []);
    const agentName = () => list().find((a) => a.id === agentId())?.name || agentId().slice(0, 8);

    const loadAgents = async () => {
        try {
            // Hidden agents too: what was typed to them is still kept.
            const defs = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient, { include_hidden: true });
            const found = typedAgents(defs);
            setAgents(found);
            if (!found.some((a) => a.id === agentId())) setAgentId(found[0]?.id ?? "");
            setError(null);
        } catch (e) {
            // Not "Loading…" under the error: Refresh tries again.
            if (agents() === null) setAgents([]);
            setError(String(e));
        }
    };
    return { agents, agentId, setAgentId, list, agentName, loadAgents };
}

/** The entries read for the agent and search shown. A read overtaken by another must not land. */
function createReader(agentId: Accessor<string>, setError: SetError) {
    const [query, setQuery] = createSignal("");
    const [entries, setEntries] = createSignal<InputHistoryItem[]>([]);
    const [more, setMore] = createSignal(false);
    const [loading, setLoading] = createSignal(false);
    let readSeq = 0;

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
    return { query, setQuery, entries, setEntries, more, setMore, loading, read };
}

/** What was forgotten and can still come back, for the agent shown. */
function createTrash(agentId: Accessor<string>, setError: SetError) {
    const [trash, setTrash] = createSignal<InputHistoryTrashItem[]>([]);
    const [retentionDays, setRetentionDays] = createSignal(7);
    let trashSeq = 0;

    const readTrash = async () => {
        const id = agentId();
        const seq = ++trashSeq;
        if (!id) {
            setTrash([]);
            return;
        }
        try {
            const res = await RpcApi.AgentInputHistoryTrashCommand(TabRpcClient, { agent_id: id });
            if (seq !== trashSeq) return;
            setTrash(res.batches);
            setRetentionDays(Math.round(res.retention_ms / (24 * 60 * 60 * 1000)));
        } catch (e) {
            if (seq === trashSeq) setError(String(e));
        }
    };
    return { trash, retentionDays, readTrash };
}

// Forgetting and restoring go through the composers' shared cache (agent-input-history.ts), not the RPC
// alone: an open composer would otherwise still recall, and resend, what was forgotten. The agent is the one
// asked about, not the one shown when the answer comes back.
function createForgetting(
    agentId: Accessor<string>,
    reader: ReturnType<typeof createReader>,
    trash: ReturnType<typeof createTrash>,
    setError: SetError,
) {
    // The entry asked "forget this?", or the agent asked "forget everything?".
    const [confirmId, setConfirmId] = createSignal<number | null>(null);
    const [confirmClear, setConfirmClear] = createSignal(false);
    const { entries, setEntries, setMore, read } = reader;
    const { readTrash } = trash;

    const forget = async (item: InputHistoryItem) => {
        const target = agentId();
        setConfirmId(null);
        try {
            await forgetInputHistoryEntry(target, item.id);
            if (agentId() !== target) return;
            setEntries(entries().filter((e) => e.id !== item.id));
            setError(null);
            void readTrash();
        } catch (e) {
            if (agentId() === target) setError(String(e));
        }
    };
    // The composers read the restored entries again on srv's `restored` event.
    const restore = async (batch: InputHistoryTrashItem) => {
        const target = agentId();
        try {
            await restoreInputHistory(batch.batch_id);
            if (agentId() !== target) return;
            setError(null);
            void read(false);
            void readTrash();
        } catch (e) {
            if (agentId() === target) setError(String(e));
        }
    };
    const clearAll = async () => {
        const target = agentId();
        setConfirmClear(false);
        try {
            await clearInputHistory(target);
            if (agentId() !== target) return;
            setEntries([]);
            setMore(false);
            setError(null);
            void readTrash();
        } catch (e) {
            if (agentId() === target) setError(String(e));
        }
    };
    return { confirmId, setConfirmId, confirmClear, setConfirmClear, forget, restore, clearAll };
}

/** The Typed pane's state: everything its markup reads, and what its controls do. */
export function createHistoryTyped(props: { visible: boolean }) {
    const [error, setError] = createSignal<string | null>(null);
    const roster = createRoster(setError);
    const reader = createReader(roster.agentId, setError);
    const trash = createTrash(roster.agentId, setError);
    const forgetting = createForgetting(roster.agentId, reader, trash, setError);
    const { agents, agentId, loadAgents } = roster;
    const { query, read } = reader;
    const { readTrash } = trash;

    // Loads the first time the section is shown; Refresh reads it again.
    createEffect(() => {
        if (props.visible && agents() === null) void loadAgents();
    });
    // A different agent: its newest page. Typing in the search: after a pause.
    createEffect(
        on([agentId, query, () => props.visible], ([, , visible]) => {
            forgetting.setConfirmId(null);
            forgetting.setConfirmClear(false);
            if (!visible) return;
            const t = setTimeout(() => void read(false), query() ? SEARCH_DELAY_MS : 0);
            onCleanup(() => clearTimeout(t));
        }),
    );
    // The trash belongs to the agent, not to the search.
    createEffect(
        on([agentId, () => props.visible], ([, visible]) => {
            if (visible) void readTrash();
        }),
    );

    return { error, ...roster, ...reader, ...trash, ...forgetting };
}

/** What the Typed pane's markup reads. */
export type HistoryTypedState = ReturnType<typeof createHistoryTyped>;
