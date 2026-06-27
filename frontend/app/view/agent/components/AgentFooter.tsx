// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { getVoiceSession, type PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { markEnd, markStart } from "@/perf";
import type { AgentViewModel } from "../agent-model";
import type { SlashCommand } from "../commands/types";
import type { SessionStats, TurnTokens } from "../types";
import { SlashAutocomplete } from "./SlashAutocomplete";

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

// ── AgentWorkingRow ───────────────────────────────────────────────────────────
// Rendered immediately below AgentDocumentView in the conversation area.
// Shows spinner + "Working… · Ns" while loading, "✓ Worked · 42s" on completion.
// Returns null when neither loading nor has stats — no idle placeholder.
// Stays visible as a turn delimiter until the user sends the next message.

/** Truncate tool arg to `max` chars, left-truncating file paths to preserve filename. */
function abbreviateArg(s: string, max: number): string {
    if (s.length <= max) return s;
    // For paths (contain / or \), keep the tail (filename) end.
    if (s.includes("/") || s.includes("\\")) {
        return "…" + s.slice(-(max - 1));
    }
    return s.slice(0, max - 1) + "…";
}

interface AgentWorkingRowProps {
    loading: boolean;
    stopping?: boolean;
    currentTool?: string | null;
    /** First significant argument of the active tool (file path, command, etc.).
     *  When set alongside currentTool, shown as "tool · arg" in the left zone. */
    currentToolArg?: string | null;
    sessionStats?: SessionStats | null;
    turnTokens?: TurnTokens | null;
    /** Set when the provider is rate-limited; shows "Rate limited…" in place of thinking phrase. */
    waitingReason?: "rate_limited" | null;
    /** Milliseconds until next retry (from provider Retry-After). Shown when waitingReason is set. */
    retryAfterMs?: number | null;
}

export const AgentWorkingRow = (props: AgentWorkingRowProps): JSX.Element => {
    const [phrase, setPhrase] = createSignal(pickThinkingPhrase());
    const [lastPhrase, setLastPhrase] = createSignal(pickThinkingPhrase());
    const tick = useTick(1000);
    const [loadStartMs, setLoadStartMs] = createSignal<number | null>(null);
    const elapsedMs = createMemo(() => {
        const s = loadStartMs();
        return s != null ? (tick(), Date.now() - s) : 0;
    });

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

    createEffect(() => {
        if (props.loading) {
            setLoadStartMs((prev) => prev ?? Date.now());
        } else {
            setLoadStartMs(null);
        }
    });

    createEffect(() => {
        if (props.loading && !props.currentTool) setLastPhrase(phrase());
    });

    const workedSummary = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const parts: string[] = ["✓ " + ingToEd(lastPhrase())];
        if (stats.duration_ms != null) {
            const s = Math.round(stats.duration_ms / 1000);
            parts.push(s < 60 ? `${Math.max(1, s)}s` : `${Math.floor(s / 60)}m ${s % 60}s`);
        }
        if (stats.input_tokens != null || stats.output_tokens != null) {
            parts.push(fmtTokens({ input: stats.input_tokens ?? 0, output: stats.output_tokens ?? 0 }));
        }
        return parts.join("  ·  ");
    });

    const workedSecondary = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const parts: string[] = [];
        if (stats.cost_usd != null) parts.push(`$${stats.cost_usd.toFixed(3)}`);
        if (stats.num_turns) parts.push(`${stats.num_turns} ${stats.num_turns === 1 ? "turn" : "turns"}`);
        return parts.length ? parts.join("  ·  ") : null;
    });

    const rightText = createMemo((): string => {
        const right: string[] = [];
        if (props.turnTokens) right.push(fmtTokens(props.turnTokens));
        right.push(fmtElapsed(elapsedMs()));
        return right.join("  ·  ");
    });

    return (
        <Show
            when={props.loading}
            fallback={
                <Show when={workedSummary()}>
                    <span class="agent-working-row agent-working-row--worked">
                        <span class="agent-working-row-left">{workedSummary()}</span>
                        <span class="agent-working-row-right">
                            <Show when={workedSecondary()}>
                                <span class="agent-working-row-secondary">{workedSecondary()}</span>
                            </Show>
                        </span>
                    </span>
                </Show>
            }
        >
            <span class="agent-working-row agent-working-row--loading">
                <span class="agent-spinner-dot" />
                <span class="agent-working-row-left">
                    {props.stopping
                        ? "Stopping…"
                        : props.waitingReason === "rate_limited"
                            ? props.retryAfterMs != null
                                ? `Rate limited — retrying in ${Math.ceil(props.retryAfterMs / 1000)}s`
                                : "Rate limited — retrying…"
                        : props.currentTool
                            ? props.currentToolArg
                                ? `${props.currentTool}  ·  ${abbreviateArg(props.currentToolArg, 40)}`
                                : props.currentTool
                            : `${phrase()}…`}
                </span>
                <span class="agent-working-row-right">{rightText()}</span>
            </span>
        </Show>
    );
};

