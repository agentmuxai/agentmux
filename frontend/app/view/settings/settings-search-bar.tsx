// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Search bar at the top of the Settings pane — docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
// §3.5. Typo-tolerant and synonym-aware (via each entry's curated `keywords`
// — §3.4) through the shared `fuzzySearch` utility (§2), same one the
// command palette and agent-picker filter use.

import { createMemo, createSignal, For, Show, type Accessor, type JSX } from "solid-js";

import { fuzzySearch } from "@/app/util/fuzzysearch";
import { SETTINGS_INDEX } from "./settings-index";
import type { SettingsIndexEntry } from "./settings-model";
import "./settings-search-bar.scss";

const MAX_RESULTS = 8;

export interface SettingsSearchBarProps {
    query: Accessor<string>;
    setQuery: (q: string) => void;
    onSelectResult: (entry: SettingsIndexEntry) => void;
}

export function SettingsSearchBar(props: SettingsSearchBarProps): JSX.Element {
    const [selectedIdx, setSelectedIdx] = createSignal(0);
    let inputRef!: HTMLInputElement;

    const results = createMemo(() => {
        const q = props.query().trim();
        if (!q) return [];
        return fuzzySearch(SETTINGS_INDEX, q, {
            keys: [
                { name: "label", weight: 0.45 },
                { name: "keywords", weight: 0.35 },
                { name: "description", weight: 0.2 },
            ],
        }).slice(0, MAX_RESULTS);
    });

    const isOpen = () => props.query().trim().length > 0;

    const clampedIdx = createMemo(() => {
        const len = results().length;
        if (len === 0) return 0;
        return Math.min(selectedIdx(), len - 1);
    });

    function selectEntry(entry: SettingsIndexEntry) {
        props.onSelectResult(entry);
        props.setQuery("");
        setSelectedIdx(0);
    }

    function handleInput(e: InputEvent) {
        props.setQuery((e.target as HTMLInputElement).value);
        setSelectedIdx(0);
    }

    function handleKeyDown(e: KeyboardEvent) {
        if (!isOpen()) return;
        if (e.key === "ArrowDown") {
            e.preventDefault();
            setSelectedIdx((i) => Math.min(i + 1, results().length - 1));
            return;
        }
        if (e.key === "ArrowUp") {
            e.preventDefault();
            setSelectedIdx((i) => Math.max(i - 1, 0));
            return;
        }
        if (e.key === "Enter") {
            e.preventDefault();
            const entry = results()[clampedIdx()];
            if (entry) selectEntry(entry);
            return;
        }
        if (e.key === "Escape") {
            e.preventDefault();
            props.setQuery("");
            inputRef?.blur();
        }
    }

    return (
        <div class="settings-search-bar" data-testid="settings-search-bar">
            <div class="settings-search-input-row">
                <i class="fa-solid fa-magnifying-glass settings-search-icon" aria-hidden="true" />
                <input
                    ref={inputRef}
                    class="settings-search-input"
                    type="text"
                    placeholder="Search settings..."
                    value={props.query()}
                    data-testid="settings-search-input"
                    onInput={handleInput}
                    onKeyDown={handleKeyDown}
                    autocomplete="off"
                    spellcheck={false}
                />
                <Show when={props.query()}>
                    <button
                        type="button"
                        class="settings-search-clear"
                        onClick={() => props.setQuery("")}
                        aria-label="Clear search"
                        data-testid="settings-search-clear"
                    >
                        &times;
                    </button>
                </Show>
            </div>
            <Show when={isOpen()}>
                <div class="settings-search-results" data-testid="settings-search-results">
                    <For each={results()}>
                        {(entry, i) => (
                            <div
                                class={`settings-search-result${i() === clampedIdx() ? " selected" : ""}`}
                                data-testid="settings-search-result"
                                onClick={() => selectEntry(entry)}
                                onMouseEnter={() => setSelectedIdx(i())}
                            >
                                <span class="settings-search-result-label">{entry.label}</span>
                                <Show when={entry.description}>
                                    <span class="settings-search-result-desc">{entry.description}</span>
                                </Show>
                            </div>
                        )}
                    </For>
                    <Show when={results().length === 0}>
                        <div class="settings-search-empty" data-testid="settings-search-empty">
                            No settings match "{props.query()}"
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
}

SettingsSearchBar.displayName = "SettingsSearchBar";
