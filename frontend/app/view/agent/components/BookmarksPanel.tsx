// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BookmarksPanel — collapsible panel listing all bookmarks for the current agent pane.
 *
 * Bookmarks are stored in block meta under "agent:bookmarks" as a JSON array.
 * Clicking a bookmark calls onJump which scrolls the matching node into view.
 */

import { createSignal, For, Show, type Accessor, type JSX } from "solid-js";
import type { Bookmark } from "../types";

export interface BookmarksPanelProps {
    bookmarks: Accessor<Bookmark[]>;
    onJump: (nodeId: string) => void;
    onDelete: (id: string) => void;
    onRename: (id: string, label: string) => void;
}

/**
 * Format a Unix-ms timestamp as a short relative or absolute string.
 */
function formatTimestamp(ms: number): string {
    const now = Date.now();
    const diff = now - ms;
    if (diff < 60_000) return "just now";
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
    return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export const BookmarksPanel = ({
    bookmarks,
    onJump,
    onDelete,
    onRename,
}: BookmarksPanelProps): JSX.Element => {
    const [collapsed, setCollapsed] = createSignal(false);
    // Track which bookmark is being renamed
    const [renamingId, setRenamingId] = createSignal<string | null>(null);
    const [renameValue, setRenameValue] = createSignal("");

    const sorted = () => [...bookmarks()].sort((a, b) => b.createdAt - a.createdAt);

    const startRename = (bm: Bookmark, e: MouseEvent) => {
        e.stopPropagation();
        setRenamingId(bm.id);
        setRenameValue(bm.label);
    };

    const commitRename = (id: string) => {
        const val = renameValue().trim();
        if (val) onRename(id, val);
        setRenamingId(null);
    };

    const handleRenameKeyDown = (id: string, e: KeyboardEvent) => {
        if (e.key === "Enter") commitRename(id);
        if (e.key === "Escape") setRenamingId(null);
    };

    return (
        <div class="agent-bookmarks-panel">
            {/* Panel header */}
            <div
                class="agent-bookmarks-header"
                onClick={() => setCollapsed((v) => !v)}
                title={collapsed() ? "Show bookmarks" : "Hide bookmarks"}
            >
                <span class="agent-bookmarks-icon">&#x1F516;</span>
                <span class="agent-bookmarks-title">Bookmarks</span>
                <span class="agent-bookmarks-count">{bookmarks().length}</span>
                <span class="agent-bookmarks-chevron">{collapsed() ? "\u25BA" : "\u25BC"}</span>
            </div>

            {/* Bookmark list */}
            <Show when={!collapsed()}>
                <Show
                    when={sorted().length > 0}
                    fallback={
                        <div class="agent-bookmarks-empty">No bookmarks yet. Right-click a message to bookmark it.</div>
                    }
                >
                    <div class="agent-bookmarks-list">
                        <For each={sorted()}>
                            {(bm) => (
                                <div
                                    class="agent-bookmark-entry"
                                    onClick={() => onJump(bm.nodeId)}
                                    title="Click to scroll to this message"
                                >
                                    <div class="agent-bookmark-entry-main">
                                        {/* Label — editable on double-click */}
                                        <Show
                                            when={renamingId() === bm.id}
                                            fallback={
                                                <span
                                                    class="agent-bookmark-label"
                                                    onDblClick={(e) => startRename(bm, e)}
                                                    title="Double-click to rename"
                                                >
                                                    {bm.label}
                                                </span>
                                            }
                                        >
                                            <input
                                                class="agent-bookmark-rename-input"
                                                value={renameValue()}
                                                onInput={(e) => setRenameValue(e.currentTarget.value)}
                                                onKeyDown={(e) => handleRenameKeyDown(bm.id, e)}
                                                onBlur={() => commitRename(bm.id)}
                                                onClick={(e) => e.stopPropagation()}
                                                autofocus
                                            />
                                        </Show>

                                        <span class="agent-bookmark-time">{formatTimestamp(bm.createdAt)}</span>

                                        {/* Delete button */}
                                        <button
                                            class="agent-bookmark-delete"
                                            onClick={(e) => { e.stopPropagation(); onDelete(bm.id); }}
                                            title="Remove bookmark"
                                        >
                                            &times;
                                        </button>
                                    </div>

                                    {/* Preview text */}
                                    <div class="agent-bookmark-preview">{bm.preview}</div>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
};

BookmarksPanel.displayName = "BookmarksPanel";
