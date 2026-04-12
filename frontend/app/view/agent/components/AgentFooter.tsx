// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { Show, type JSX } from "solid-js";

interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void;
    loading?: boolean;
}

export const AgentFooter = (props: AgentFooterProps): JSX.Element => {
    // Uncontrolled textarea — DOM owns the value. Reading via ref on send
    // avoids re-rendering the component tree on every keystroke, which was
    // causing severe typing lag when the parent's `loading` signal updated
    // frequently from streaming controller status events.
    let textareaRef: HTMLTextAreaElement | undefined;

    const autoGrow = (el: HTMLTextAreaElement) => {
        el.style.height = "auto";
        el.style.height = el.scrollHeight + "px";
    };

    const handleSend = () => {
        if (!textareaRef) return;
        const message = textareaRef.value;
        if (!message.trim()) return;
        if (props.onSendMessage) {
            props.onSendMessage(message);
            textareaRef.value = "";
            textareaRef.style.height = "auto";
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
    };

    const handleInput = (e: Event) => {
        // Only auto-grow — do NOT update a signal here. Keeping the DOM as
        // the source of truth means keystrokes don't trigger re-renders.
        autoGrow(e.target as HTMLTextAreaElement);
    };

    return (
        <div class="agent-footer">
            <div class="agent-input-container">
                <textarea
                    ref={textareaRef}
                    class="agent-input"
                    placeholder={`Send message to ${props.agentId}...`}
                    onInput={handleInput}
                    onKeyDown={handleKeyDown}
                    rows={1}
                />
                <div class="agent-input-hint">
                    <span>Enter to send • Shift+Enter for newline</span>
                    <Show when={props.loading}>
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
