// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The editor file tree's keyboard, as pure functions: the rows the user can
// see, where each tree key moves the selection (the WAI-ARIA tree pattern),
// type-ahead, and where a file sits under the tree's roots, for revealing it.
// file-tree.tsx and file-tree-model.ts apply what these return.
// docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §4, §5.

import type { DirEntry } from "@/app/store/rpc-api";
import { samePath } from "@/app/view/files/files-path";

// Names hidden when show-hidden is OFF.
// Dotfiles match via leading '.' (handled in `isHiddenName`).
const HIDDEN_NAMES = new Set(["node_modules", "Thumbs.db", "$RECYCLE.BIN"]);

export function isHiddenName(name: string): boolean {
    if (name.startsWith(".")) return true;
    return HIDDEN_NAMES.has(name);
}

/**
 * Join a parent path and a child name using the parent's existing separator
 * (Windows-style if the parent contains `\`, else POSIX). The backend
 * canonicalizes paths so this stays consistent across the tree.
 */
export function joinPath(base: string, name: string): string {
    if (!base) return name;
    const sep = base.includes("\\") ? "\\" : "/";
    if (base.endsWith(sep) || base.endsWith("/")) return base + name;
    return base + sep + name;
}

/** What the tree reads to list its rows: FileTreeModel, or a stand-in in tests. */
export interface TreeSource {
    roots: { name: string; path: string }[];
    isExpanded(path: string): boolean;
    /** A folder's loaded entries; undefined until it has loaded. */
    entries(path: string): DirEntry[] | undefined;
}

/** A row the user can see, in display order. */
export interface TreeRow {
    path: string;
    name: string;
    isDir: boolean;
    /** 0 for a root. */
    depth: number;
    /** The folder holding it; null for a root. */
    parent: string | null;
    expanded: boolean;
}

/** A folder's entries as the tree shows them: hidden ones only on request. */
export function shownEntries(entries: DirEntry[] | undefined, showHidden: boolean): DirEntry[] {
    if (!entries) return [];
    return showHidden ? entries : entries.filter((e) => !isHiddenName(e.name));
}

/** Every row the user can see: the roots, and the contents of each open folder. */
export function visibleRows(source: TreeSource, showHidden: boolean): TreeRow[] {
    const rows: TreeRow[] = [];
    const walk = (path: string, name: string, isDir: boolean, depth: number, parent: string | null) => {
        const expanded = isDir && source.isExpanded(path);
        rows.push({ path, name, isDir, depth, parent, expanded });
        if (!expanded) return;
        for (const e of shownEntries(source.entries(path), showHidden)) {
            walk(joinPath(path, e.name), e.name, e.is_dir, depth + 1, path);
        }
    };
    for (const root of source.roots) walk(root.path, root.name, true, 0, null);
    return rows;
}

/** What a navigation key does: select a row, open or close a folder, or nothing. */
export type TreeMove =
    | { kind: "select"; path: string }
    | { kind: "expand"; path: string }
    | { kind: "collapse"; path: string }
    | { kind: "none" };

const NAV_KEYS = new Set(["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft", "Home", "End", "PageDown", "PageUp"]);

/**
 * Where `key` takes the selection from `selected` (a row's path, or null),
 * or null when `key` isn't a navigation key. `pageSize` is how many rows a
 * screenful holds. With no row selected, every key starts at the first row
 * (End at the last).
 */
export function treeMove(rows: TreeRow[], selected: string | null, key: string, pageSize: number): TreeMove | null {
    if (!NAV_KEYS.has(key)) return null;
    if (rows.length === 0) return { kind: "none" };
    const last = rows.length - 1;
    const at = (i: number): TreeMove => ({ kind: "select", path: rows[Math.max(0, Math.min(last, i))].path });
    const i = selected == null ? -1 : rows.findIndex((r) => r.path === selected);
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
            if (!row.isDir) return { kind: "none" };
            if (!row.expanded) return { kind: "expand", path: row.path };
            // Its first child, once there is one (it may still be loading).
            return rows[i + 1]?.parent === row.path ? at(i + 1) : { kind: "none" };
        case "ArrowLeft":
            if (row.isDir && row.expanded) return { kind: "collapse", path: row.path };
            return row.parent != null ? { kind: "select", path: row.parent } : { kind: "none" };
    }
    return { kind: "none" };
}

