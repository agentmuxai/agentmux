// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Command palette modal — opened via Ctrl+P, lists all registered commands.
// Uses the canonical `element/modal` primitive; the palette's specific
// chrome (search row + scroll list) lives in the panel body.

import { commandRegistry, isCommandAvailable, type CommandEntry } from "@/app/store/command-registry";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import { disableGlobalKeybindings, enableGlobalKeybindings } from "@/app/store/keymodel";
import { SearchIcon } from "@/app/element/ui";
import { Modal } from "@/element/modal";
import { searchCommands } from "./command-palette-search";
import { usePaletteAgents } from "./palette-agents";
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import "./command-palette.scss";
import { shortcutFor } from "@/app/keybindings";

const CATEGORY_ORDER: string[] = ["Open", "Split", "Window", "Tab", "Pane", "Agent", "Dev"];

function sortCommands(cmds: CommandEntry[]): CommandEntry[] {
    return [...cmds].sort((a, b) => {
        const ai = CATEGORY_ORDER.indexOf(a.category);
        const bi = CATEGORY_ORDER.indexOf(b.category);
        const ac = ai === -1 ? 99 : ai;
        const bc = bi === -1 ? 99 : bi;
        if (ac !== bc) return ac - bc;
        return a.label.localeCompare(b.label);
    });
}

function PaletteRow(props: {
    cmd: CommandEntry;
    selected: boolean;
    onHover: (e: MouseEvent) => void;
    onPick: (e: MouseEvent) => void;
}): JSX.Element {
    return (
        <div
            class={`command-palette-item${props.selected ? " selected" : ""}`}
            onClick={(e) => props.onPick(e)}
            onMouseMove={(e) => props.onHover(e)}
        >
            {props.cmd.icon && (
                <i
                    class={`fa-solid fa-${props.cmd.icon} command-palette-item-icon`}
                    style={props.cmd.iconColor ? { color: props.cmd.iconColor } : {}}
                />
            )}
            <span class="command-palette-item-text">
                <span class="command-palette-item-label">{props.cmd.label}</span>
                <Show when={props.cmd.detail}>
                    <span class="command-palette-item-detail">{props.cmd.detail}</span>
                </Show>
            </span>
            <span class="command-palette-item-category">{props.cmd.category}</span>
            <Show when={shortcutFor(props.cmd.id)}>{(k) => <kbd class="command-palette-item-shortcut">{k()}</kbd>}</Show>
        </div>
    );
}

/**
 * A pointer over the list selects the row it moves onto, and only a real move does: the list scrolling under a
 * pointer at rest (a key move) puts another row under it, and that must not take the selection. The first
 * event only records where the pointer is.
 */
function createPointerGate(): (e: MouseEvent) => boolean {
    let at: string | null = null;
    return (e) => {
        const here = `${e.clientX},${e.clientY}`;
        const moved = at != null && at !== here;
        at = here;
        return moved;
    };
}

/** After a key move: the selected row in view (the first row: the list's top, padding included). */
function keepRowInView(list: HTMLElement, idx: number) {
    if (idx === 0) list.scrollTop = 0;
    else list.querySelector(".command-palette-item.selected")?.scrollIntoView?.({ block: "nearest" });
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
    return { filtered, clampedIdx };
}

const CommandPaletteModal = (props: ModalCloseProps): JSX.Element => {
    const [query, setQuery] = createSignal("");
    const [selectedIdx, setSelectedIdx] = createSignal(0);
    let inputRef!: HTMLInputElement;
    let listRef!: HTMLDivElement;
    const { filtered, clampedIdx } = usePaletteResults(query, selectedIdx);

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

    const pointerMoved = createPointerGate();

    function moveSelection(next: (i: number) => number) {
        const idx = next(clampedIdx());
        setSelectedIdx(idx);
        keepRowInView(listRef, idx);
    }

    // Arrow keys + Enter are palette-specific navigation. ESC and
    // backdrop close are handled by `<Modal>` → `onClose`.
    function handleKeyDown(e: KeyboardEvent) {
        if (e.key === "ArrowDown") {
            e.preventDefault();
            moveSelection((i) => Math.min(i + 1, filtered().length - 1));
            return;
        }
        if (e.key === "ArrowUp") {
            e.preventDefault();
            moveSelection((i) => Math.max(i - 1, 0));
            return;
        }
        if (e.key === "Enter") {
            e.preventDefault();
            executeSelected(e.shiftKey);
            return;
        }
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
            placement="top"
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
                        placeholder="Search commands and agents..."
                        value={query()}
                        onInput={handleInput}
                        autocomplete="off"
                        spellcheck={false}
                    />
                    <kbd class="command-palette-esc-hint">ESC</kbd>
                </div>
                <div class="command-palette-list" ref={listRef}>
                    <For each={filtered()}>
                        {(cmd, i) => (
                            <PaletteRow
                                cmd={cmd}
                                selected={i() === clampedIdx()}
                                onHover={(e) => {
                                    if (pointerMoved(e)) setSelectedIdx(i());
                                }}
                                onPick={(e) => {
                                    setSelectedIdx(i());
                                    executeSelected(e.shiftKey);
                                }}
                            />
                        )}
                    </For>
                    {filtered().length === 0 && (
                        <div class="command-palette-empty">Nothing matches "{query()}"</div>
                    )}
                </div>
            </div>
        </Modal>
    );
};

CommandPaletteModal.displayName = "CommandPaletteModal";

export { CommandPaletteModal };
