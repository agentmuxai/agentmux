// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders the styled document as a list of DocumentNodes.
 * Routes each node type to the appropriate block component.
 */

import { createEffect, For, Show, type Accessor, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState } from "../types";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { MarkdownBlock } from "./MarkdownBlock";
import { ToolBlock } from "./ToolBlock";

interface AgentDocumentViewProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
}

export const AgentDocumentView = ({ documentAtom, documentStateAtom }: AgentDocumentViewProps): JSX.Element => {
    const [document] = documentAtom;
    const [documentState, setDocumentState] = documentStateAtom;
    let scrollRef!: HTMLDivElement;
    let autoScroll = true;

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

    // Auto-scroll to bottom when new nodes arrive
    createEffect(() => {
        // Read document length to track changes
        const len = document().length;
        if (autoScroll && scrollRef) {
            scrollRef.scrollTop = scrollRef.scrollHeight;
        }
    });

    // Detect if user scrolled up (disable auto-scroll)
    const handleScroll = () => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;
        autoScroll = scrollHeight - scrollTop - clientHeight < 50;
    };

    return (
        <Show
            when={document().length > 0}
            fallback={
                <div class="agent-document" ref={scrollRef}>
                    <div class="agent-styled-empty">
                        <div class="agent-styled-spinner" />
                        <div class="agent-styled-status-text">Waiting for output...</div>
                    </div>
                </div>
            }
        >
            <div class="agent-document" ref={scrollRef} onScroll={handleScroll}>
                <For each={document()}>
                    {(node) => (
                        <DocumentNodeRenderer
                            node={node}
                            collapsed={documentState().collapsedNodes.has(node.id)}
                            onToggle={() => toggleCollapse(node.id)}
                        />
                    )}
                </For>
            </div>
        </Show>
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Node renderer ────────────────────────────────────────────────────────────

const DocumentNodeRenderer = ({
    node,
    collapsed,
    onToggle,
}: {
    node: DocumentNode;
    collapsed: boolean;
    onToggle: () => void;
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
                    <span class="agent-user-message-icon">{node.summary}</span>
                    <div class="agent-user-message-content">
                        <pre>{node.message}</pre>
                    </div>
                </div>
            );

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