AgentWorkingRow.displayName = "AgentWorkingRow";

interface AgentFooterProps {
    /**
     * Display name for the composer placeholder ("Send message to …").
     * The caller derives this from `block().meta.agentName ?? agentId`
     * so the placeholder shows the human-readable name (e.g. "Claude")
     * instead of the hex agent UUID. Same fallback chain used elsewhere
     * in `agent-view.tsx` (see the log() call on line 380).
     */
    agentName: string;
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
     * Recall the most-recently queued ("send now") message — invoked on
     * ArrowUp when the composer is empty (Claude-Code-CLI un-queue gesture).
     * Returns the recalled message text (and removes it from the queue), or
     * null when nothing is queued. The composer restores the text for editing.
     */
    onRecallLatestQueued?: () => { text: string } | null;
    /**
     * Slash command completions. When the textarea value matches
     * `^/\w*$` (no space), AgentFooter calls this with the prefix
     * (no leading slash) and renders the SlashAutocomplete dropdown.
     * If absent, autocomplete is disabled.
     */
    getCompletions?: (prefix: string) => SlashCommand[];
    /**
     * AgentViewModel — used to register a textarea-backed
     * PaneVoiceHandle so the frame-header mic button can write
     * transcripts into the composer. Optional so tests / older
     * callers that haven't been updated still type-check.
     */
    viewModel?: AgentViewModel;
}

/**
 * Returns whether the textarea caret is on the first and/or last *visual* line.
 *
 * `<textarea>` wraps text both at explicit `\n` characters and at soft-wrap
 * word-boundaries determined by CSS layout. The simple `includes("\n")` test
 * cannot detect soft-wrap line boundaries — a long single-paragraph message
 * with no `\n` can span many visual rows but always looks like "one line" to a
 * character scan. This function uses a mirror div that replicates the textarea's
 * CSS so the browser word-wraps identically, then reads the zero-width-space
 * marker's `offsetTop` to get the actual visual row Y.
 *
 * Fast-path: when a physical `\n` exists before (or after) the cursor, the
 * answer is known from character data alone — no DOM measurement needed.
 */
