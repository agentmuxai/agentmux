// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders the styled document as a list of DocumentNodes.
 * Routes each node type to the appropriate block component.
 * When no document nodes exist yet, shows accumulated log lines (terminal-style).
 */

import { createEffect, For, Show, type Accessor, type JSX, onCleanup } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { MarkdownBlock } from "./MarkdownBlock";
import { SubagentLinkBlock } from "./SubagentLinkBlock";
import { ToolBlock } from "./ToolBlock";

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
}

export const AgentDocumentView = ({ documentAtom, documentStateAtom, logLines, authUrl, onSubagentClick, onLoadOlder, loadingOlder }: AgentDocumentViewProps): JSX.Element => {
    const [document] = documentAtom;
    const [documentState, setDocumentState] = documentStateAtom;
    let scrollRef!: HTMLDivElement;
    let autoScroll = true;
    // Guard against concurrent older-history fetches triggered by scroll
    let loadingOlderInFlight = false;

    // Toggle collapsed state for a node
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

    return (
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
                {(node) => (
                    <div class="agent-document-node-wrapper">
                        <DocumentNodeRenderer
                            node={node}
                            collapsed={documentState().collapsedNodes.has(node.id)}
                            onToggle={() => toggleCollapse(node.id)}
                            onSubagentClick={onSubagentClick}
                        />
                    </div>
                )}
            </For>
        </div>
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Node renderer ────────────────────────────────────────────────────────────

const DocumentNodeRenderer = ({
    node,
    collapsed,
    onToggle,
    onSubagentClick,
}: {
    node: DocumentNode;
    collapsed: boolean;
    onToggle: () => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
}): JSX.Element => {
    switch (node.type) {
        case "markdown":
            return <MarkdownBlock node={node} />;

        case "tool":
            return <ToolBlock node={node} collapsed={collapsed} onToggle={onToggle} />;

        case "agent_message":
            return <AgentMessageBlock node={node} collapsed={collapsed} onToggle={onToggle} />;

        case "user_message":
            return (
                <div class="agent-user-message">
                    <div class="agent-user-message-content">
                        <pre>{node.message}</pre>
                    </div>
                </div>
            );

        case "subagent_link":
            return <SubagentLinkBlock node={node} onClick={onSubagentClick ?? (() => {})} />;

        case "section":
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

        default:
            return null;
    }
};

DocumentNodeRenderer.displayName = "DocumentNodeRenderer";
