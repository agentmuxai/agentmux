// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { Show, type JSX } from "solid-js";

interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void;
    /**
     * Called when the user types in the composer. Used to tell the document
     * view to scroll to the latest content so the composer input is visually
     * anchored to the most recent message. Fires RAF-debounced — one callback
     * per animation frame regardless of how many keystrokes queued up.
     */
    onTyping?: () => void;
    /**
     * Called on Esc when the textarea is empty (or whitespace only). The
     * parent sends SIGINT to the agent CLI process — equivalent to Ctrl+C
     * in a terminal. With text in the textarea, Esc clears instead.
     * See SPEC_AGENT_PANE_FOLLOWUPS item #9.
     */
    onStopAgent?: () => void;
    loading?: boolean;
}

export const AgentFooter = (props: AgentFooterProps): JSX.Element => {
    // Uncontrolled textarea — DOM owns the value. Reading via ref on send
    // avoids re-rendering the component tree on every keystroke.
    //
    // Auto-resize is handled entirely by CSS (`field-sizing: content` on
    // .agent-input). No `scrollHeight` read — the browser grows the textarea
    // natively as text wraps. A prior version of this file had a JS
    // `autoGrow` helper that did
    //   el.style.height = "auto"; el.style.height = el.scrollHeight + "px";
    // which forced a synchronous layout on every keystroke. In the agent
    // pane (flex column with a large content-visibility:auto document view
    // above), that layout cost ~22ms per keystroke and blocked character
    // paint — see docs/analysis/agent-typing-lag-trace-2026-04-12.md.
    //
    // There IS now an onInput handler, but it's tightly scoped: one boolean
    // check + (at most once per frame) a requestAnimationFrame enqueue, no
    // layout reads. The scroll itself happens in the RAF callback via
    // `scrollRef.scrollTo({top: MAX_SAFE_INTEGER})` which lets the browser
    // clamp internally instead of us reading scrollHeight in JS. Target
    // per-keystroke cost: <2ms. See
    // specs/SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md §3.4.
    let textareaRef: HTMLTextAreaElement | undefined;

    // RAF debounce for the onTyping callback. Sustained typing in a single
    // frame collapses to one callback; even rapid typing costs ~1 callback
    // per 16ms. Flag is per-component-instance (captured in closure).
    let typingScrollPending = false;
    const handleInput = () => {
        const cb = props.onTyping;
        if (!cb) return;
        if (typingScrollPending) return;
        typingScrollPending = true;
        requestAnimationFrame(() => {
            typingScrollPending = false;
            cb();
        });
    };

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
            return;
        }
        if (e.key === "Escape") {
            // Esc semantics (SPEC_AGENT_PANE_FOLLOWUPS item #9):
            //  - textarea has text → clear it, stay focused
            //  - textarea is empty → send SIGINT to the agent CLI
            if (!textareaRef) return;
            if (textareaRef.value.trim().length > 0) {
                e.preventDefault();
                textareaRef.value = "";
                // Kick the RAF-debounced typing scroll so the footer's
                // auto-grow collapses and the document stays anchored.
                props.onTyping?.();
            } else {
                e.preventDefault();
                props.onStopAgent?.();
            }
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
                    onInput={handleInput}
                    rows={1}
                />
                <div class="agent-input-hint">
                    <span>Enter to send • Shift+Enter for newline • Esc to clear / stop</span>
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
