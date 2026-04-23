// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import type { SlashCommand } from "../commands/types";
import type { SessionStats, TurnTokens } from "../types";
import { SlashAutocomplete } from "./SlashAutocomplete";

// ── AgentStatusLine ───────────────────────────────────────────────────────────
// Displayed above the control bar. Shows a cycling thinking phrase while the
// agent is processing, then the last phrase converted to past tense + session
// stats when the turn completes.

function pickThinkingPhrase(_exclude?: string): string {
    return "Working";
}

function ingToEd(_phrase: string): string {
    return "Worked";
}

function fmtElapsed(ms: number): string {
    const s = Math.floor(ms / 1000);
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

function fmtTokens(t: TurnTokens): string {
    const fmt = (n: number) => n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
    return `\u2191${fmt(t.input)} \u2193${fmt(t.output)}`;
}

interface AgentStatusLineProps {
    loading?: boolean;
    /** True after Esc → SIGINT, until session_end arrives. Overrides the
     *  cycling "Working…" phrase with a static "Stopping…" label so the
     *  user has immediate acknowledgement that the interrupt was received. */
    stopping?: boolean;
    currentTool?: string | null;
    sessionStats?: SessionStats | null;
    turnTokens?: TurnTokens | null;
    /** Count of OS processes currently tracked for this agent block
     *  (backgrounded bash, dev servers, docker containers, watchers,
     *  etc.). Drives the `⚙ N` badge; click opens the swarm Activity
     *  tab. 0 or undefined hides the badge. */
    processCount?: number;
    /** Fires when the user clicks the process-count badge — typically
     *  opens the swarm pane so they can see what's running. */
    onProcessBadgeClick?: () => void;
}

export const AgentStatusLine = (props: AgentStatusLineProps): JSX.Element => {
    const [phrase, setPhrase] = createSignal(pickThinkingPhrase());
    const [lastPhrase, setLastPhrase] = createSignal(pickThinkingPhrase());
    const [elapsedMs, setElapsedMs] = createSignal(0);

    // Phrase cycling: 30s interval while loading without a tool name.
    createEffect(() => {
        if (!props.loading || props.currentTool) return;
        setPhrase(pickThinkingPhrase());
        const id = setInterval(() => {
            setPhrase((prev) => {
                const next = pickThinkingPhrase(prev);
                setLastPhrase(next);
                return next;
            });
        }, 30000);
        onCleanup(() => clearInterval(id));
    });

    // Elapsed timer: reset to 0 and tick every second while loading.
    createEffect(() => {
        if (!props.loading) return;
        const start = Date.now();
        setElapsedMs(0);
        const id = setInterval(() => setElapsedMs(Date.now() - start), 1000);
        onCleanup(() => clearInterval(id));
    });

    // Seed lastPhrase when loading begins.
    createEffect(() => {
        if (props.loading && !props.currentTool) setLastPhrase(phrase());
    });

    // Reactive derived values used in both branches.
    const statsText = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const parts: string[] = [];
        parts.push(ingToEd(lastPhrase()));
        if (stats.cost_usd != null) parts.push(`$${stats.cost_usd.toFixed(3)}`);
        if (stats.duration_ms != null) {
            const s = Math.round(stats.duration_ms / 1000);
            parts.push(s < 60 ? `${Math.max(1, s)}s` : `${Math.floor(s / 60)}m ${s % 60}s`);
        }
        if (stats.num_turns) {
            parts.push(`${stats.num_turns} ${stats.num_turns === 1 ? "turn" : "turns"}`);
        }
        return parts.join("  \u00b7  ");
    });

    const rightText = createMemo((): string => {
        const right: string[] = [];
        if (props.turnTokens) right.push(fmtTokens(props.turnTokens));
        right.push(fmtElapsed(elapsedMs()));
        return right.join("  \u00b7  ");
    });

    const processBadge = () => (
        <Show when={(props.processCount ?? 0) > 0}>
            <button
                type="button"
                class="agent-process-badge"
                title={`${props.processCount} tracked ${props.processCount === 1 ? "process" : "processes"} spawned by this agent — click to open swarm`}
                onClick={() => props.onProcessBadgeClick?.()}
            >
                <span class="agent-process-badge-icon">⚙</span>
                <span class="agent-process-badge-count">{props.processCount}</span>
            </button>
        </Show>
    );

    return (
        <Show
            when={props.loading}
            fallback={
                <Show
                    when={statsText()}
                    fallback={
                        <span class="agent-status-line">
                            {processBadge()}
                        </span>
                    }
                >
                    <span class="agent-status-line agent-status-line--stats">
                        {statsText()}
                        {processBadge()}
                    </span>
                </Show>
            }
        >
            <span class="agent-status-line agent-status-line--loading">
                <span class="agent-spinner-dot" />
                <span class="agent-status-left">
                    {props.stopping
                        ? "Stopping\u2026"
                        : props.currentTool
                            ? props.currentTool
                            : `${phrase()}\u2026`}
                </span>
                <span class="agent-status-right">
                    {rightText()}
                    {processBadge()}
                </span>
            </span>
        </Show>
    );
};

