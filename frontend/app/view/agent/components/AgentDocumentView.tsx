// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders the styled document as a list of DocumentNodes.
 * Routes each node type to the appropriate block component.
 * When no document nodes exist yet, shows accumulated log lines (terminal-style).
 */

import { createEffect, createSignal, For, Show, type Accessor, type JSX, onCleanup } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { AgentTimeline } from "./AgentTimeline";
import { MarkdownBlock } from "./MarkdownBlock";
import { SubagentLinkBlock } from "./SubagentLinkBlock";
import { ToolBlock } from "./ToolBlock";
import { ContextMenuModel } from "@/app/store/contextmenu";

export interface LogLine {
    tag: string;        // "agent", "cli", "auth", "env", "error", etc.
    text: string;
    level?: "info" | "error" | "warn";
}

interface AgentDocumentViewProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
    logLines: Accessor<LogLine[]>;
    authUrl?: Accessor<string | null>;
    onSubagentClick?: (node: SubagentLinkNode) => void;
    /** Called when the user scrolls near the top — load the previous page of history. */
    onLoadOlder?: () => Promise<void>;
    /** Whether an older-history load is currently in progress. */
    loadingOlder?: Accessor<boolean>;
    /** session:start_ts_ms from block meta — enables the timeline minimap. */
    startTsMs?: Accessor<number | null>;
    /** session:last_activity_ms from block meta — enables the timeline minimap. */
    endTsMs?: Accessor<number | null>;
    /** Set of bookmarked node IDs — drives the bookmarked visual indicator. */
    bookmarkedNodeIds?: Accessor<Set<string>>;
    /** Called when the user bookmarks or un-bookmarks a node via context menu. */
    onBookmark?: (node: DocumentNode) => void;
    /** Expose a scrollToNode function to the parent for jump-to-bookmark support. */
    scrollToNodeRef?: (fn: (nodeId: string) => void) => void;
    /** The node id of the currently highlighted search match (if any). */
    highlightNodeId?: Accessor<string | null>;
}

