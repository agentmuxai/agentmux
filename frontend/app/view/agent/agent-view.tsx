// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { useAtomValue, useSetAtom } from "jotai";
import { Atom } from "jotai";
import React, { memo, useCallback, useEffect, useRef, useState } from "react";
import type { DocumentNode } from "./types";
import type { AgentViewModel } from "./agent-model";
import type { AgentAtoms } from "./state";
import { getProviderList, type ProviderDefinition } from "./providers";
import { getApi } from "@/app/store/global";
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
            onConnectWithProvider={model.connectWithProvider}
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
    onConnectWithProvider?: (providerId: string, cliPath: string) => Promise<void>;
    onKill?: () => void;
    onRestart?: () => void;
}

const PROVIDER_ICONS: Record<string, string> = {
    claude: "\u2728",   // sparkles
    codex: "\uD83E\uDD16",    // robot
    gemini: "\uD83D\uDC8E",   // gem
};

/**
 * Provider button in the connect screen.
 */
const ProviderButton: React.FC<{
    provider: ProviderDefinition;
    onSelect: (providerId: string) => void;
    installing: boolean;
}> = ({ provider, onSelect, installing }) => {
    return (
        <button
            className="agent-provider-btn"
            onClick={() => onSelect(provider.id)}
            disabled={installing}
        >
            <span className="agent-provider-icon">{PROVIDER_ICONS[provider.id] || "\u26A1"}</span>
            <span className="agent-provider-name">{provider.displayName}</span>
        </button>
    );
};

export const AgentViewInner: React.FC<AgentViewProps> = memo(
    ({
        agentId,
        atoms,
        filteredDocumentAtom,
        documentStatsAtom,
        toggleNodeCollapsed,
        onSendMessage,
        onConnectWithProvider,
        onKill,
        onRestart,
    }) => {
        const document = useAtomValue(filteredDocumentAtom);
        const documentState = useAtomValue(atoms.documentStateAtom);
        const authState = useAtomValue(atoms.authAtom);
        const rawOutput = useAtomValue(atoms.rawOutputAtom);
        const toggleCollapse = useSetAtom(toggleNodeCollapsed);
        const scrollRef = useRef<HTMLDivElement>(null);
        const [installingProvider, setInstallingProvider] = useState<string | null>(null);
        const [installError, setInstallError] = useState<string | null>(null);

        // Auto-scroll to bottom on new nodes or raw output
        useEffect(() => {
            if (scrollRef.current) {
                scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
            }
        }, [document.length, rawOutput]);

        const handleProviderSelect = useCallback(
            async (providerId: string) => {
                if (!onConnectWithProvider) return;
                setInstallError(null);
                setInstallingProvider(providerId);

                try {
                    // Check if CLI is already installed locally
                    let cliPath = await getApi().getCliPath(providerId);

                    if (!cliPath) {
                        // Install it
                        console.log(`[agent] Installing ${providerId} CLI...`);
                        const result = await getApi().installCli(providerId);
                        cliPath = result.cli_path;
                    }

                    setInstallingProvider(null);
                    await onConnectWithProvider(providerId, cliPath);
                } catch (error) {
                    console.error(`[agent] Failed to install/connect ${providerId}:`, error);
                    setInstallingProvider(null);
                    setInstallError(`Failed to install ${providerId}: ${String(error)}`);
                }
            },
            [onConnectWithProvider]
        );

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

        const providers = getProviderList();
        const hasRawOutput = rawOutput.length > 0;
        const hasDocumentNodes = document.length > 0;

        return (
            <div className="agent-view">
                <div className="agent-document" ref={scrollRef}>
                    {isDisconnected && !hasDocumentNodes && !hasRawOutput && (
                        <div className="agent-empty">
                            <div className="agent-connect-header">Connect</div>
                            <div className="agent-provider-buttons">
                                {providers.map((provider) => (
                                    <ProviderButton
                                        key={provider.id}
                                        provider={provider}
                                        onSelect={handleProviderSelect}
                                        installing={installingProvider === provider.id}
                                    />
                                ))}
                            </div>
                            {installingProvider && (
                                <div className="agent-install-status">
                                    Installing {installingProvider}...
                                </div>
                            )}
                            {installError && (
                                <div className="agent-install-error">{installError}</div>
                            )}
                        </div>
                    )}

                    {isConnecting && !hasDocumentNodes && !hasRawOutput && (
                        <div className="agent-empty">
                            <div className="agent-empty-text">Checking authentication...</div>
                        </div>
                    )}

                    {isConnected && !hasDocumentNodes && !hasRawOutput && (
                        <div className="agent-empty">
                            <div className="agent-empty-text">
                                Connected. Waiting for activity...
                            </div>
                        </div>
                    )}

                    {/* Raw output mode */}
                    {hasRawOutput && (
                        <pre className="agent-raw-output">{rawOutput}</pre>
                    )}

                    {/* Structured document mode */}
                    {hasDocumentNodes && document.map(renderNode)}
                </div>

                {isConnected && (
                    <AgentFooter agentId={agentId} onSendMessage={onSendMessage} />
                )}

                {isAwaitingBrowser && (
                    <div className="agent-auth-overlay">
                        <div className="agent-auth-overlay-content">
                            <div className="agent-auth-spinner">&#9203;</div>
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
