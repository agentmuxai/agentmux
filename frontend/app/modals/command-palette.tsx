// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Command palette modal — opened via Ctrl+P, lists all registered commands.
// Migrated to the shared `element/modal-v2` primitive; the palette's
// specific chrome (search row + scroll list) lives in the panel body.

import { commandRegistry, type CommandEntry } from "@/app/store/command-registry";
import { modalsModel } from "@/app/store/modalmodel";
import { disableGlobalKeybindings, enableGlobalKeybindings } from "@/app/store/keymodel";
import { Modal } from "@/element/modal-v2";
import { createMemo, createSignal, For, onCleanup, onMount, type JSX } from "solid-js";
import "./command-palette.scss";

const CATEGORY_ORDER: string[] = ["Open", "Split", "Window", "Tab", "Pane", "Dev"];

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

const CommandPaletteModal = (): JSX.Element => {
    const [query, setQuery] = createSignal("");
    const [selectedIdx, setSelectedIdx] = createSignal(0);
    let inputRef!: HTMLInputElement;

    const filtered = createMemo(() => {
        const q = query().toLowerCase().trim();
        const all = sortCommands(commandRegistry.all());
        if (!q) return all;
        return all.filter(
            (cmd) =>
                cmd.label.toLowerCase().includes(q) ||
                cmd.id.toLowerCase().includes(q) ||
                cmd.category.toLowerCase().includes(q)
        );
    });

    // Clamp selectedIdx when results change
    const clampedIdx = createMemo(() => {
        const len = filtered().length;
        if (len === 0) return 0;
        return Math.min(selectedIdx(), len - 1);
    });

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
        modalsModel.popModal();
    }

    function executeSelected() {
        const cmds = filtered();
        const idx = clampedIdx();
        if (cmds.length === 0) return;
        const cmd = cmds[idx];
        close();
        // Execute after close so the palette doesn't interfere
        setTimeout(() => {
            void Promise.resolve(cmd.execute());
        }, 0);
    }

    // Arrow keys + Enter are palette-specific navigation. ESC and
    // backdrop close are handled by modal-v2 → `onClose`.
    function handleKeyDown(e: KeyboardEvent) {
        if (e.key === "ArrowDown") {
            e.preventDefault();
            setSelectedIdx((i) => Math.min(i + 1, filtered().length - 1));
            return;
        }
        if (e.key === "ArrowUp") {
            e.preventDefault();
            setSelectedIdx((i) => Math.max(i - 1, 0));
            return;
        }
        if (e.key === "Enter") {
            e.preventDefault();
            executeSelected();
            return;
        }
    }

    function handleInput(e: InputEvent) {
        setQuery((e.target as HTMLInputElement).value);
        setSelectedIdx(0);
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
                    <i class="fa-sharp fa-solid fa-magnifying-glass command-palette-search-icon" />
                    <input
                        ref={inputRef}
                        class="command-palette-input"
                        type="text"
                        placeholder="Search commands..."
                        value={query()}
                        onInput={handleInput}
                        autocomplete="off"
                        spellcheck={false}
                    />
                    <kbd class="command-palette-esc-hint">ESC</kbd>
                </div>
                <div class="command-palette-list">
                    <For each={filtered()}>
                        {(cmd, i) => (
                            <div
                                class={`command-palette-item${i() === clampedIdx() ? " selected" : ""}`}
                                onClick={() => {
                                    setSelectedIdx(i());
                                    executeSelected();
                                }}
                                onMouseEnter={() => setSelectedIdx(i())}
                            >
                                {cmd.icon && (
                                    <i
                                        class={`fa-sharp fa-solid fa-${cmd.icon} command-palette-item-icon`}
                                        style={cmd.iconColor ? { color: cmd.iconColor } : {}}
                                    />
                                )}
                                <span class="command-palette-item-label">{cmd.label}</span>
                                <span class="command-palette-item-category">{cmd.category}</span>
                            </div>
                        )}
                    </For>
                    {filtered().length === 0 && (
                        <div class="command-palette-empty">No commands match "{query()}"</div>
                    )}
                </div>
            </div>
        </Modal>
    );
};

CommandPaletteModal.displayName = "CommandPaletteModal";

export { CommandPaletteModal };
