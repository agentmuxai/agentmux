// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a navigation key does in a folder tree (the WAI-ARIA tree pattern): move over the visible rows, open or close
 * a folder, step into or out of one. Pure, over plain rows, so the Editor's side tree and the Files pane's tree view
 * answer a key the same way (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md section 12, phase 4).
 */

/** A visible row, as the keys need it. */
export interface KeyRow {
    id: string;
    /** A folder that can be opened. */
    expandable: boolean;
    expanded: boolean;
    /** The id of the folder holding it; null at the top level. */
    parent: string | null;
}

/** What a navigation key does: select a row, open or close a folder, or nothing. */
export type TreeMove = { kind: "select"; id: string } | { kind: "expand"; id: string } | { kind: "collapse"; id: string } | { kind: "none" };

export const NAV_KEYS: ReadonlySet<string> = new Set(["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft", "Home", "End", "PageDown", "PageUp"]);

/**
 * Where `key` takes the selection from `selected` (a row's id, or null), or null when `key` isn't a navigation key.
 * `pageSize` is how many rows a screenful holds. With no row selected, every key starts at the first row (End at the
 * last).
 */
export function treeMove(rows: readonly KeyRow[], selected: string | null, key: string, pageSize: number): TreeMove | null {
    if (!NAV_KEYS.has(key)) return null;
    if (rows.length === 0) return { kind: "none" };
    const last = rows.length - 1;
    const at = (i: number): TreeMove => ({ kind: "select", id: rows[Math.max(0, Math.min(last, i))].id });
    const i = selected == null ? -1 : rows.findIndex((r) => r.id === selected);
    if (i < 0) return at(key === "End" ? last : 0);
    const row = rows[i];
    switch (key) {
        case "ArrowDown":
            return at(i + 1);
        case "ArrowUp":
            return at(i - 1);
        case "Home":
            return at(0);
        case "End":
            return at(last);
        case "PageDown":
            return at(i + Math.max(1, pageSize));
        case "PageUp":
            return at(i - Math.max(1, pageSize));
        case "ArrowRight":
            if (!row.expandable) return { kind: "none" };
            if (!row.expanded) return { kind: "expand", id: row.id };
            // Its first child, once there is one (it may still be loading).
            return rows[i + 1]?.parent === row.id ? at(i + 1) : { kind: "none" };
        case "ArrowLeft":
            if (row.expandable && row.expanded) return { kind: "collapse", id: row.id };
            return row.parent != null ? { kind: "select", id: row.parent } : { kind: "none" };
    }
    return { kind: "none" };
}
