// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Command palette modal — opened via Ctrl+P, lists all registered commands.
// Uses the canonical `element/modal` primitive, hung from the top of the window with no backdrop; the palette's
// specific chrome (search row + scroll list) lives in the panel body.

import { commandRegistry, isCommandAvailable, type CommandEntry } from "@/app/store/command-registry";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import { disableGlobalKeybindings, enableGlobalKeybindings } from "@/app/store/keymodel";
import { isKeyFor } from "@/app/keybindings";
import { SearchIcon } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Modal } from "@/element/modal";
import { searchCommands } from "./command-palette-search";
import { usePaletteAgents } from "./palette-agents";
import { paletteRows, paletteSegments, sortCommands } from "./palette-groups";
import { PaletteRow } from "./palette-row";
import { createMemo, createSignal, createUniqueId, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import "./command-palette.scss";

/**
 * The row a navigation key selects from row `i` (`last` is the last row), or null for any other key. Up and Down
 * wrap at the ends (R17); a page, Home and End stop there.
 */
function selectionAfterKey(key: string, i: number, last: number, page: number): number | null {
    const to = (row: number) => Math.max(0, Math.min(row, last));
    switch (key) {
        case "ArrowDown":
            return i >= last ? 0 : i + 1;
        case "ArrowUp":
            return i <= 0 ? Math.max(last, 0) : i - 1;
        case "PageDown":
            return to(i + page);
        case "PageUp":
            return to(i - page);
        case "Home":
            return 0;
        case "End":
            return to(last);
        default:
            return null;
    }
}

/** How many rows the list shows at once, for PageUp and PageDown (at least 1). */
function rowsPerPage(list: HTMLElement): number {
    const row = list.querySelector<HTMLElement>(".command-palette-item");
    return row && row.offsetHeight > 0 ? Math.max(1, Math.floor(list.clientHeight / row.offsetHeight) - 1) : 8;
}

/** After a key move: the selected row in view (the first row: the list's top, padding included). */
function keepRowInView(list: HTMLElement, idx: number) {
    if (idx === 0) list.scrollTop = 0;
    else list.querySelector(".command-palette-item.selected")?.scrollIntoView?.({ block: "nearest" });
}

type KeyIntent = { kind: "close" } | { kind: "run"; forceNew: boolean } | { kind: "move"; to: number };

/** What a key does in the palette, or null when it is none of its keys. */
function keyIntent(e: KeyboardEvent, i: number, last: number, list: HTMLElement): KeyIntent | null {
    // While text is being composed (an IME) these keys belong to the composition (R19).
    if (e.isComposing || e.keyCode === 229) return null;
    // The key that opens the palette closes it (R20). The window's own keys are off while it is open.
    if (isKeyFor(e, "view:command-palette")) return { kind: "close" };
    const to = selectionAfterKey(e.key, i, last, rowsPerPage(list));
    if (to != null) return { kind: "move", to };
    return e.key === "Enter" ? { kind: "run", forceNew: e.shiftKey } : null;
}

/** What the palette lists for `query`, and which row is selected (held to the list when it shrinks). */
function usePaletteResults(query: () => string, selectedIdx: () => number) {
    // The agents AgentMux knows, listed after the commands and found by name, seat or state.
    const agents = usePaletteAgents();
    const filtered = createMemo(() => {
        const q = query().trim();
        const all = sortCommands([...commandRegistry.all(), ...agents()].filter((c) => !c.hidden && isCommandAvailable(c)));
        if (!q) return all; // browsing, unfiltered: keep category+label order
        // Ranked while actively searching: the exact command first, then
        // commands containing what was typed (category+label order among
        // equals); fuzzy only for a typo. See command-palette-search.ts.
        return searchCommands(all, q);
    });
    const clampedIdx = createMemo(() => Math.min(selectedIdx(), Math.max(filtered().length - 1, 0)));
    const rows = createMemo(() => paletteRows(filtered(), query().trim() !== ""));
    return { filtered, clampedIdx, rows };
}

/** The listbox: the rows in their groups (each group a screen-reader group, the unnamed first one none). */
function PaletteList(props: {
    ref: (el: HTMLDivElement) => void;
    id: string;
    segments: ReturnType<typeof paletteSegments>;
    selectedIdx: number;
    groupColumn: boolean;
    optionId: (i: number) => string;
    onPick: (row: number, shift: boolean) => void;
}): JSX.Element {
    return (
        <div class="command-palette-list" ref={props.ref} id={props.id} role="listbox" aria-label={t("palette.listLabel")}>
            <For each={props.segments}>
                {(segment) => (
                    <div role={segment.label ? "group" : undefined} aria-label={segment.label}>
                        <For each={segment.rows}>
                            {(row) => (
                                <PaletteRow
                                    row={row}
                                    id={props.optionId(row.index)}
                                    selected={row.index === props.selectedIdx}
                                    groupColumn={props.groupColumn}
                                    onPick={(e) => props.onPick(row.index, e.shiftKey)}
                                />
                            )}
                        </For>
                    </div>
                )}
            </For>
        </div>
    );
}

const CommandPaletteModal = (props: ModalCloseProps): JSX.Element => {
    const [query, setQuery] = createSignal("");
    const [selectedIdx, setSelectedIdx] = createSignal(0);
    let inputRef!: HTMLInputElement;
    let listRef!: HTMLDivElement;
    const { filtered, clampedIdx, rows } = usePaletteResults(query, selectedIdx);
    const listId = `command-palette-list-${createUniqueId()}`;
    const optionId = (i: number) => `${listId}-option-${i}`;
    const segments = createMemo(() => paletteSegments(rows()));

    onMount(() => {
        disableGlobalKeybindings();
        // Modal-v2 will focus the first focusable after Portal mount
        // (the search input). Keep a defensive explicit focus() in case
        // the order of focusable elements ever changes.
        queueMicrotask(() => inputRef?.focus());
    });

    onCleanup(() => {
        enableGlobalKeybindings();
    });

    function close() {
        props.close();
    }

    function executeSelected(forceNew = false) {
        const cmds = filtered();
        const idx = clampedIdx();
        if (cmds.length === 0) return;
        const cmd = cmds[idx];
        close();
        // Execute after close so the palette doesn't interfere
        setTimeout(() => {
            void Promise.resolve(cmd.execute({ forceNew }));
        }, 0);
    }

    // Arrow keys + Enter are palette-specific navigation. ESC and
    // backdrop close are handled by `<Modal>` → `onClose`.
    function handleKeyDown(e: KeyboardEvent) {
        const intent = keyIntent(e, clampedIdx(), filtered().length - 1, listRef);
        if (!intent) return;
        e.preventDefault();
        if (intent.kind === "close") return close();
        if (intent.kind === "run") return executeSelected(intent.forceNew);
        setSelectedIdx(intent.to);
        keepRowInView(listRef, intent.to);
    }

    function handleInput(e: InputEvent) {
        setQuery((e.target as HTMLInputElement).value);
        setSelectedIdx(0);
        listRef.scrollTop = 0; // the first row is selected again: show it
    }

    return (
        <Modal
            open={true}
            onClose={close}
            placement="dropdown"
            backdrop="none"
            panelClass="command-palette-panel"
            ariaLabel="Command palette"
        >
            <div class="command-palette-container" onKeyDown={handleKeyDown}>
                <div class="command-palette-input-row">
                    <SearchIcon />
                    <input
                        ref={inputRef}
                        class="command-palette-input" aria-label="Search commands and agents" data-raw-text-field="the command palette's search line: it owns the arrow, Enter and Escape keys of the list below it, not a form field"
                        type="text"
                        role="combobox"
                        aria-expanded={true}
                        aria-controls={listId}
                        aria-autocomplete="list"
                        aria-activedescendant={filtered().length > 0 ? optionId(clampedIdx()) : undefined}
                        placeholder="Search commands and agents"
                        value={query()}
                        onInput={handleInput}
                        autocomplete="off"
                        spellcheck={false}
                    />
                </div>
                <PaletteList
                    ref={(el) => (listRef = el)}
                    id={listId}
                    segments={segments()}
                    selectedIdx={clampedIdx()}
                    groupColumn={query().trim() === ""}
                    optionId={optionId}
                    onPick={(row, shift) => {
                        setSelectedIdx(row);
                        executeSelected(shift);
                    }}
                />
                <Show when={filtered().length === 0}>
                    <div class="command-palette-empty" role="status">
                        Nothing matches "{query()}"
                    </div>
                </Show>
            </div>
        </Modal>
    );
};

CommandPaletteModal.displayName = "CommandPaletteModal";

export { CommandPaletteModal };
