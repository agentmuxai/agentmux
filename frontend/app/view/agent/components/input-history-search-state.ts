// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Ctrl+R in the composer, the state: the entries srv has for the query, which one is selected, and the keys, forget
// and clear that act on them. (input-history-search-parts.tsx draws them.)

import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { clearInputHistory, forgetInputHistoryEntry } from "../agent-input-history";

const LIMIT = 50;
/** How long typing settles before srv is asked. */
const SEARCH_DELAY_MS = 120;

export interface InputHistorySearchProps {
    agentId: string;
    /** Close; with the entry to put in the composer, when one was picked. */
    onDone: (text?: string) => void;
}

export type InputHistorySearchState = ReturnType<typeof createInputHistorySearch>;

export function createInputHistorySearch(props: InputHistorySearchProps) {
    const [query, setQuery] = createSignal("");
    const [items, setItems] = createSignal<InputHistoryItem[]>([]);
    const [selected, setSelected] = createSignal(0);
    const [confirmClear, setConfirmClear] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    let input: HTMLInputElement | undefined;
    let list: HTMLDivElement | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let asked = 0;
    const setInput = (el: HTMLInputElement) => {
        input = el;
    };
    const setList = (el: HTMLDivElement) => {
        list = el;
    };

    const load = (q: string) => {
        const mine = ++asked;
        const read = q
            ? RpcApi.AgentInputHistorySearchCommand(TabRpcClient, { agent_id: props.agentId, query: q, limit: LIMIT })
            : RpcApi.AgentInputHistoryCommand(TabRpcClient, { agent_id: props.agentId, limit: LIMIT });
        read.then((res) => {
            if (mine !== asked) return; // a later query answered first, or will
            setItems(res.entries);
            setSelected(0);
        }).catch(() => {});
    };
    createEffect(
        on(query, (q) => {
            clearTimeout(timer);
            timer = setTimeout(() => load(q), q ? SEARCH_DELAY_MS : 0);
        })
    );
    onCleanup(() => clearTimeout(timer));
    // Keep the selected row in sight as the arrows move past the visible ones.
    createEffect(
        on(selected, (i) => (list?.children[i] as HTMLElement | undefined)?.scrollIntoView?.({ block: "nearest" }))
    );
    onMount(() => input?.focus());

    const move = (by: number) => {
        const n = items().length;
        if (n > 0) setSelected((i) => (i + by + n) % n);
    };
    const forgetSelected = () => {
        const item = items()[selected()];
        if (!item) return;
        setError(null);
        forgetInputHistoryEntry(props.agentId, item.id)
            .then(() => {
                setItems((entries) => entries.filter((e) => e.id !== item.id));
                setSelected((i) => Math.min(i, Math.max(0, items().length - 1)));
            })
            .catch((e) => setError(`Couldn't forget it: ${String(e?.message ?? e)}`));
    };
    const clearAll = () => {
        setError(null);
        clearInputHistory(props.agentId)
            .then(() => setItems([]))
            .catch((e) => setError(`Couldn't clear the history: ${String(e?.message ?? e)}`))
            .finally(() => {
                setConfirmClear(false);
                input?.focus();
            });
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.preventDefault();
            props.onDone();
        } else if (e.key === "Enter") {
            e.preventDefault();
            const item = items()[selected()];
            props.onDone(item?.text);
        } else if (e.key === "ArrowDown" || (e.key === "r" && e.ctrlKey)) {
            e.preventDefault();
            move(1);
        } else if (e.key === "ArrowUp") {
            e.preventDefault();
            move(-1);
        } else if (e.key === "Delete" && e.shiftKey) {
            e.preventDefault();
            forgetSelected();
        }
    };

    return { query, setQuery, items, selected, setSelected, confirmClear, setConfirmClear, error, setInput, setList, clearAll, onKeyDown };
}
