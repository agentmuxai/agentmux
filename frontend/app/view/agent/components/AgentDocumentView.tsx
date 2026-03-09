// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders the styled document as a list of DocumentNodes.
 * Routes each node type to the appropriate block component.
 */

import React, { memo, useCallback, useEffect, useRef } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import type { PrimitiveAtom } from "jotai";
import type { DocumentNode, DocumentState } from "../types";
import { MarkdownBlock } from "./MarkdownBlock";
import { ToolBlock } from "./ToolBlock";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { TerminalOutputBlock } from "./TerminalOutputBlock";

interface AgentDocumentViewProps {
    documentAtom: PrimitiveAtom<DocumentNode[]>;
    documentStateAtom: PrimitiveAtom<DocumentState>;
}

export const AgentDocumentView: React.FC<AgentDocumentViewProps> = memo(
    ({ documentAtom, documentStateAtom }) => {
        const document = useAtomValue(documentAtom);
        const documentState = useAtomValue(documentStateAtom);
        const setDocumentState = useSetAtom(documentStateAtom);
        const scrollRef = useRef<HTMLDivElement>(null);
        const autoScrollRef = useRef(true);

        // Toggle collapsed state for a node
        const toggleCollapse = useCallback(
            (nodeId: string) => {
                setDocumentState((prev) => {
                    const collapsed = new Set(prev.collapsedNodes);
                    if (collapsed.has(nodeId)) {
                        collapsed.delete(nodeId);
                    } else {
                        collapsed.add(nodeId);
                    }
                    return { ...prev, collapsedNodes: collapsed };
                });
            },
            [setDocumentState]
        );

        // Auto-scroll to bottom when new nodes arrive
        useEffect(() => {
            if (autoScrollRef.current && scrollRef.current) {
                scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
            }
        }, [document.length]);

        // Detect if user scrolled up (disable auto-scroll)
        const handleScroll = useCallback(() => {
            if (!scrollRef.current) return;
            const { scrollTop, scrollHeight, clientHeight } = scrollRef.current;
            autoScrollRef.current = scrollHeight - scrollTop - clientHeight < 50;
        }, []);

        if (document.length === 0) {
            return (
                <div className="agent-document" ref={scrollRef}>
                    <div className="agent-styled-empty">
                        <div className="agent-styled-spinner" />
                        <div className="agent-styled-status-text">Waiting for output...</div>
                    </div>
                </div>
            );
        }

        return (
            <div className="agent-document" ref={scrollRef} onScroll={handleScroll}>
                {document.map((node) => (
                    <DocumentNodeRenderer
                        key={node.id}
                        node={node}
                        collapsed={documentState.collapsedNodes.has(node.id)}
                        onToggle={() => toggleCollapse(node.id)}
                    />
                ))}
            </div>
        );
    }
);

AgentDocumentView.displayName = "AgentDocumentView";

// ── Node renderer ────────────────────────────────────────────────────────────

const DocumentNodeRenderer: React.FC<{
    node: DocumentNode;
    collapsed: boolean;
    onToggle: () => void;
}> = memo(({ node, collapsed, onToggle }) => {
    switch (node.type) {
        case "markdown":
            return <MarkdownBlock node={node} />;

        case "tool":
            return <ToolBlock node={node} collapsed={collapsed} onToggle={onToggle} />;

        case "agent_message":
            return <AgentMessageBlock node={node} collapsed={collapsed} onToggle={onToggle} />;

        case "user_message":
            return (
                <div className="agent-user-message">
                    <span className="agent-user-message-icon">{node.summary}</span>
                    <div className="agent-user-message-content">
                        <pre>{node.message}</pre>
                    </div>
                </div>
            );

        case "terminal_output":
            return <TerminalOutputBlock node={node} />;

        case "section":
            return (
                <div className={`agent-section level-${node.level}`}>
                    {node.level === 1 && <h1>{node.title}</h1>}
                    {node.level === 2 && <h2>{node.title}</h2>}
                    {node.level === 3 && <h3>{node.title}</h3>}
                </div>
            );

        default:
            return null;
    }
});

DocumentNodeRenderer.displayName = "DocumentNodeRenderer";
