// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Ctrl+R in the composer, the views: the query box, the list of entries and the hint line with Clear history, over the
// state of input-history-search-state.ts.

import { Button, TextInput } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";
import { tip } from "@/app/element/tip";
import { formatLocale } from "@/app/i18n/i18n";
import { formatTimeOrDate } from "@/util/format-time";
import type { InputHistorySearchProps, InputHistorySearchState } from "./input-history-search-state";

/** "14:05", "Oct 3", for when an entry was last sent. */
function sentAt(ms: number, now = new Date(), locale?: string): string {
    return formatTimeOrDate(ms, now, locale);
}

type Parts = { s: InputHistorySearchState };

/** The query box; losing focus to anything outside the search closes it. */
export function SearchField(props: Parts & Pick<InputHistorySearchProps, "onDone">): JSX.Element {
    const { s, onDone } = props;
    return (
        <div class="input-history-search__query">
            <span>Search history</span>
            <TextInput
                variant="plain"
                ref={s.setInput}
                value={s.query()}
                placeholder="Type to filter"
                spellcheck={false}
                aria-controls="input-history-search-list"
                onInput={(e) => s.setQuery(e.currentTarget.value)}
                onKeyDown={s.onKeyDown}
                onBlur={(e) => {
                    // Clicking a row or a button inside keeps it open.
                    const next = e.relatedTarget as Node | null;
                    if (!next || !e.currentTarget.closest(".input-history-search")?.contains(next)) onDone();
                }}
            />
        </div>
    );
}

/** The entries, the selected one marked, or why there are none. */
export function EntryList(props: Parts & Pick<InputHistorySearchProps, "onDone">): JSX.Element {
    const { s, onDone } = props;
    return (
        <div class="input-history-search__list" id="input-history-search-list" role="listbox" ref={s.setList}>
            <For each={s.items()}>
                {(item, i) => (
                    <div
                        class="input-history-search__row"
                        classList={{ "input-history-search__row--selected": i() === s.selected() }}
                        role="option"
                        aria-selected={i() === s.selected()}
                        {...tip(item.text)}
                        onMouseEnter={() => s.setSelected(i())}
                        onMouseDown={(e) => {
                            e.preventDefault();
                            onDone(item.text);
                        }}
                    >
                        <span class="input-history-search__text">{item.text.split("\n")[0]}</span>
                        <span class="input-history-search__meta">
                            {item.sends > 1 ? `${item.sends}× · ` : ""}
                            {sentAt(item.sent_at_ms, new Date(), formatLocale())}
                        </span>
                    </div>
                )}
            </For>
            <Show when={s.items().length === 0}>
                <div class="input-history-search__empty">{s.query() ? "Nothing typed to this agent matches." : "Nothing typed to this agent yet."}</div>
            </Show>
        </div>
    );
}

/** What went wrong with a forget or a clear, then the keys and Clear history (which asks again before it does it). */
export function SearchFooter(props: Parts): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={s.error()}>
                <div class="input-history-search__empty" role="alert">{s.error()}</div>
            </Show>
            <div class="input-history-search__hint">
                <span>Enter to use</span>
                <span>Shift+Del to forget</span>
                <span>Esc to close</span>
                <span class="input-history-search__clear">
                    <Show
                        when={s.confirmClear()}
                        fallback={
                            <Button density="compact" onMouseDown={(e) => e.preventDefault()} onClick={() => s.setConfirmClear(true)} disabled={s.items().length === 0 && !s.query()}>
                                Clear history
                            </Button>
                        }
                    >
                        <Button density="compact" tone="danger" onMouseDown={(e) => e.preventDefault()} onClick={s.clearAll}>
                            Forget everything typed to this agent
                        </Button>
                    </Show>
                </span>
            </div>
        </>
    );
}
