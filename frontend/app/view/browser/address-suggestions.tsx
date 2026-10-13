// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The address bar's suggestion list (address-autocomplete.ts). It drops over
 * the pane's page, which is a native window above the DOM, so it registers as
 * a pane overlay: a hole on Windows, the page hidden behind a still on macOS
 * and Linux (platform/pane-overlay.ts). It exists only while open, and its rect
 * changes per keystroke at most, never per frame.
 */

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import clsx from "clsx";
import { For, type JSX } from "solid-js";

/** Font Awesome icon for a Chrome match type. */
export function suggestionIcon(type: string): string {
    if (type.startsWith("search")) return "magnifying-glass";
    if (type.startsWith("history") || type === "shortcuts") return "clock-rotate-left";
    if (type.startsWith("bookmark")) return "bookmark";
    return "globe";
}

export function AddressSuggestions(props: {
    matches: BrowserAutocompleteMatch[];
    /** The keyboard highlight. Hover is only a style (`:hover`): it never
     *  changes what Enter opens, as in Chrome. */
    selected: number;
    onChoose: (match: BrowserAutocompleteMatch) => void;
}): JSX.Element {
    let el: HTMLDivElement | undefined;
    usePaneOverlay(() => el);
    return (
        <div class="browser-address-suggestions" ref={el} role="listbox">
            <For each={props.matches}>
                {(m, i) => (
                    <div
                        class={clsx("browser-address-suggestion", { selected: i() === props.selected })}
                        role="option"
                        aria-selected={i() === props.selected}
                        // Keep focus in the address bar: a click must not blur it first.
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => props.onChoose(m)}
                        title={m.url}
                    >
                        <i class={clsx("fa fa-solid fa-fw", `fa-${suggestionIcon(m.type)}`)} aria-hidden="true" />
                        <span class="browser-address-suggestion-text">{m.contents}</span>
                        {m.description && m.description !== m.contents ? (
                            <span class="browser-address-suggestion-desc">{m.description}</span>
                        ) : null}
                    </div>
                )}
            </For>
        </div>
    );
}
