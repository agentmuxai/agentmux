// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's tree view (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.2, "expand in place"): the shown
 * folder's rows, with each expanded folder's rows under it. A row below the first level is named by its path from the
 * shown folder, `src/lib/a.ts`, always with `/`: the name is what selection, focus, rename and the operations key
 * their rows by, so a nested row needs a name no sibling in another folder shares. `pathOf` turns it back into a path.
 */

import type { FsEntry } from "@/types/rpc/FsEntry";

/** One row of the tree: the entry (named by its path from the shown folder), and how to draw it. */
export interface TreeRow {
    entry: FsEntry;
    /** 0 for a row of the shown folder. */
    depth: number;
    /** The name alone, which the row shows. */
    label: string;
    /** A folder can be opened (a link to a folder is shown, but not expanded in place). */
    expandable: boolean;
    expanded: boolean;
    /** Its rows are being listed. */
    loading: boolean;
}

/** The most folders a pane keeps open at once: each one is watched, and srv allows 64 watches to a pane. */
export const MAX_EXPANDED = 40;

/** The folder a nested row is in: `src/lib` for `src/lib/a.ts`, "" at the first level. */
export function parentRel(rel: string): string {
    const i = rel.lastIndexOf("/");
    return i < 0 ? "" : rel.slice(0, i);
}

/** The last segment of a row's name. */
export function labelOf(rel: string): string {
    return rel.slice(rel.lastIndexOf("/") + 1);
}

/** `rel` is `dir`, or inside it (both from the shown folder). */
export function isUnderRel(rel: string, dir: string): boolean {
    return rel === dir || rel.startsWith(`${dir}/`);
}

/**
 * The rows to show, depth first: each folder's entries in the order `sort` gives, an expanded folder's entries right
 * after it. `childrenOf(rel)` is what has been listed of a folder (undefined while it hasn't); `keep` drops the
 * entries not shown (hidden ones).
 */
export function flattenTree(
    top: readonly FsEntry[],
    expanded: ReadonlySet<string>,
    childrenOf: (rel: string) => readonly FsEntry[] | undefined,
    keep: (e: FsEntry) => boolean,
    sort: (list: readonly FsEntry[]) => FsEntry[]
): TreeRow[] {
    const rows: TreeRow[] = [];
    const walk = (list: readonly FsEntry[], prefix: string, depth: number): void => {
        for (const e of sort(list.filter(keep))) {
            const rel = prefix + e.name;
            const expandable = e.is_dir && !e.is_symlink;
            const open = expandable && expanded.has(rel);
            const kids = open ? childrenOf(rel) : undefined;
            rows.push({
                entry: depth === 0 ? e : { ...e, name: rel },
                depth,
                label: e.name,
                expandable,
                expanded: open,
                loading: open && kids === undefined,
            });
            if (kids) walk(kids, `${rel}/`, depth + 1);
        }
    };
    walk(top, "", 0);
    return rows;
}