export const AgentDocumentView = ({ documentAtom, documentStateAtom, logLines, authUrl, onSubagentClick, onLoadOlder, loadingOlder, startTsMs, endTsMs, bookmarkedNodeIds, onBookmark, scrollToNodeRef, highlightNodeId }: AgentDocumentViewProps): JSX.Element => {
    const [document] = documentAtom;
    const [documentState, setDocumentState] = documentStateAtom;
    let scrollRef!: HTMLDivElement;
    let autoScroll = true;
    // Guard against concurrent older-history fetches triggered by scroll
    let loadingOlderInFlight = false;
    // 0..1 fraction of the current scroll position within the document
    const [scrollFraction, setScrollFraction] = createSignal(0);

    // Scroll to a node by its data-node-id attribute.
    // Exposed to the parent via scrollToNodeRef so BookmarksPanel can call it.
    const scrollToNode = (nodeId: string) => {
        const el = scrollRef?.querySelector(`[data-node-id="${nodeId}"]`);
        if (el) {
            el.scrollIntoView({ behavior: "smooth", block: "center" });
            // Disable auto-scroll after a manual jump
            autoScroll = false;
        }
    };

    // Expose scrollToNode to the parent on mount
    if (scrollToNodeRef) scrollToNodeRef(scrollToNode);

    // Toggle collapsed state for a node (agent messages only — tool blocks
    // manage their own expand/collapse via hover + pin).
    const toggleCollapse = (nodeId: string) => {
        setDocumentState((prev) => {
            const collapsed = new Set(prev.collapsedNodes);
            if (collapsed.has(nodeId)) {
                collapsed.delete(nodeId);
            } else {
                collapsed.add(nodeId);
            }
            return { ...prev, collapsedNodes: collapsed };
        });
    };

    // Toggle the "pinned open" state for a tool node. Tool blocks render
    // collapsed by default — hover expands, click pins. See
    // docs/specs/tool-collapse.md.
    const toggleToolPin = (nodeId: string) => {
        setDocumentState((prev) => {
            const pinned = new Set(prev.pinnedToolNodes);
            if (pinned.has(nodeId)) {
                pinned.delete(nodeId);
            } else {
                pinned.add(nodeId);
            }
            return { ...prev, pinnedToolNodes: pinned };
        });
    };

    // Auto-scroll to bottom when new content arrives.
    let scrollRafId: number | null = null;

    const scrollToBottom = () => {
        if (autoScroll && scrollRef) {
            scrollRef.scrollTop = scrollRef.scrollHeight;
        }
    };

    // Scroll when document or log signals change — throttled to one RAF per batch
    createEffect(() => {
        const _docLen = document().length;
        const _logLen = logLines().length;
        if (scrollRafId == null) {
            scrollRafId = requestAnimationFrame(() => {
                scrollRafId = null;
                scrollToBottom();
            });
        }
    });

    onCleanup(() => {
        if (scrollRafId != null) cancelAnimationFrame(scrollRafId);
    });

    // Detect if user scrolled up (disable auto-scroll) and trigger older-history load
    const handleScroll = () => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;
        autoScroll = scrollHeight - scrollTop - clientHeight < 50;

        // Update timeline scroll indicator
        const maxScroll = scrollHeight - clientHeight;
        setScrollFraction(maxScroll > 0 ? Math.min(1, scrollTop / maxScroll) : 0);

        // Trigger older-history load when near the top
        if (
            onLoadOlder &&
            scrollTop < 50 &&
            !loadingOlderInFlight &&
            !(loadingOlder?.())
        ) {
            // Snapshot scroll anchor before content is prepended
            const snapshotScrollHeight = scrollHeight;
            const snapshotScrollTop = scrollTop;
            loadingOlderInFlight = true;
            onLoadOlder().then(() => {
                // Restore scroll position so the user's viewport doesn't jump.
                // Use a RAF so the DOM has updated with the new nodes first.
                requestAnimationFrame(() => {
                    if (scrollRef) {
                        scrollRef.scrollTop =
                            scrollRef.scrollHeight - snapshotScrollHeight + snapshotScrollTop;
                    }
                    loadingOlderInFlight = false;
                });
            }).catch(() => {
                loadingOlderInFlight = false;
            });
        }
    };

    // Called when the user clicks the timeline — scroll the document to that position.
    const handleTimelineJump = (fraction: number) => {
        if (!scrollRef) return;
        const { scrollHeight, clientHeight } = scrollRef;
        const maxScroll = scrollHeight - clientHeight;
        scrollRef.scrollTop = fraction * maxScroll;
        // Disable auto-scroll when the user manually jumps to a position
        autoScroll = fraction >= 0.98;
    };

    return (
        <div class="agent-document-wrapper">
        <div
            class="agent-document"
            ref={scrollRef}
            onScroll={handleScroll}
        >
            {/* Older-history loading indicator — pinned at the very top */}
            <Show when={loadingOlder?.()}>
                <div class="agent-history-loading">Loading older messages...</div>
            </Show>

            {/* Log lines always shown at the top */}
            <Show when={logLines().length > 0}>
                <div class="agent-status-log">
                    <For each={logLines()}>
                        {(line) => (
                            <div
                                class="agent-status-line"
                                classList={{
                                    "agent-status-line--error": line.level === "error",
                                    "agent-status-line--warn": line.level === "warn",
                                }}
                            >
                                <span class="agent-status-tag">[{line.tag}]</span> {line.text}
                            </div>
                        )}
                    </For>
                    <Show when={authUrl?.()}>
                        {(url) => (
                            <div class="agent-auth-url-box">
                                <div class="agent-auth-url-label">Login URL (if browser didn't open):</div>
                                <div class="agent-auth-url-row">
                                    <span class="agent-auth-url-text">{url()}</span>
                                    <button
                                        class="agent-auth-url-copy"
                                        onClick={() => { import("@/util/clipboard").then(c => c.writeText(url())); }}
                                        title="Copy URL"
                                    >
                                        Copy
                                    </button>
                                </div>
                            </div>
                        )}
                    </Show>
                </div>
            </Show>

            {/* Document nodes render below log lines.
                `content-visibility: auto` on the wrapper lets the browser
                skip layout/paint for off-screen nodes — critical for
                long sessions where thousands of DOM elements accumulate. */}
            <For each={document()}>
                {(node) => {
                    const isBookmarked = () => bookmarkedNodeIds?.().has(node.id) ?? false;

                    const handleContextMenu = (e: MouseEvent) => {
                        if (!onBookmark) return;
                        // Don't show bookmark menu on top of text selections — let the parent handle those.
                        const sel = window.getSelection()?.toString();
                        if (sel) return;
                        e.preventDefault();
                        e.stopPropagation();
                        ContextMenuModel.showContextMenu(
                            [
                                {
                                    label: isBookmarked() ? "Remove bookmark" : "Bookmark this message",
                                    click: () => onBookmark(node),
                                },
                            ],
                            e,
                        );
                    };

                    return (
                        <div
                            class="agent-document-node-wrapper"
                            classList={{
                                "agent-node-bookmarked": isBookmarked(),
                                "agent-node-search-match": highlightNodeId?.() === node.id,
                            }}
                            data-node-id={node.id}
                            onContextMenu={handleContextMenu}
                        >
                            {/* Bookmark indicator button — shown on hover */}
                            <Show when={onBookmark != null}>
                                <button
                                    class="agent-bookmark-btn"
                                    classList={{ "agent-bookmark-btn--active": isBookmarked() }}
                                    onClick={(e) => { e.stopPropagation(); onBookmark!(node); }}
                                    title={isBookmarked() ? "Remove bookmark" : "Bookmark this message"}
                                >
                                    {isBookmarked() ? "\uD83D\uDD16" : "\uD83D\uDD16"}
                                </button>
                            </Show>
                            <DocumentNodeRenderer
                                node={node}
                                collapsed={documentState().collapsedNodes.has(node.id)}
                                onToggle={() => toggleCollapse(node.id)}
                                toolPinned={documentState().pinnedToolNodes.has(node.id)}
                                onToggleToolPin={() => toggleToolPin(node.id)}
                                onSubagentClick={onSubagentClick}
                            />
                        </div>
                    );
                }}
            </For>
        </div>
        <Show when={startTsMs != null && endTsMs != null}>
            <AgentTimeline
                document={document}
                startTsMs={startTsMs ?? (() => null)}
                endTsMs={endTsMs ?? (() => null)}
                scrollPosition={scrollFraction}
                onJump={handleTimelineJump}
            />
        </Show>
        </div>
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Node renderer ────────────────────────────────────────────────────────────

// SolidJS reactivity note: props are accessed via `props.X` and NEVER
// destructured in the parameter list. Destructuring captures mount-time
// values and breaks reactivity for any prop that changes without triggering
// the parent to re-create this component. Since the tool pin state
// (`toolPinned`) lives in documentState — separate from the document array
// that <For> keys off — pin toggles do NOT re-create DocumentNodeRenderer,
// so a destructured `toolPinned` would stay stale forever even though the
// child ToolBlock uses `props.pinned` correctly.
//
// See commit adcec38 for the first half of this fix (ToolBlock), and the
// reagent review on PR #346 that caught this upstream companion.

interface DocumentNodeRendererProps {
    node: DocumentNode;
    collapsed: boolean;
    onToggle: () => void;
    toolPinned: boolean;
    onToggleToolPin: () => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
}

const DocumentNodeRenderer = (props: DocumentNodeRendererProps): JSX.Element => {
    switch (props.node.type) {
        case "markdown":
            return <MarkdownBlock node={props.node} />;

        case "tool":
            return (
                <ToolBlock
                    node={props.node}
                    pinned={props.toolPinned}
                    onTogglePin={props.onToggleToolPin}
                />
            );

        case "agent_message":
            return (
                <AgentMessageBlock
                    node={props.node}
                    collapsed={props.collapsed}
                    onToggle={props.onToggle}
                />
            );

        case "user_message": {
            const node = props.node;
            return (
                <div class="agent-user-message">
                    <div class="agent-user-message-content">
                        <pre>{node.message}</pre>
                    </div>
                </div>
            );
        }

        case "subagent_link":
            return (
                <SubagentLinkBlock
                    node={props.node}
                    onClick={props.onSubagentClick ?? (() => {})}
                />
            );

        case "section": {
            const node = props.node;
            return (
                <div class={`agent-section level-${node.level}`}>
                    <Show when={node.level === 1}>
                        <h1>{node.title}</h1>
                    </Show>
                    <Show when={node.level === 2}>
                        <h2>{node.title}</h2>
                    </Show>
                    <Show when={node.level === 3}>
                        <h3>{node.title}</h3>
                    </Show>
                </div>
            );
        }

        default:
            return null;
    }
};

DocumentNodeRenderer.displayName = "DocumentNodeRenderer";
