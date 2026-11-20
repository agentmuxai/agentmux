// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What order the command palette lists, and where its groups begin (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md
// R12, R13). Browsing, the list is in groups, one per category; a group's name is said once, on its first row, and a
// hairline sets it off from the group above. Typing, the list is the search's ranking and says no category at all.

import type { CommandEntry } from "@/app/store/command-registry";

/** The categories in the order the palette lists them; any other follows, by name. */
export const CATEGORY_ORDER: string[] = ["Open", "Split", "Window", "Tab", "Pane", "Agent", "Dev"];

function categoryRank(category: string): number {
    const i = CATEGORY_ORDER.indexOf(category);
    return i === -1 ? CATEGORY_ORDER.length : i;
}

/** Commands in category order, then by name; a category outside the order sorts by its name, so its rows stay together. */
export function sortCommands(cmds: CommandEntry[]): CommandEntry[] {
    return [...cmds].sort((a, b) => {
        const rank = categoryRank(a.category) - categoryRank(b.category);
        if (rank !== 0) return rank;
        const category = a.category.localeCompare(b.category);
        return category !== 0 ? category : a.label.localeCompare(b.label);
    });
}

export interface PaletteRowModel {
    cmd: CommandEntry;
    /** The row's place in the whole list: what selection and the option ids count by. */
    index: number;
    /** The group's name, in lower case, on the group's first row only; undefined elsewhere and while typing. */
    group?: string;
    /** The first row of a group below the first: a hairline above it. */
    divider: boolean;
}

/**
 * The rows to draw for `cmds`, already in the order to show. With `typing`, they are one ranked list and carry no
 * group; browsing, each run of one category is a group.
 */
export function paletteRows(cmds: CommandEntry[], typing: boolean): PaletteRowModel[] {
    return cmds.map((cmd, i) => {
        const starts = !typing && (i === 0 || cmds[i - 1].category !== cmd.category);
        return { cmd, index: i, group: starts ? cmd.category.toLowerCase() : undefined, divider: starts && i > 0 };
    });
}

/** A run of rows the screen reader reads as one named group (R26). Typing, the list is one run with no name. */
export interface PaletteSegment {
    label?: string;
    rows: PaletteRowModel[];
}

/** `rows` cut into their groups: a group begins at the row that names it. */
export function paletteSegments(rows: PaletteRowModel[]): PaletteSegment[] {
    const out: PaletteSegment[] = [];
    for (const row of rows) {
        if (row.group !== undefined || out.length === 0) out.push({ label: row.group, rows: [row] });
        else out[out.length - 1].rows.push(row);
    }
    return out;
}
