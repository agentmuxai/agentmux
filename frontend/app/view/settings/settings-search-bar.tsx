// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Search bar at the top of the Settings pane — docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
// §3.5. Literal first, synonym-aware (via each entry's curated `keywords`
// — §3.4) and typo-tolerant through `searchSettings` (settings-index.ts),
// which uses the same shared `literalFirstSearch` as the command palette and
// the My Agents filter.

import { createMemo, createSignal, For, Show, type Accessor, type JSX } from "solid-js";

import { FilterInput } from "@/app/element/ui";
import { devModeOn } from "@/app/store/dev-mode";
import { SETTINGS_INDEX, searchSettings, settingsPath } from "./settings-index";
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
        const shown = devModeOn() ? SETTINGS_INDEX : SETTINGS_INDEX.filter((e) => !e.devOnly);
        return searchSettings(shown, q).slice(0, MAX_RESULTS);
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
                <FilterInput
                    bare
                    value={props.query()}
                    onInput={(q) => {
                        props.setQuery(q);
                        setSelectedIdx(0);
                    }}
                    onKeyDown={handleKeyDown}
                    inputRef={(el) => (inputRef = el)}
                    placeholder="Search settings..."
                    iconClass="settings-search-icon"
                    inputClass="settings-search-input"
                    clearClass="settings-search-clear"
                    testId="settings-search"
                />
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
                                title={settingsPath(entry).join(" › ")}
                            >
                                <span class="settings-search-result-path">
                                    <span class="settings-search-result-parents">
                                        <bdi>{settingsPath(entry).slice(0, -1).join(" › ")} › </bdi>
                                    </span>
                                    <span class="settings-search-result-label">{entry.label}</span>
                                </span>
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
