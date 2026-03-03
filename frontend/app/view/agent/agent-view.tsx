// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { useAtomValue, useSetAtom } from "jotai";
import { Atom } from "jotai";
import React, { memo, useCallback, useEffect, useRef } from "react";
import type { DocumentNode } from "./types";
import type { AgentViewModel } from "./agent-model";
import type { AgentAtoms } from "./state";
import { MarkdownBlock } from "./components/MarkdownBlock";
import { ToolBlock } from "./components/ToolBlock";
import { AgentMessageBlock } from "./components/AgentMessageBlock";
import { AgentFooter } from "./components/AgentFooter";
import "./agent-view.scss";

/**
 * Top-level wrapper: just passes model methods into the inner view.
 */
export const AgentViewWrapper: React.FC<ViewComponentProps<AgentViewModel>> = memo(({ model }) => {
    return (
        <AgentViewInner
            agentId={model.agentIdValue}
            atoms={model.atoms}
            filteredDocumentAtom={model.filteredDocumentAtom}
            documentStatsAtom={model.documentStatsAtom}
            toggleNodeCollapsed={model.toggleNodeCollapsed}
            onSendMessage={model.sendMessage}
            onConnect={model.connect}
            onKill={model.killAgent}
            onRestart={model.restartAgent}
        />
    );
});

AgentViewWrapper.displayName = "AgentViewWrapper";

interface AgentViewProps {
    agentId: string;
    atoms: AgentAtoms;
    filteredDocumentAtom: Atom<DocumentNode[]>;
    documentStatsAtom: Atom<any>;
    toggleNodeCollapsed: any;
    onSendMessage?: (message: string) => void;
    onConnect?: () => void;
    onKill?: () => void;
    onRestart?: () => void;
}

export const AgentViewInner: React.FC<AgentViewProps> = memo(
    ({
        agentId,
        atoms,
        filteredDocumentAtom,
        documentStatsAtom,
        toggleNodeCollapsed,
        onSendMessage,
        onConnect,
        onKill,
        onRestart,
    }) => {
        const document = useAtomValue(filteredDocumentAtom);
        const documentState = useAtomValue(atoms.documentStateAtom);
        const authState = useAtomValue(atoms.authAtom);
        const toggleCollapse = useSetAtom(toggleNodeCollapsed);
        const scrollRef = useRef<HTMLDivElement>(null);

        // Auto-scroll to bottom on new nodes
        useEffect(() => {
            if (scrollRef.current) {
                scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
            }
        }, [document.length]);

        const renderNode = useCallback(
            (node: DocumentNode) => {
                const isCollapsed = documentState.collapsedNodes.has(node.id);

                switch (node.type) {
                    case "markdown":
                        return <MarkdownBlock key={node.id} node={node} />;

                    case "tool":
                        return (
                            <ToolBlock
                                key={node.id}
                                node={node}
                                collapsed={isCollapsed || node.collapsed}
                                onToggle={() => toggleCollapse(node.id)}
                            />
                        );

                    case "agent_message":
                        return (
                            <AgentMessageBlock
                                key={node.id}
                                node={node}
                                collapsed={isCollapsed || node.collapsed}
                                onToggle={() => toggleCollapse(node.id)}
                            />
                        );

                    case "user_message":
                        return (
                            <div key={node.id} className="agent-user-message">
                                <div className="agent-user-message-content">
                                    <pre>{node.message}</pre>
                                </div>
                            </div>
                        );

                    default:
                        return null;
                }
            },
            [documentState.collapsedNodes, toggleCollapse]
        );

        const isDisconnected = authState.status === "disconnected";
        const isConnecting = authState.status === "connecting";
        const isAwaitingBrowser = authState.status === "awaiting_browser";
        const isConnected = authState.status === "connected";

        return (
            <div className="agent-view">
                <div className="agent-document" ref={scrollRef}>
                    {document.length === 0 ? (
                        <div className="agent-empty">
                            {isDisconnected && (
                                <>
                                    <div className="agent-empty-text">Claude Code</div>
                                    <button className="agent-connect-btn" onClick={onConnect}>
                                        Connect
                                    </button>
                                </>
                            )}
                            {isConnecting && (
                                <div className="agent-empty-text">Checking authentication...</div>
                            )}
                            {isConnected && (
                                <div className="agent-empty-text">
                                    Connected. Waiting for activity...
                                </div>
                            )}
                        </div>
                    ) : (
                        document.map(renderNode)
                    )}
                </div>

                {isConnected && (
                    <AgentFooter agentId={agentId} onSendMessage={onSendMessage} />
                )}

                {isAwaitingBrowser && (
                    <div className="agent-auth-overlay">
                        <div className="agent-auth-overlay-content">
                            <div className="agent-auth-spinner">⏳</div>
                            <div className="agent-auth-title">Waiting for authorization</div>
                            <div className="agent-auth-subtitle">Complete sign-in in your browser</div>
                        </div>
                    </div>
                )}
            </div>
        );
    }
);

AgentViewInner.displayName = "AgentViewInner";

export const AgentView = AgentViewWrapper;
