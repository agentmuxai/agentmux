// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useBookmarks — owns the bookmark CRUD state for the agent pane.
 *
 * Step 7 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Bookmarks are stored as block meta under "agent:bookmarks" (a
 * Bookmark[] JSON array). The hook provides reactive accessors for
 * the list + the derived nodeId set, and CRUD callbacks that
 * persist via SetMetaCommand.
 *
 * The hook does NOT own the scroll-to-node implementation — that
 * lives in AgentDocumentView (Step 9 useScrollToNode will formalize
 * that). Callers pass a `jumpTo(nodeId)` callback which the hook's
 * `jump` handler invokes.
 *
 * Returns:
 *   - bookmarks         — reactive Bookmark[] from block meta
 *   - bookmarkedNodeIds — Set<string> of nodeIds, O(1) lookup
 *   - visible           — whether the bookmarks panel is shown
 *   - setVisible        — toggle the panel
 *   - add(node)         — toggle a bookmark (add if missing, remove if present)
 *   - remove(id)        — delete by bookmark id
 *   - rename(id, label) — change a bookmark's label
 *   - jump(nodeId)      — call the parent's scroll-to-node function
 */

import { createMemo, createSignal, type Accessor, type Setter } from "solid-js";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import * as WOS from "@/app/store/wos";
import type { Bookmark, DocumentNode } from "../types";

export type LogFn = (tag: string, text: string, level?: "info" | "error" | "warn") => void;

export interface UseBookmarksOptions {
    blockId: string;
    block: Accessor<{ meta?: Record<string, unknown> } | undefined>;
    log: LogFn;
    /**
     * Called when the user clicks a bookmark. The hook doesn't own
     * the scroll target — pass the parent's scrollToNodeFn here.
     * Optional because bookmarks can exist without scrolling support
     * (e.g. during a brief window before the document view mounts).
     */
    jumpTo?: (nodeId: string) => void;
}

export interface UseBookmarks {
    bookmarks: Accessor<Bookmark[]>;
    bookmarkedNodeIds: Accessor<Set<string>>;
    visible: Accessor<boolean>;
    /** Full SolidJS Setter — accepts both `(true)` and `((prev) => !prev)`. */
    setVisible: Setter<boolean>;
    add: (node: DocumentNode) => void;
    remove: (id: string) => void;
    rename: (id: string, label: string) => void;
    jump: (nodeId: string) => void;
}

const META_KEY = "agent:bookmarks";

/** Extract a short plain-text preview from any document node (≤80 chars). */
function nodePreview(node: DocumentNode): string {
    let raw = "";
    switch (node.type) {
        case "markdown":      raw = node.content; break;
        case "user_message":  raw = node.message; break;
        case "tool":          raw = node.summary || node.tool; break;
        case "agent_message": raw = node.summary || node.message; break;
        case "section":       raw = node.title; break;
        case "subagent_link": raw = node.slug || node.subagentId; break;
    }
    return raw.replace(/\s+/g, " ").trim().slice(0, 80);
}

export function useBookmarks(opts: UseBookmarksOptions): UseBookmarks {
    // Reactive read from block meta. Recomputes when the block atom
    // changes (e.g. after SetMetaCommand mutations propagate via WPS).
    const bookmarks = createMemo<Bookmark[]>(() => {
        const raw = opts.block()?.meta?.[META_KEY];
        if (!Array.isArray(raw)) return [];
        return raw as Bookmark[];
    });

    // Derived id set for the renderer's O(1) "is this node bookmarked"
    // checks. Re-derives only when bookmarks() changes.
    const bookmarkedNodeIds = createMemo<Set<string>>(
        () => new Set(bookmarks().map((b) => b.nodeId)),
    );

    const [visible, setVisible] = createSignal(false);

    /** Persist the bookmark array via SetMetaCommand. */
    const save = async (next: Bookmark[]): Promise<void> => {
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", opts.blockId),
            meta: { [META_KEY]: next },
        });
    };

    const add = (node: DocumentNode): void => {
        const current = bookmarks();
        const existingIdx = current.findIndex((b) => b.nodeId === node.id);
        let next: Bookmark[];
        if (existingIdx >= 0) {
            // Toggle: remove an existing bookmark
            next = current.filter((_, i) => i !== existingIdx);
        } else {
            const preview = nodePreview(node);
            const label = preview.slice(0, 60) || node.id;
            const newBookmark: Bookmark = {
                id: crypto.randomUUID(),
                nodeId: node.id,
                createdAt: Date.now(),
                label,
                preview,
            };
            next = [...current, newBookmark];
            // Open the panel on first bookmark so the user sees it
            setVisible(true);
        }
        save(next).catch((err) => {
            opts.log("bookmark", `failed to save: ${err?.message ?? String(err)}`, "warn");
        });
    };

    const remove = (id: string): void => {
        const next = bookmarks().filter((b) => b.id !== id);
        save(next).catch((err) => {
            opts.log("bookmark", `failed to save: ${err?.message ?? String(err)}`, "warn");
        });
    };

    const rename = (id: string, label: string): void => {
        const next = bookmarks().map((b) => (b.id === id ? { ...b, label } : b));
        save(next).catch((err) => {
            opts.log("bookmark", `failed to save: ${err?.message ?? String(err)}`, "warn");
        });
    };

    const jump = (nodeId: string): void => {
        opts.jumpTo?.(nodeId);
    };

    return {
        bookmarks,
        bookmarkedNodeIds,
        visible,
        setVisible,
        add,
        remove,
        rename,
        jump,
    };
}