function caretVisualEdge(ta: HTMLTextAreaElement): { first: boolean; last: boolean } {
    const pos = ta.selectionStart;
    const val = ta.value;
    const needsFirst = !val.slice(0, pos).includes("\n");
    const needsLast  = !val.slice(pos).includes("\n");
    if (!needsFirst && !needsLast) return { first: false, last: false };

    const cs = window.getComputedStyle(ta); // perf:allow-layout-read — mirror-div caret detection; runs only on ArrowUp/Down when no physical \n exists, not on every keystroke
    const baseCss =
        "position:absolute;visibility:hidden;overflow:hidden;top:-9999px;left:-9999px;" +
        `width:${ta.clientWidth}px;white-space:pre-wrap;word-wrap:break-word;` + // perf:allow-layout-read — same as above; synchronous read required for mirror-div width match
        `font:${cs.font};` +
        `padding:${cs.paddingTop} ${cs.paddingRight} ${cs.paddingBottom} ${cs.paddingLeft};` +
        `box-sizing:${cs.boxSizing};`;

    const measureY = (text: string): number => {
        const div = document.createElement("div");
        div.style.cssText = baseCss;
        div.textContent = text;
        const span = document.createElement("span");
        span.textContent = "​"; // zero-width space — marks the caret position
        div.appendChild(span);
        document.body.appendChild(div);
        const y = span.offsetTop; // perf:allow-layout-read — reads detached mirror div; synchronous result needed to decide ArrowUp/Down behavior
        document.body.removeChild(div);
        return y;
    };

    const caretY = measureY(val.slice(0, pos));
    return {
        first: needsFirst && caretY <= measureY(""),   // matches baseline (first visual row)
        last:  needsLast  && caretY >= measureY(val),  // matches Y at end of text
    };
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

    // ── Sent-message history (shell-style ArrowUp / ArrowDown recall) ──
    // Per-pane, in-memory, capped. The component body runs once (SolidJS), so
    // these closure `let`s persist for the footer's lifetime — same pattern as
    // the voice/typing flags below. `histPos === sentHistory.length` means
    // "not navigating — showing the live draft"; lower values point at a prior
    // sent message. `histDraft` stashes the in-progress text while navigating
    // so ArrowDown past the newest entry restores it.
    const HISTORY_MAX = 200;
    let sentHistory: string[] = [];
    let histPos = 0;
    let histDraft = "";

    // ── Slash autocomplete state ──────────────────────────────────────
    // Tracks the current `/prefix` (without the leading slash) when the
    // textarea matches `^/\w*$`. Null = dropdown hidden. Reading the
    // value via signal lets the dropdown re-render reactively without
    // controlling the textarea.
    const [autocompletePrefix, setAutocompletePrefix] = createSignal<string | null>(null);
    const [autocompleteIndex, setAutocompleteIndex] = createSignal(0);
    const [isBangCmd, setIsBangCmd] = createSignal(false);

    const completions = createMemo<SlashCommand[]>(() => {
        const p = autocompletePrefix();
        if (p === null || !props.getCompletions) return [];
        return props.getCompletions(p);
    });

    // Composer ghost text. While voice is listening AND this pane owns the
    // session, switch to "Speak to <agent>…" so it's obvious transcription is
    // live and routed here (the mic button also shows a red recording ring).
    // Otherwise the normal "Send message to <agent>…" prompt. Reactive: reads
    // the voice singleton's SignalAtoms so it flips the instant the mic toggles
    // or retargets to another pane.
    const voice = getVoiceSession();
    const placeholder = createMemo(() => {
        const vm = props.viewModel;
        const listeningHere =
            !!vm && voice.isListening() && voice.currentTargetId() === vm.blockId;
        return listeningHere
            ? `Speak to ${props.agentName}...`
            : `Send message to ${props.agentName}...`;
    });

    // IME composition state — true between compositionstart and compositionend.
    // CJK / Vietnamese / any IME user types into a composition buffer; intermediate
    // values fire `input` events but should NOT trigger slash-autocomplete matching
    // (the prefix is unfinished) and Enter must NOT submit (it's confirming a
    // candidate). Tracked as a plain `let` instead of a signal — read synchronously
    // in handlers, never drives JSX. See SPEC_INPUT_RESPONSIVENESS §6.2.
    let composingRef = false;

    const updateAutocomplete = (): void => {
        if (!textareaRef) return;
        if (composingRef) return;
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
        setIsBangCmd(false);
        textareaRef.focus();
        props.onTyping?.();
    };

    // Replace the composer's content programmatically (history recall, queued
    // un-send) and park the caret at the end. Mirrors the autocomplete/voice
    // writers: sets the uncontrolled value directly, then refreshes autocomplete
    // + the typing-scroll. Deliberately does NOT dispatch an `input` event, so
    // it never trips the history-cursor reset in handleInput.
    const setComposerValue = (text: string): void => {
        if (!textareaRef) return;
        textareaRef.value = text;
        textareaRef.setSelectionRange(text.length, text.length);
        setIsBangCmd(text.startsWith("!"));
        updateAutocomplete();
        props.onTyping?.();
    };

    // RAF debounce for the onTyping callback. Sustained typing in a single
    // frame collapses to one callback; even rapid typing costs ~1 callback
    // per 16ms. Flag is per-component-instance (captured in closure).
    let typingScrollPending = false;
    const handleInput = () => {
        // A manual edit exits history navigation — the next ArrowUp starts
        // again from the newest sent message. (Programmatic recall writes the
        // value without dispatching `input`, so it doesn't reach here.)
        histPos = sentHistory.length;
        // Perf marks per SPEC_INPUT_RESPONSIVENESS §7.1. Target: handler
        // body P95 < 5 ms. The mark span ends BEFORE the RAF enqueue so
        // we measure only the synchronous handler cost.
        markStart("agent-keystroke");
        updateAutocomplete();
        setIsBangCmd(textareaRef?.value.startsWith("!") ?? false);
        const cb = props.onTyping;
        if (!cb) {
            markEnd("agent-keystroke", "done");
            return;
        }
        if (typingScrollPending) {
            markEnd("agent-keystroke", "coalesced");
            return;
        }
        typingScrollPending = true;
        markStart("agent-input-raf");
        requestAnimationFrame(() => {
            typingScrollPending = false;
            markEnd("agent-input-raf", "fired");
            markStart("agent-input-raf-cb");
            cb();
            markEnd("agent-input-raf-cb", "done");
        });
        markEnd("agent-keystroke", "scheduled");
    };

    // ── Voice input handle ───────────────────────────────────────────
    // Registers a textarea-backed PaneVoiceHandle on the AgentViewModel
    // so the frame-header mic button can write voice transcripts into
    // the composer. `baseValue` is the prefix the user typed (or the
    // last appended-final state) — interim updates render *after* it
    // without committing, final updates fold in and become the new
    // base. Cleared on unmount.
    onMount(() => {
        const vm = props.viewModel;
        if (!vm) return;
        // Two pieces of state interleave with the user's manual typing:
        //   * baseValue — the textarea content as of the last user-or-
        //     final-voice event. Interim updates render AFTER it but
        //     don't promote into it.
        //   * lastVoiceWrite — the exact string we last wrote to the
        //     textarea. If ta.value diverges from this between events,
        //     the user typed something manually and we re-snapshot
        //     baseValue from ta.value so the next voice event preserves
        //     what they added.
        //
        // Earlier guard (`!startsWith(baseValue)`) never fired for the
        // common case where the user APPENDED to existing content —
        // their additions were silently clobbered by the next voice
        // append. (reagent P1 on PR #930.)
        let baseValue = textareaRef?.value ?? "";
        let lastVoiceWrite = baseValue;
        const handle: PaneVoiceHandle = {
            appendFinal: (text) => {
                const ta = textareaRef;
                if (!ta) return;
                if (ta.value !== lastVoiceWrite) baseValue = ta.value;
                const next = baseValue + text + " ";
                ta.value = next;
                ta.dispatchEvent(new Event("input", { bubbles: true }));
                baseValue = next;
                lastVoiceWrite = next;
            },
            setInterim: (text) => {
                const ta = textareaRef;
                if (!ta) return;
                if (ta.value !== lastVoiceWrite) baseValue = ta.value;
                const next = baseValue + text;
                ta.value = next;
                ta.dispatchEvent(new Event("input", { bubbles: true }));
                lastVoiceWrite = next;
            },
        };
        vm.voiceTargetRef.current = handle;
        onCleanup(() => {
            if (vm.voiceTargetRef.current === handle) {
                vm.voiceTargetRef.current = null;
            }
        });
    });

    const handleSend = () => {
        if (!textareaRef) return;
        const message = textareaRef.value;
        if (!message.trim()) return;
        if (props.onSendMessage) {
            // agent-submit span per SPEC_INPUT_RESPONSIVENESS §7.1. Includes
            // the synchronous onSendMessage cost (WS send, slice dispatch).
            markStart("agent-submit");
            props.onSendMessage(message);
            // Record in shell-style history (skip a consecutive duplicate) and
            // reset the navigation cursor back to the live (now empty) draft.
            if (message !== sentHistory[sentHistory.length - 1]) {
                sentHistory.push(message);
                if (sentHistory.length > HISTORY_MAX) sentHistory.shift();
            }
            histPos = sentHistory.length;
            histDraft = "";
            textareaRef.value = "";
            setAutocompletePrefix(null);
            setIsBangCmd(false);
            // Scroll the new user message into view. SolidJS flushes the
            // document signal synchronously before this point, so jumpToBottom
            // will include the just-added node in scrollHeight.
            props.onTyping?.();
            markEnd("agent-submit", "sent");
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        // IME composition guard: when a CJK / Vietnamese / etc. IME is
        // converting (between compositionstart and compositionend),
        // Enter/Tab/Arrows/Esc confirm or navigate composition candidates
        // — the IME, not our handler, owns them. `keyCode === 229` is
        // Safari's quirk where it emits a synthetic keydown for IME
        // events without setting `isComposing`. Both checks are
        // load-bearing. See SPEC_INPUT_RESPONSIVENESS §6.2.
        if (e.isComposing || e.keyCode === 229) return;
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
        // ── ArrowUp / ArrowDown: queued-message recall + sent-message history ──
        // On an EMPTY composer, ArrowUp first un-queues the most-recently queued
        // ("send now") held message — the Claude-Code-CLI gesture (unsent, so a
        // true un-send). When nothing is queued, ArrowUp walks back through
        // previously SENT messages (shell-style history) and ArrowDown walks
        // forward toward the live draft. The caret-on-first/last-line guards let
        // multiline editing of a recalled message still move line-by-line, and
        // only cross into history at the top/bottom edge.
        if (textareaRef && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
            const empty = textareaRef.value.length === 0;
            const navigating = histPos < sentHistory.length;
            const { first: caretOnFirstLine, last: caretOnLastLine } = caretVisualEdge(textareaRef);

            // Empty composer: ArrowUp un-queues a held message before history.
            if (e.key === "ArrowUp" && empty) {
                const recalled = props.onRecallLatestQueued?.();
                if (recalled) {
                    e.preventDefault();
                    setComposerValue(recalled.text);
                    return;
                }
            }

            // Older: whenever the caret is on the first line (so we don't fight
            // multiline cursor movement). Covers an empty composer, a partially
            // typed draft (stashed into histDraft, restorable with ArrowDown),
            // and continuing further back while already navigating.
            if (e.key === "ArrowUp" && histPos > 0 && caretOnFirstLine) {
                if (!navigating) histDraft = textareaRef.value; // stash the live draft
                histPos--;
                e.preventDefault();
                setComposerValue(sentHistory[histPos]);
                return;
            }

            // Newer: only while navigating, caret on the last line. Past the
            // newest entry, restore the stashed draft.
            if (e.key === "ArrowDown" && navigating && caretOnLastLine) {
                histPos++;
                e.preventDefault();
                setComposerValue(histPos >= sentHistory.length ? histDraft : sentHistory[histPos]);
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
                setIsBangCmd(false);
                // Clearing exits history navigation — park the cursor at the
                // newest message so the next ArrowUp doesn't skip an entry.
                // (The clear doesn't dispatch `input`, so handleInput's reset
                // wouldn't otherwise fire.)
                histPos = sentHistory.length;
                histDraft = "";
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
            {/* "Send now" now renders inside PendingMessagesPanel (right
                of the queue header) so it sits next to the messages it
                accelerates, not above the composer. */}
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
                    class={`agent-input${isBangCmd() ? " bang-command" : ""}`}
                    placeholder={placeholder()}
                    onKeyDown={handleKeyDown}
                    onInput={handleInput}
                    onCompositionStart={() => {
                        composingRef = true;
                    }}
                    onCompositionEnd={() => {
                        composingRef = false;
                        // The textarea value updated during the composition
                        // but we skipped slash matching. Run it now so the
                        // dropdown reflects the committed text.
                        updateAutocomplete();
                    }}
                    rows={1}
                />
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
