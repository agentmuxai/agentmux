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
    // avoids re-rendering the component tree on every keystroke.
    //
    // Auto-resize is handled entirely by CSS (`field-sizing: content` on
    // .agent-input). No `onInput` handler, no `scrollHeight` read — the
    // browser grows the textarea natively as text wraps. A prior version of
    // this file had a JS `autoGrow` helper that did
    //   el.style.height = "auto"; el.style.height = el.scrollHeight + "px";
    // which forced a synchronous layout on every keystroke. In the agent
    // pane (flex column with a large content-visibility:auto document view
    // above), that layout cost ~22ms per keystroke and blocked character
    // paint — see docs/analysis/agent-typing-lag-trace-2026-04-12.md for
    // the full trace analysis.
    let textareaRef: HTMLTextAreaElement | undefined;

    const handleSend = () => {
        if (!textareaRef) return;
        const message = textareaRef.value;
        if (!message.trim()) return;
        if (props.onSendMessage) {
            props.onSendMessage(message);
            textareaRef.value = "";
            // No style reset — browser's field-sizing handles it
            // automatically when the content empties.
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
                    ref={textareaRef}
                    class="agent-input"
                    placeholder={`Send message to ${props.agentId}...`}
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
