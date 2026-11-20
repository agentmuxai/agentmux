// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// One row of the command palette (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md R9 to R14): the icon when the
// command has one, the label, the detail in muted text, and at the right the shortcut as key caps and, on the first
// row of a group, the group's name. A row with no shortcut has nothing at the right.

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { tip } from "@/app/element/tip";
import { Kbd } from "@/app/element/ui";
import { shortcutFor } from "@/app/keybindings";
import { keyCaps, keyCapsWords } from "@/app/keybindings/caps";
import type { PaletteRowModel } from "./palette-groups";

/** A shortcut label as key caps: one cap per key, a gap between the two steps of a chord. */
function ShortcutCaps(props: { label: string }): JSX.Element {
    return (
        <span class="command-palette-item-shortcut">
            <For each={keyCaps(props.label)}>
                {(step) => (
                    <span class="command-palette-item-step">
                        <For each={step}>{(cap) => <Kbd>{cap}</Kbd>}</For>
                    </span>
                )}
            </For>
        </span>
    );
}

/** What a screen reader says for a row: the label, the detail and the shortcut in words (R26). */
function rowName(cmd: PaletteRowModel["cmd"], shortcut: string): string {
    return [cmd.label, cmd.detail, shortcut && keyCapsWords(shortcut)].filter(Boolean).join(", ");
}

export function PaletteRow(props: {
    row: PaletteRowModel;
    /** The option's id: the search field's `aria-activedescendant` names it. */
    id: string;
    selected: boolean;
    /** Browsing: every row keeps room for a group's name, so the shortcuts line up down the list. */
    groupColumn: boolean;
    onPick: (e: MouseEvent) => void;
}): JSX.Element {
    const cmd = () => props.row.cmd;
    // A label the row cuts is the one whose full text the tooltip gives (R14); a whole one needs no tooltip.
    let labelRef!: HTMLSpanElement;
    const [cut, setCut] = createSignal(false);
    onMount(() => {
        const measure = () => setCut(labelRef.scrollWidth > labelRef.clientWidth);
        measure();
        // A window resized with the palette open cuts other labels, and whole ones that were cut.
        if (typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver(measure);
        observer.observe(labelRef);
        onCleanup(() => observer.disconnect());
    });
    return (
        <div
            id={props.id}
            role="option"
            aria-selected={props.selected}
            aria-label={rowName(cmd(), shortcutFor(cmd().id))}
            class="command-palette-item"
            classList={{ selected: props.selected, "group-start": props.row.divider }}
            {...tip(cut() ? cmd().label : undefined)}
            onClick={(e) => props.onPick(e)}
        >
            <Show when={cmd().icon}>
                <i
                    class={`fa-solid fa-${cmd().icon} command-palette-item-icon`}
                    style={cmd().iconColor ? { color: cmd().iconColor } : {}}
                />
            </Show>
            <span class="command-palette-item-text">
                <span ref={labelRef} class="command-palette-item-label">
                    {cmd().label}
                </span>
                <Show when={cmd().detail}>
                    <span class="command-palette-item-detail">{cmd().detail}</span>
                </Show>
            </span>
            <Show when={shortcutFor(cmd().id)}>{(k) => <ShortcutCaps label={k()} />}</Show>
            <Show when={props.groupColumn}>
                <span class="command-palette-item-group">{props.row.group}</span>
            </Show>
        </div>
    );
}
