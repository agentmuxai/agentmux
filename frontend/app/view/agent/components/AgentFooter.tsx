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
    let textareaRef: HTMLTextAreaElement | undefined;

    const autoGrow = (el: HTMLTextAreaElement) => {
        el.style.height = "auto";
        el.style.height = el.scrollHeight + "px";
    };

    const handleSend = () => {
        if (!message().trim()) return;
        if (onSendMessage) {
            onSendMessage(message());
            setMessage("");
            if (textareaRef) {
                textareaRef.style.height = "auto";
            }
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
    };

    const handleInput = (e: Event) => {
        const el = e.target as HTMLTextAreaElement;
        setMessage(el.value);
        autoGrow(el);
    };

    return (
        <div class="agent-footer">
            <div class="agent-input-container">
                <textarea
                    ref={textareaRef}
                    class="agent-input"
                    placeholder={`Send message to ${agentId}...`}
                    value={message()}
                    onInput={handleInput}
                    onKeyDown={handleKeyDown}
                    rows={1}
                />
                <div class="agent-input-hint">Enter to send • Shift+Enter for newline</div>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
