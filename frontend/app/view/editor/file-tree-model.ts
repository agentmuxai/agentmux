// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// State for the editor pane's file-tree explorer.
//
// Two reactive bags:
//   - `expandedAtom: Set<string>`   — which folder paths are currently open
//   - `dataAtom:     Map<string,…>` — fetched child rows + load phase per path
//
// Lazy-load on first expand; subsequent collapse keeps the data cached so
// re-expanding is instant. Refresh re-fetches every currently-expanded path.
// Hidden-file filtering is applied at render time, not at fetch — so toggling
// the eye button is free, no RPC.
//
// Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createSignal, type Accessor } from "solid-js";

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

    private _root = createSignal<string>("");
    rootAtom: Accessor<string> = this._root[0];

    isExpanded(path: string): boolean {
        return this._expanded[0]().has(path);
    }

    getNodeData(path: string): NodeData | undefined {
        return this._data[0]().get(path);
    }

    /**
     * Set the tree root and immediately expand + load it. Called once on
     * mount after `GetEditorHomeCommand` returns.
     */
    async setRootAndLoad(home: string): Promise<void> {
        this._root[1](home);
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

    /** Close every folder except the root. */
    collapseAll(): void {
        const root = this._root[0]();
        this._expanded[1](root ? new Set<string>([root]) : new Set<string>());
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
