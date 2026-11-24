// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// One row of the command palette (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md R9 to R14): the icon when the
// command has one, the label, the detail in muted text, and at the right the shortcut as key caps and, on the first
// row of a group, the group's name. A row with no shortcut has nothing at the right.
//
// A row is built once for its command and then updated in place (the list keys its rows by command id), so nothing
// here measures or observes anything: the tooltip a cut label gets is set by `palette-cut-labels.ts` for the whole
// list at once, after the frame that drew it.

import { createMemo, For, Show, type JSX } from "solid-js";
import { isLightThemeActive } from "@/app/block/pane-identity";
import { paneRoleColor } from "@/app/block/pane-color-scheme";
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

/** An icon-font name ("robot", "arrow-right"), as against a glyph drawn as text. */
const isIconName = (icon: string | undefined): boolean => !!icon && /^[a-z0-9-]+$/i.test(icon);

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
    // Looked up once per change of command, not once per use (the name, the caps and the test for them each need it).
    const shortcut = createMemo(() => shortcutFor(cmd().keyOf ?? cmd().id));
    // An agent's row is marked in its own colour: the identity tone the pane's ring and the Swarm row use, so the
    // mark follows the theme. An agent with no colour yet has no mark and a muted icon.
    const identity = () => {
        const hex = cmd().identity;
        return hex ? paneRoleColor(undefined, hex, isLightThemeActive(), "identity") : undefined;
    };
    return (
        <div
            id={props.id}
            role="option"
            aria-selected={props.selected}
            aria-label={rowName(cmd(), shortcut())}
            class="command-palette-item"
            classList={{ selected: props.selected, "group-start": props.row.divider, "has-identity": identity() !== undefined }}
            style={identity() ? { "--palette-identity": identity() } : undefined}
            onClick={(e) => props.onPick(e)}
        >
            <Show when={cmd().icon}>
                <Show
                    when={isIconName(cmd().icon)}
                    fallback={
                        // An agent's provider glyph ("✦") is text, not an icon-font name.
                        <span class="command-palette-item-icon" style={cmd().iconColor ? { color: cmd().iconColor } : {}}>
                            {cmd().icon}
                        </span>
                    }
                >
                    <i
                        class={`fa-solid fa-${cmd().icon} command-palette-item-icon`}
                        style={cmd().iconColor ? { color: cmd().iconColor } : {}}
                    />
                </Show>
            </Show>
            <span class="command-palette-item-text">
                <span class="command-palette-item-label">{cmd().label}</span>
                <Show when={cmd().detail}>
                    <span class="command-palette-item-detail">{cmd().detail}</span>
                </Show>
            </span>
            <Show when={shortcut()}>{(k) => <ShortcutCaps label={k()} />}</Show>
            <Show when={props.groupColumn}>
                <span class="command-palette-item-group">{props.row.group}</span>
            </Show>
        </div>
    );
}
