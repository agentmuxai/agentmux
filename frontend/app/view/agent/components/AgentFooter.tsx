// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import React, { memo, useCallback, useState } from "react";

interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void;
}

export const AgentFooter: React.FC<AgentFooterProps> = memo(({ agentId, onSendMessage }) => {
    const [message, setMessage] = useState("");

    const handleSend = useCallback(() => {
        if (!message.trim()) return;
        if (onSendMessage) {
            onSendMessage(message);
            setMessage("");
        }
    }, [message, onSendMessage]);

    const handleKeyDown = useCallback(
        (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
            if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                handleSend();
            }
        },
        [handleSend]
    );

    return (
        <div className="agent-footer">
            <div className="agent-input-container">
                <textarea
                    className="agent-input"
                    placeholder={`Send message to ${agentId}...`}
                    value={message}
                    onChange={(e) => setMessage(e.target.value)}
                    onKeyDown={handleKeyDown}
                    rows={2}
                />
                <div className="agent-input-hint">Enter to send • Shift+Enter for newline</div>
            </div>
        </div>
    );
});

AgentFooter.displayName = "AgentFooter";
