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
// The selected row (where the tree's keyboard is) lives here too, so it
// survives hiding and showing the tree; revealPath() opens the folders down
// to a file, for marking the open file.
//
// Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md,
// docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §4

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DirEntry } from "@/app/store/rpc-api";
import { createSignal, type Accessor } from "solid-js";
import { entryNamed, isWindowsPath, joinPath, rootHolding, segmentsBelow } from "./tree-keyboard";

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

export class FileTreeModel {
    private _expanded = createSignal<Set<string>>(new Set());
    expandedAtom: Accessor<Set<string>> = this._expanded[0];

    private _data = createSignal<Map<string, NodeData>>(new Map());
    dataAtom: Accessor<Map<string, NodeData>> = this._data[0];

    private _roots = createSignal<Root[]>([]);
    rootsAtom: Accessor<Root[]> = this._roots[0];

    private _selected = createSignal<string | null>(null);
    /** The selected row's path: where the tree's keyboard is. */
    selectedPath: Accessor<string | null> = this._selected[0];

    /** Loads in flight, so a reveal can wait for one already started. */
    private _loading = new Map<string, Promise<void>>();

    setSelectedPath(path: string | null): void {
        this._selected[1](path);
    }

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
        // Setting the roots can start a reveal, which may be loading HOME already.
        await (this._loading.get(home) ?? this.load(home));
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

    /**
     * Open the folders from the root holding `file` down to it, loading each,
     * and return the file's row path as the tree writes it, or null when no
     * root holds it or a folder on the way doesn't list it. `file` may be
     * written differently from the tree (separators, a drive letter's case).
     */
    async revealPath(file: string): Promise<string | null> {
        const root = rootHolding(
            this._roots[0]().map((r) => r.path),
            file
        );
        if (root == null) return null;
        const ignoreCase = isWindowsPath(root);
        let path = root;
        for (const name of segmentsBelow(root, file)) {
            if (!this.isExpanded(path)) {
                const next = new Set(this._expanded[0]());
                next.add(path);
                this._expanded[1](next);
            }
            await this.ensureLoaded(path);
            const entry = entryNamed(this._data[0]().get(path)?.entries, name, ignoreCase);
            if (!entry) return null;
            path = joinPath(path, entry.name);
        }
        return path;
    }

    private async ensureLoaded(path: string): Promise<void> {
        const inFlight = this._loading.get(path);
        if (inFlight) return inFlight;
        if (this._data[0]().get(path)?.phase === "loaded") return;
        await this.load(path);
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

    private load(path: string): Promise<void> {
        const loading: Promise<void> = this.fetchDir(path).finally(() => {
            if (this._loading.get(path) === loading) this._loading.delete(path);
        });
        this._loading.set(path, loading);
        return loading;
    }

    private async fetchDir(path: string): Promise<void> {
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
        } catch (e) {
            setData((map) => {
                map.set(path, { phase: "error", error: (e as Error)?.message ?? String(e) });
            });
        }
    }
}
