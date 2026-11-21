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
    /** The group's name, in lower case, on the group's first row only; undefined elsewhere. */
    group?: string;
    /** The first row of a group below the first: a hairline above it. */
    divider: boolean;
}

/** How many recently used commands the unfiltered list shows (R30). */
export const RECENT_SHOWN = 8;
export const RECENT_GROUP = "recently used";
export const OTHER_GROUP = "other commands";

/**
 * `cmds` (the commands to list now: the available ones, or the search's matches in its ranking) with the remembered
 * ones first, in the order they were used (R30). Browsing shows the 8 newest; typing shows every one that matches.
 * A remembered id that is not in `cmds` (gone, or cannot run now) is skipped, never shown greyed out (R31); a command
 * is never listed twice. `recent` is how many rows lead.
 */
export function arrangeRecent(cmds: CommandEntry[], recentIds: string[], typing: boolean): { cmds: CommandEntry[]; recent: number } {
    const byId = new Map(cmds.map((c) => [c.id, c]));
    const lead: CommandEntry[] = [];
    for (const id of recentIds) {
        const cmd = byId.get(id);
        if (cmd && !lead.includes(cmd)) lead.push(cmd);
        if (!typing && lead.length === RECENT_SHOWN) break;
    }
    return { cmds: [...lead, ...cmds.filter((c) => !lead.includes(c))], recent: lead.length };
}

/**
 * The rows to draw for `cmds`, already in the order to show, the first `recent` of them the recently used. Browsing,
 * the recent rows are one group and each run of one category after them is a group. Typing, the list is the search's
 * ranking: with recent matches it is two groups ("recently used", then "other commands"), without any it is one list
 * with no group at all.
 */
export function paletteRows(cmds: CommandEntry[], typing: boolean, recent = 0): PaletteRowModel[] {
    return cmds.map((cmd, i) => {
        const group = groupStartingAt(cmds, i, typing, recent);
        return { cmd, index: i, group, divider: group !== undefined && i > 0 };
    });
}

/** The name of the group that begins at row `i`, if one does. */
function groupStartingAt(cmds: CommandEntry[], i: number, typing: boolean, recent: number): string | undefined {
    if (recent > 0 && i < recent) return i === 0 ? RECENT_GROUP : undefined;
    if (typing) return recent > 0 && i === recent ? OTHER_GROUP : undefined;
    const starts = i === recent || cmds[i - 1].category !== cmds[i].category;
    return starts ? cmds[i].category.toLowerCase() : undefined;
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
