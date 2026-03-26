// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { createSignal, type JSX } from "solid-js";

interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void;
}

export const AgentFooter = ({ agentId, onSendMessage }: AgentFooterProps): JSX.Element => {
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
                <div class="agent-input-hint">Enter to send • Shift+Enter for newline</div>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
