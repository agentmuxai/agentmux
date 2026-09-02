// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// State for the editor pane's file-tree explorer.
//
// Three reactive bags:
//   - `rootsAtom:    Root[]`         — top-level entries (HOME + drives/mounts)
//   - `expandedAtom: Set<string>`    — which folder paths are currently open
//   - `dataAtom:     Map<string,…>`  — fetched child rows + load phase per path
//
// HOME is the primary root and is auto-expanded on first load; drives/mounts
// are sibling roots, collapsed by default — user can navigate anywhere on
// the system without leaving the tree.
//
// Lazy-load on first expand; subsequent collapse keeps the data cached so
// re-expanding is instant. Refresh re-fetches every currently-expanded path.
// Hidden-file filtering is applied at render time, not at fetch — so toggling
// the eye button is free, no RPC.
//
// Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createSignal, type Accessor } from "solid-js";

export interface Root {
    /** Display name (e.g. "asaf", "C:", "/", "Macintosh HD") */
    name: string;
    /** Absolute path the tree expands under */
    path: string;
    /** Marks the user's HOME root — gets auto-expanded on load */
    isHome: boolean;
}

interface NodeData {
    phase: "loading" | "loaded" | "error";
    entries?: DirEntry[];
    error?: string;
}

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

export class FileTreeModel {
    private _expanded = createSignal<Set<string>>(new Set());
    expandedAtom: Accessor<Set<string>> = this._expanded[0];

    private _data = createSignal<Map<string, NodeData>>(new Map());
    dataAtom: Accessor<Map<string, NodeData>> = this._data[0];

    private _roots = createSignal<Root[]>([]);
    rootsAtom: Accessor<Root[]> = this._roots[0];

    isExpanded(path: string): boolean {
        return this._expanded[0]().has(path);
    }

    getNodeData(path: string): NodeData | undefined {
        return this._data[0]().get(path);
    }

    /**
     * Set the roots (HOME + drives/mounts) and auto-expand HOME.
     * Called once on mount after `GetEditorRootsCommand` returns.
     */
    async setRootsAndLoad(home: string, drives: { name: string; path: string }[]): Promise<void> {
        // Derive HOME's display name from the path's basename.
        const homeName =
            home.split(/[\\/]/).filter(Boolean).pop() ?? home;
        const roots: Root[] = [
            { name: homeName, path: home, isHome: true },
            ...drives
                // Don't double-show the drive that hosts HOME on Windows
                // (e.g. hide C:\ when HOME is C:\Users\…). UNIX `/` is the
                // exception: every HOME starts with `/`, but `/` is the only
                // way to reach /etc, /opt, /mnt etc., so we always keep it.
                .filter((d) => {
                    if (d.path === "/") return true;
                    return !home.toLowerCase().startsWith(d.path.toLowerCase());
                })
                .map((d) => ({ name: d.name, path: d.path, isHome: false })),
        ];
        this._roots[1](roots);

        // Auto-expand HOME (only); drives stay collapsed.
        const next = new Set(this._expanded[0]());
        next.add(home);
        this._expanded[1](next);
        await this.load(home);
    }

    /** Toggle expand/collapse on a folder. Lazy-loads the first time. */
    async toggleExpand(path: string): Promise<void> {
        if (this.isExpanded(path)) {
            const next = new Set(this._expanded[0]());
            next.delete(path);
            this._expanded[1](next);
        } else {
            const next = new Set(this._expanded[0]());
            next.add(path);
            this._expanded[1](next);
            if (!this._data[0]().has(path)) {
                await this.load(path);
            }
        }
    }

    /** Re-fetch every currently-expanded path. Preserves expansion state. */
    async refresh(): Promise<void> {
        const paths = Array.from(this._expanded[0]());
        await Promise.all(paths.map((p) => this.load(p)));
    }

    /** Close every folder. Roots stay rendered (they're the top-level entries). */
    collapseAll(): void {
        this._expanded[1](new Set<string>());
    }

    /** Collapse a single folder without affecting siblings. */
    collapseFolder(path: string): void {
        const next = new Set(this._expanded[0]());
        next.delete(path);
        this._expanded[1](next);
    }

    /** Expand a folder and lazy-load its children if not yet fetched. */
    async expandFolder(path: string): Promise<void> {
        if (this.isExpanded(path)) return;
        const next = new Set(this._expanded[0]());
        next.add(path);
        this._expanded[1](next);
        if (!this._data[0]().has(path)) {
            await this.load(path);
        }
    }

    /** Re-fetch a single path (e.g. after a file-system mutation in that dir). */
    async refreshPath(path: string): Promise<void> {
        await this.load(path);
    }

    /** Optimistically remove an entry from a loaded parent. Used after delete
     *  so the tree updates immediately without a full re-fetch. */
    removeEntryOptimistic(parentPath: string, name: string): void {
        const setData = (updater: (map: Map<string, NodeData>) => void) => {
            const next = new Map(this._data[0]());
            updater(next);
            this._data[1](next);
        };
        setData((map) => {
            const existing = map.get(parentPath);
            if (existing?.entries) {
                map.set(parentPath, {
                    ...existing,
                    entries: existing.entries.filter((e) => e.name !== name),
                });
            }
        });
    }

    /** Optimistically add an entry to a loaded parent. Used after create. */
    addEntryOptimistic(parentPath: string, entry: DirEntry): void {
        const setData = (updater: (map: Map<string, NodeData>) => void) => {
            const next = new Map(this._data[0]());
            updater(next);
            this._data[1](next);
        };
        setData((map) => {
            const existing = map.get(parentPath);
            if (existing?.entries) {
                const entries = [...existing.entries, entry];
                // Re-sort: dirs first, then alpha.
                entries.sort((a, b) => {
                    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
                    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
                });
                map.set(parentPath, { ...existing, entries });
            }
        });
    }

    private async load(path: string): Promise<void> {
        const setData = (updater: (map: Map<string, NodeData>) => void) => {
            const next = new Map(this._data[0]());
            updater(next);
            this._data[1](next);
        };
        setData((map) => {
            map.set(path, { phase: "loading" });
        });
        try {
            const result = await RpcApi.ListEditorDirCommand(TabRpcClient, { path });
            setData((map) => {
                map.set(result.path, { phase: "loaded", entries: result.entries });
                // If the canonical path differs from the input (e.g. symlink
                // resolution), key both so the tree row matches.
                if (result.path !== path) {
                    map.set(path, { phase: "loaded", entries: result.entries });
                }
            });
        } catch (e: any) {
            setData((map) => {
                map.set(path, { phase: "error", error: e?.message ?? String(e) });
            });
        }
    }
}
