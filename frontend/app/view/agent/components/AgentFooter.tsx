// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { createSignal, Show, type JSX } from "solid-js";

interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void;
    loading?: boolean;
}

export const AgentFooter = ({ agentId, onSendMessage, loading }: AgentFooterProps): JSX.Element => {
    const [message, setMessage] = createSignal("");

    const handleSend = () => {
        if (!message().trim()) return;
        if (onSendMessage) {
            onSendMessage(message());
            setMessage("");
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
    };

    return (
        <div class="agent-footer">
            <div class="agent-input-container">
                <textarea
                    class="agent-input"
                    placeholder={`Send message to ${agentId}...`}
                    value={message()}
                    onInput={(e) => setMessage((e.target as HTMLTextAreaElement).value)}
                    onKeyDown={handleKeyDown}
                    rows={2}
                />
                <div class="agent-input-hint">
                    <span>Enter to send • Shift+Enter for newline</span>
                    <Show when={loading}>
                        <span class="agent-loading-spinner">
                            <span class="agent-spinner-dot" />
                            loading
                        </span>
                    </Show>
                </div>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