AgentStatusLine.displayName = "AgentStatusLine";

interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void | Promise<void>;
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
    /**
     * Slash command completions. When the textarea value matches
     * `^/\w*$` (no space), AgentFooter calls this with the prefix
     * (no leading slash) and renders the SlashAutocomplete dropdown.
     * If absent, autocomplete is disabled.
     */
    getCompletions?: (prefix: string) => SlashCommand[];
    /**
     * Show the "Send now" button above the composer. Caller typically
     * passes `turnActive() && (hasText || pendingCount > 0)` so the
     * button appears only when there's something to force through and
     * the agent is currently busy.
     */
    showSendNow?: () => boolean;
    /**
     * Fires when the user clicks "Send now". Caller is expected to
     * `stopAgent()` (SIGINT) and then — if the composer has text —
     * call `onSendMessage` with it. See AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC.md.
     */
    onSendImmediately?: () => void;
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

    // ── Slash autocomplete state ──────────────────────────────────────
    // Tracks the current `/prefix` (without the leading slash) when the
    // textarea matches `^/\w*$`. Null = dropdown hidden. Reading the
    // value via signal lets the dropdown re-render reactively without
    // controlling the textarea.
    const [autocompletePrefix, setAutocompletePrefix] = createSignal<string | null>(null);
    const [autocompleteIndex, setAutocompleteIndex] = createSignal(0);

    const completions = createMemo<SlashCommand[]>(() => {
        const p = autocompletePrefix();
        if (p === null || !props.getCompletions) return [];
        return props.getCompletions(p);
    });

    const updateAutocomplete = (): void => {
        if (!textareaRef) return;
        const val = textareaRef.value;
        // Show the dropdown only when the value starts with `/` AND
        // contains no space — once the user types past the command name
        // they're filling in args, not picking a command.
        if (val.startsWith("/") && !val.includes(" ") && !val.includes("\n")) {
            const prefix = val.slice(1);
            if (autocompletePrefix() !== prefix) {
                setAutocompletePrefix(prefix);
                setAutocompleteIndex(0);
            }
        } else if (autocompletePrefix() !== null) {
            setAutocompletePrefix(null);
        }
    };

    const acceptCompletion = (cmd: SlashCommand): void => {
        if (!textareaRef) return;
        textareaRef.value = `/${cmd.name} `;
        setAutocompletePrefix(null);
        textareaRef.focus();
        props.onTyping?.();
    };

    // RAF debounce for the onTyping callback. Sustained typing in a single
    // frame collapses to one callback; even rapid typing costs ~1 callback
    // per 16ms. Flag is per-component-instance (captured in closure).
    let typingScrollPending = false;
    const handleInput = () => {
        updateAutocomplete();
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
            setAutocompletePrefix(null);
            // Scroll the new user message into view. SolidJS flushes the
            // document signal synchronously before this point, so jumpToBottom
            // will include the just-added node in scrollHeight.
            props.onTyping?.();
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        // Autocomplete keys take precedence when the dropdown is open
        // and has at least one match. Tab/Enter accept the selection;
        // arrows navigate; Esc dismisses without affecting text.
        if (autocompletePrefix() !== null && completions().length > 0) {
            const list = completions();
            if (e.key === "ArrowDown") {
                e.preventDefault();
                setAutocompleteIndex((i) => (i + 1) % list.length);
                return;
            }
            if (e.key === "ArrowUp") {
                e.preventDefault();
                setAutocompleteIndex((i) => (i - 1 + list.length) % list.length);
                return;
            }
            if (e.key === "Tab") {
                // Tab: fill in the command name for further editing (e.g. adding args)
                e.preventDefault();
                const match = list[autocompleteIndex()];
                if (match) acceptCompletion(match);
                return;
            }
            if (e.key === "Enter") {
                // Enter: select and send immediately — no second Enter needed
                e.preventDefault();
                const match = list[autocompleteIndex()];
                if (match) {
                    acceptCompletion(match);
                    handleSend();
                }
                return;
            }
            if (e.key === "Escape") {
                e.preventDefault();
                setAutocompletePrefix(null);
                return;
            }
        }
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
            <Show when={props.showSendNow?.()}>
                <button
                    type="button"
                    class="agent-send-immediately-btn"
                    onClick={() => props.onSendImmediately?.()}
                    title="Stop the current turn and process the queue now"
                >
                    <span class="agent-send-immediately-icon">⏭</span>
                    <span>Send now</span>
                </button>
            </Show>
            <div class="agent-input-container">
                <Show when={autocompletePrefix() !== null && completions().length > 0}>
                    <SlashAutocomplete
                        completions={completions()}
                        selectedIndex={autocompleteIndex()}
                        onHover={(idx) => setAutocompleteIndex(idx)}
                        onSelect={acceptCompletion}
                    />
                </Show>
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
                </div>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
