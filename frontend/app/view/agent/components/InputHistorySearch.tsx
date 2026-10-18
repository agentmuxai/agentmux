// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ctrl+R in the composer: search what was typed to this agent, as a shell's
 * reverse search does (docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md §4.6).
 * Type to filter, Up/Down (or Ctrl+R again) to move, Enter to put the entry in
 * the composer, Escape to go back to it. Shift+Delete forgets the selected
 * entry; "Clear history" forgets them all, after asking.
 */

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";
import { createEffect, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { clearInputHistory, forgetInputHistoryEntry } from "../agent-input-history";

const LIMIT = 50;
/** How long typing settles before srv is asked. */
const SEARCH_DELAY_MS = 120;

/** "14:05", "Oct 3", for when an entry was last sent. */
function sentAt(ms: number, now = new Date()): string {
    const at = new Date(ms);
    return at.toDateString() === now.toDateString()
        ? at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : at.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

export function InputHistorySearch(props: {
    agentId: string;
    /** Close; with the entry to put in the composer, when one was picked. */
    onDone: (text?: string) => void;
}): JSX.Element {
    const [query, setQuery] = createSignal("");
    const [items, setItems] = createSignal<InputHistoryItem[]>([]);
    const [selected, setSelected] = createSignal(0);
    const [confirmClear, setConfirmClear] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    let input: HTMLInputElement | undefined;
    let list: HTMLDivElement | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let asked = 0;

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

    return (
        <div class="input-history-search" role="dialog" aria-label="Search what you typed to this agent">
            <div class="input-history-search__query">
                <span>Search history</span>
                <input
                    ref={input}
                    type="text"
                    value={query()}
                    placeholder="Type to filter"
                    spellcheck={false}
                    aria-controls="input-history-search-list"
                    onInput={(e) => setQuery(e.currentTarget.value)}
                    onKeyDown={onKeyDown}
                    onBlur={(e) => {
                        // Clicking a row or a button inside keeps it open.
                        const next = e.relatedTarget as Node | null;
                        if (!next || !e.currentTarget.closest(".input-history-search")?.contains(next)) props.onDone();
                    }}
                />
            </div>
            <div class="input-history-search__list" id="input-history-search-list" role="listbox" ref={list}>
                <For each={items()}>
                    {(item, i) => (
                        <div
                            class="input-history-search__row"
                            classList={{ "input-history-search__row--selected": i() === selected() }}
                            role="option"
                            aria-selected={i() === selected()}
                            title={item.text}
                            onMouseEnter={() => setSelected(i())}
                            onMouseDown={(e) => {
                                e.preventDefault();
                                props.onDone(item.text);
                            }}
                        >
                            <span class="input-history-search__text">{item.text.split("\n")[0]}</span>
                            <span class="input-history-search__meta">
                                {item.sends > 1 ? `${item.sends}× · ` : ""}
                                {sentAt(item.sent_at_ms)}
                            </span>
                        </div>
                    )}
                </For>
                <Show when={items().length === 0}>
                    <div class="input-history-search__empty">{query() ? "Nothing typed to this agent matches." : "Nothing typed to this agent yet."}</div>
                </Show>
            </div>
            <Show when={error()}>
                <div class="input-history-search__empty" role="alert">{error()}</div>
            </Show>
            <div class="input-history-search__hint">
                <span>Enter to use</span>
                <span>Shift+Del to forget</span>
                <span>Esc to close</span>
                <span class="input-history-search__clear">
                    <Show
                        when={confirmClear()}
                        fallback={
                            <Button density="compact" onMouseDown={(e) => e.preventDefault()} onClick={() => setConfirmClear(true)} disabled={items().length === 0 && !query()}>
                                Clear history
                            </Button>
                        }
                    >
                        <Button density="compact" tone="danger" onMouseDown={(e) => e.preventDefault()} onClick={clearAll}>
                            Forget everything typed to this agent
                        </Button>
                    </Show>
                </span>
            </div>
        </div>
    );
}