const TYPEAHEAD_RESET_MS = 500;

/**
 * Type-ahead over the visible rows: keys typed within 500 ms build one
 * prefix, and the next row whose name starts with it is selected. Repeating
 * one letter steps through the rows starting with it. By row, not by name
 * (the Files pane's TypeAhead), since a tree shows the same name in many
 * folders.
 */
export class TreeTypeAhead {
    private buffer = "";
    private last = 0;

    /** The row to select after typing `key`, or null for no match. */
    next(key: string, rows: TreeRow[], selected: string | null, now: number = Date.now()): string | null {
        if (now - this.last > TYPEAHEAD_RESET_MS) this.buffer = "";
        this.last = now;
        this.buffer += key.toLowerCase();
        const cycling = this.buffer.length > 1 && [...this.buffer].every((c) => c === this.buffer[0]);
        const prefix = cycling ? this.buffer[0] : this.buffer;
        const start = selected == null ? -1 : rows.findIndex((r) => r.path === selected);
        // A fresh prefix (or a repeated letter) looks past the selected row;
        // a growing prefix may stay on it.
        const from = this.buffer.length === 1 || cycling ? start + 1 : Math.max(start, 0);
        for (let k = 0; k < rows.length; k++) {
            const row = rows[(from + k) % rows.length];
            if (row.name.toLowerCase().startsWith(prefix)) return row.path;
        }
        return null;
    }

    /** Whether a key typed now continues the prefix (so Space adds to it). */
    active(now: number = Date.now()): boolean {
        return this.buffer !== "" && now - this.last <= TYPEAHEAD_RESET_MS;
    }
}

/** A drive or UNC path, whose case the filesystem ignores. */
export function isWindowsPath(path: string): boolean {
    return /^[A-Za-z]:([\\/]|$)/.test(path) || /^(\\\\|\/\/)[^\\/]/.test(path);
}

/** `/`-separated, no trailing separator (so `/` itself is ""). */
const slashed = (path: string): string => path.replace(/[\\/]+/g, "/").replace(/\/$/, "");

/** The root holding `file`, the deepest when roots nest (HOME under `/`), or null. */
export function rootHolding(roots: string[], file: string): string | null {
    let best: string | null = null;
    for (const root of roots) {
        const fold = (p: string) => (isWindowsPath(root) ? slashed(p).toLowerCase() : slashed(p));
        if (!fold(file).startsWith(fold(root) + "/")) continue;
        if (best == null || slashed(root).length > slashed(best).length) best = root;
    }
    return best;
}

/** The names leading from `root` down to `file`, which is inside it. */
export function segmentsBelow(root: string, file: string): string[] {
    return slashed(file).slice(slashed(root).length).split("/").filter(Boolean);
}

/** The entry called `name`: exactly, or on Windows whatever its case. */
export function entryNamed(entries: DirEntry[] | undefined, name: string, ignoreCase: boolean): DirEntry | undefined {
    if (!entries) return undefined;
    const exact = entries.find((e) => e.name === name);
    if (exact || !ignoreCase) return exact;
    const lower = name.toLowerCase();
    return entries.find((e) => e.name.toLowerCase() === lower);
}

/**
 * Whether a tree row and the open file are the same file. The tree builds its
 * paths from its root (`C:\Users\…` on Windows), while a tab's path is
 * canonical (`c:/Users/…`, editor-pane-state-store's canonicalizePath), so
 * an exact comparison never matched there.
 */
export function isOpenFile(rowPath: string, openPath: string): boolean {
    return openPath !== "" && samePath(rowPath, openPath);
}
