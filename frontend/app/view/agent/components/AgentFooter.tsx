// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */

import { Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { getVoiceSession, type PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { markEnd, markStart } from "@/perf";
import { makeORef } from "@/app/store/wos";
import { atoms } from "@/app/store/global";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { formatCompactNumber } from "@/util/format-count";
import { formatElapsedCompact } from "@/util/format-time";
import { abbreviateText } from "@/util/format-text";
import { MicButton } from "@/app/element/MicButton";
import type { AgentViewModel } from "../agent-model";
import type { SlashCommand } from "../commands/types";
import type { SessionStats, TurnTokens } from "../types";
import { formatPhaseLabel, type LaunchPhase } from "../flows/launch-phase";
import { SlashAutocomplete } from "./SlashAutocomplete";

function pickThinkingPhrase(_exclude?: string): string {
    return "Working";
}

function ingToEd(_phrase: string): string {
    return "Worked";
}

function fmtTokens(t: TurnTokens): string {
    return `\u2191${formatCompactNumber(t.input)} \u2193${formatCompactNumber(t.output)}`;
}

// ── AgentWorkingRow ───────────────────────────────────────────────────────────
// Rendered immediately below AgentDocumentView in the conversation area.
// Shows spinner + "Working… · Ns" while loading, "✓ Worked · 42s" on completion.
// Returns null when neither loading nor has stats — no idle placeholder.
// Stays visible as a turn delimiter until the user sends the next message.

/** Truncate tool arg to `max` chars, left-truncating file paths to preserve filename. */
function abbreviateArg(s: string, max: number): string {
    return abbreviateText(s, max, { pathAware: true });
}

interface AgentWorkingRowProps {
    loading: boolean;
    stopping?: boolean;
    currentTool?: string | null;
    /** First significant argument of the active tool (file path, command, etc.).
     *  When set alongside currentTool, shown as "tool · arg" in the left zone. */
    currentToolArg?: string | null;
    /** True once the current tool call has been promoted to a live
     *  ActivityDock row (tool-adapter.ts) — the row falls back to the
     *  cycling "Working…" phrase instead of repeating "tool · arg", since
     *  the dock already shows it. See REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md §4.3. */
    toolPromoted?: boolean;
    sessionStats?: SessionStats | null;
    turnTokens?: TurnTokens | null;
    /** Set when the provider is rate-limited; shows "Rate limited…" in place of thinking phrase. */
    waitingReason?: "rate_limited" | null;
    /** Milliseconds until next retry (from provider Retry-After). Shown when waitingReason is set. */
    retryAfterMs?: number | null;
    /** What the launch/login flow is doing right now — shown in place of the
     *  generic thinking phrase so a timed wait (login link, login completion)
     *  never renders as bare "Working…". See launch-phase.ts. */
    launchPhase?: LaunchPhase | null;
    /** Cancel the in-flight login. Shown as a small button while launchPhase
     *  is a pre-authUrl wait (opening-login-terminal /
     *  waiting-for-login-completion) — AuthUrlBox has its own cancel button
     *  once a URL exists, but before that this is the only affordance. */
    onCancelLogin?: () => void;
    /** True while AuthUrlBox is already showing its own Cancel button (a URL
     *  was captured — reagent P2 on PR #2300: tier 1's "opened" outcome sets
     *  launchPhase to "waiting-for-login-completion" too, so without this the
     *  row rendered a second, redundant Cancel button alongside AuthUrlBox's. */
    hasAuthUrl?: boolean;
}

// reagent P2 on PR #2304: "waiting-for-login-link" (tier 1's own up-to-15s
// countdown) was missing here, so that phase's real timed wait had no cancel
// affordance anywhere — AuthUrlBox can't show one yet (no URL captured), and
// the row's own button was withheld for exactly this phase.
const CANCELLABLE_LAUNCH_PHASES = new Set([
    "first-login",
    "auth-expired",
    "waiting-for-login-link",
    "opening-login-terminal",
    "waiting-for-login-completion",
]);

/** The exact string the loading row's left zone shows right now — pulled out
 *  of the JSX ternary chain so both the type-out reveal effect and the
 *  render itself read the same computed value. */
function loadingLeftText(props: AgentWorkingRowProps, phrase: string, nowMs: number): string {
    if (props.stopping) return "Stopping…";
    if (props.waitingReason === "rate_limited") {
        return props.retryAfterMs != null
            ? `Rate limited — retrying in ${Math.ceil(props.retryAfterMs / 1000)}s`
            : "Rate limited — retrying…";
    }
    const phaseLabel = formatPhaseLabel(props.launchPhase, nowMs);
    if (phaseLabel) return phaseLabel;
    if (props.currentTool && !props.toolPromoted) {
        return props.currentToolArg
            ? `${props.currentTool}  ·  ${abbreviateArg(props.currentToolArg, 40)}`
            : props.currentTool;
    }
    return `${phrase}…`;
}

export const AgentWorkingRow = (props: AgentWorkingRowProps): JSX.Element => {
    const [phrase, setPhrase] = createSignal(pickThinkingPhrase());
    const [lastPhrase, setLastPhrase] = createSignal(pickThinkingPhrase());
    const tick = useTick(1000);

    // ── Type-out + shimmer ──────────────────────────────────────────────
    // First display of a new left-zone string (a phrase, tool name, or
    // status change) reveals character-by-character; once fully revealed,
    // a gradient highlight sweeps back and forth over it for as long as
    // this string stays on screen. See
    // SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md §2.
    //
    // Reduced motion: read the app's centralized atom (settings override OR
    // OS preference — see atoms.prefersReducedMotionAtom / BrainSpinner.tsx),
    // not a one-off matchMedia() call, so it stays in sync with the same
    // signal every other animated element in the app honors. Skips the
    // one-shot type-out (jumps straight to full text, matching modal.tsx's
    // "brief, non-moving" convention for one-shot reveals under reduced
    // motion) but keeps the shimmer running — same precedent as
    // .agent-pane-progress-bar's marching ants above: this is a small,
    // essential, looping "still working" signal, not large/parallax motion,
    // so it slows down (CSS below) rather than disappearing entirely.
    const reducedMotion = atoms.prefersReducedMotionAtom;
    // tick() re-runs this memo every second so a phase's "up to Ys" countdown
    // (formatPhaseLabel) stays live — see useTick.ts's "always-on tick" pattern.
    const leftText = createMemo(() => loadingLeftText(props, phrase(), (tick(), Date.now())));
    const [revealed, setRevealed] = createSignal(leftText().length);
    const [typing, setTyping] = createSignal(false);
    const REVEAL_CHAR_MS = 28;

    // The very first text after ENTERING the loading state renders in full
    // instantly — the type-out reveal is a transition effect for text
    // changes while already visibly working (tool → tool → phrase). Playing
    // it on entry meant "Working…" trailed the Enter keypress by
    // ~REVEAL_CHAR_MS × 8 ≈ 250ms of a nearly-empty row, reading as "the
    // indicator comes up late" even though the state flip is synchronous
    // with the send (user report 2026-08-10). Plain (non-reactive) flag:
    // only the loading edge below writes it, only the reveal effect reads it.
    let revealInstantly = true;
    createEffect(() => {
        if (!props.loading) revealInstantly = true;
    });

    // The shimmer class is ALWAYS on (baked into the class attribute below);
    // this effect only toggles `.is-typing`, which overlays opaque white
    // text during the reveal. Toggling the shimmer class itself on every
    // phrase/tool change would restart the CSS animation from 0% each time
    // — and since the sweep period doesn't divide evenly into the interval
    // between tool transitions, the return (right→left) leg kept getting
    // truncated, so the highlight visibly "went right but never came back"
    // (found in the sandbox, sandbox/working-shimmer.html on this machine).
    createEffect(() => {
        const text = leftText();
        if (reducedMotion() || !text || revealInstantly) {
            revealInstantly = false;
            setRevealed(text.length);
            setTyping(false);
            return;
        }
        setTyping(true);
        setRevealed(0);
        const id = setInterval(() => {
            setRevealed((n) => {
                const next = n + 1;
                if (next >= text.length) {
                    clearInterval(id);
                    setTyping(false);
                }
                return next;
            });
        }, REVEAL_CHAR_MS);
        onCleanup(() => clearInterval(id));
    });
    const [loadStartMs, setLoadStartMs] = createSignal<number | null>(null);
    const elapsedMs = createMemo(() => {
        const s = loadStartMs();
        return s != null ? (tick(), Date.now() - s) : 0;
    });

    createEffect(() => {
        if (!props.loading || (props.currentTool && !props.toolPromoted)) return;
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
        if (props.loading && (!props.currentTool || props.toolPromoted)) setLastPhrase(phrase());
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
        right.push(formatElapsedCompact(elapsedMs()));
        return right.join("  ·  ");
    });

    const showCancelLogin = createMemo(
        () =>
            !!props.onCancelLogin &&
            !props.hasAuthUrl &&
            !!props.launchPhase &&
            CANCELLABLE_LAUNCH_PHASES.has(props.launchPhase.kind),
    );

    // No dedicated "Running in background" state here — the ActivityDock's
    // own running row (pinned above the composer, same data source) IS the
    // indicator for a live attached task; a footer copy of it was redundant
    // (user feedback 2026-08-10, reverting the render half of #2489 — the
    // reducer axis itself stays, for the watchdog/Swarm consumers).
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
                <span class="agent-working-row-left agent-working-shimmer" classList={{ "is-typing": typing() }}>
                    {leftText().slice(0, revealed())}
                </span>
                <span class="agent-working-row-right">
                    {rightText()}
                    <Show when={showCancelLogin()}>
                        <button
                            class="agent-working-row-cancel-login"
                            onClick={() => props.onCancelLogin?.()}
                        >
                            Cancel
                        </button>
                    </Show>
                </span>
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
     * Called on Esc when the textarea is empty (or whitespace only). With
     * text in the textarea, Esc clears instead. See SPEC_AGENT_PANE_FOLLOWUPS
     * item #9.
     *
     * The parent decides what Esc actually does, mirroring Claude Code CLI:
     * if a message is already queued (sitting behind a running turn), Esc
     * delivers it to the live agent right now instead of waiting for the
     * next natural breakpoint — "stop and consider this now." Only when
     * nothing is queued does it fall back to sending SIGINT (equivalent to
     * Ctrl+C in a terminal). See SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md.
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
     * callers that haven't been updated still type-check. Also read for
     * the ghost-text next-prompt suggestion (term:next_prompt_suggestion —
     * see docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md).
     */
    viewModel?: AgentViewModel;
    /**
     * Exposes an `isEmpty()` check on the (uncontrolled) textarea up to the
     * parent, called once on mount. useNextPromptSuggestion.ts needs this at
     * RPC-response time (which happens outside this component) to avoid
     * writing a suggestion after the user has already started typing — see
     * that hook's doc comment, guard 2.
     */
    isComposerEmptyRef?: (fn: () => boolean) => void;
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

    // Composer placeholder text, in precedence order:
    //   1. Ghost-text next-prompt suggestion (term:next_prompt_suggestion) —
    //      Haiku-predicted next message, accept with Tab (see handleKeyDown
    //      below). Only meaningful while the box is empty, which is exactly
    //      when a native `placeholder` attribute renders anyway.
    //   2. "Speak to <agent>…" while voice is listening AND this pane owns
    //      the session, so it's obvious transcription is live and routed here.
    //   3. The default "Send message to <agent>…" prompt.
    // Reactive: reads the block meta atom + the voice singleton's SignalAtoms
    // so it flips the instant either changes.
    const voice = getVoiceSession();
    const placeholder = createMemo(() => {
        const vm = props.viewModel;
        const suggestion = vm?.blockAtom()?.meta?.["term:next_prompt_suggestion"] as string | undefined;
        if (suggestion) return suggestion;
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

    // Tracks the box's emptiness immediately BEFORE the current edit — used
    // by the ghost-text clear check in handleInput below (SPEC_AMBIENT_GHOST_
    // TEXT_NEXT_PROMPT_2026_07_03.md §4.3). Every programmatic write to
    // textareaRef.value MUST go through writeComposerValue (never assign
    // `.value` directly) so this stays in sync — none of these writers
    // dispatch a real `input` event, so handleInput never sees them, and a
    // stale flag either misses clearing a real stale suggestion or
    // re-triggers the clear check on an edit that isn't actually the first
    // one from empty. Seeded `true` (safe default: even if wrong for a mount
    // with restored draft text, ghost text is only ever shown for an empty
    // composer, so there's nothing to dismiss in that case anyway).
    let boxWasEmpty = true;
    const writeComposerValue = (text: string): void => {
        if (!textareaRef) return;
        textareaRef.value = text;
        boxWasEmpty = text.length === 0;
    };

    const acceptCompletion = (cmd: SlashCommand): void => {
        if (!textareaRef) return;
        writeComposerValue(`/${cmd.name} `);
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
        // Consumes the Esc-cleared snapshot — the undo-restore call below
        // reads `escClearedDraft` as its `text` argument BEFORE this runs, so
        // nulling it here doesn't affect that write. Every other caller
        // (ghost-text accept, history recall) is a fresh edit that should
        // supersede a stale snapshot the same way typing does.
        escClearedDraft = null;
        writeComposerValue(text);
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
        // Any real keystroke (typed or voice-dispatched) supersedes the
        // Esc-cleared snapshot — an empty box reached by typing-then-deleting
        // is no longer "the direct result of Esc", so Ctrl/Cmd+Z must fall
        // through to native undo instead of resurrecting stale text. (reagent
        // P1 / codex P2 on PR #2497.)
        escClearedDraft = null;
        // Perf marks per SPEC_INPUT_RESPONSIVENESS §7.1. Target: handler
        // body P95 < 5 ms. The mark span ends BEFORE the RAF enqueue so
        // we measure only the synchronous handler cost.
        markStart("agent-keystroke");
        // The first edit into a previously-empty box dismisses any pending
        // ghost-text suggestion — it must not silently reappear if the user
        // later deletes back to empty. Checks the box's PRE-edit emptiness
        // (boxWasEmpty) rather than the post-edit length: a `value.length
        // === 1` check only catches a single typed keystroke and misses
        // paste or voice-transcript inserts, which dispatch the same
        // `input` event but can write multiple characters into an empty
        // box at once (reagentx review on #1961). See
        // docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md §4.3.
        const newValue = textareaRef?.value ?? "";
        if (boxWasEmpty && newValue.length > 0) {
            const vm = props.viewModel;
            if (vm?.blockAtom()?.meta?.["term:next_prompt_suggestion"]) {
                fireAndForget(() =>
                    ObjectService.UpdateObjectMeta(makeORef("block", vm.blockId), {
                        "term:next_prompt_suggestion": null,
                    } as any)
                );
            }
        }
        boxWasEmpty = newValue.length === 0;
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

    // Expose an isEmpty() check on this uncontrolled textarea up to the
    // parent — useNextPromptSuggestion.ts needs it at RPC-response time
    // (outside this component) to avoid writing a stale ghost-text
    // suggestion after the user already started typing.
    onMount(() => {
        props.isComposerEmptyRef?.(() => (textareaRef?.value.length ?? 0) === 0);
    });

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

    // Draft cleared by Esc, held for Undo. Plain (non-reactive) — read
    // fresh at Ctrl/Cmd+Z time and at context-menu-build time, never
    // rendered. One level deep is deliberate: Esc-clear is the only
    // destructive programmatic edit (it bypasses the browser's native undo
    // stack via direct .value assignment); ordinary typing keeps native
    // undo, which the shortcut falls through to below.
    //
    // Invalidated (set back to null) by handleInput, setComposerValue, and
    // handleSend — any edit or send after the Esc-clear means a later empty
    // box is no longer "the direct result of" that Esc-clear, so it must not
    // resurrect stale text. (reagent P1 / codex P2 on PR #2497.)
    let escClearedDraft: string | null = null;
    const undoComposer = (): void => {
        if (!textareaRef) return;
        textareaRef.focus();
        if (escClearedDraft != null && textareaRef.value.length === 0) {
            // setComposerValue nulls escClearedDraft itself (it reads this
            // argument before doing so) — no separate reset needed here.
            setComposerValue(escClearedDraft);
            return;
        }
        // No snapshot to restore — fall through to the browser's own undo
        // (covers plain typed edits; a no-op if its stack is empty).
        document.execCommand("undo");
    };
    /** "Undo" entry for the composer's right-click menu — leading item
     *  above the standard Cut/Copy/Paste block (contextmenu.ts). Enabled
     *  when there's an Esc-cleared draft to restore or any text native
     *  undo could plausibly act on. */
    const composerUndoItems = (): ContextMenuItem[] => [
        {
            label: "Undo",
            enabled: escClearedDraft != null || (textareaRef?.value.length ?? 0) > 0,
            click: undoComposer,
        },
    ];

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
            writeComposerValue("");
            // A sent message supersedes any pending Esc-cleared snapshot —
            // the now-empty box must not resurrect old text via Ctrl/Cmd+Z.
            // (reagent P1 on PR #2497.)
            escClearedDraft = null;
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
        // Ghost-text next-prompt suggestion: Tab accepts it into the real
        // input, matching Claude Code CLI's own terminal UX (see
        // docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md).
        // Right Arrow is a deliberate AgentMux addition on top of that
        // parity baseline — not something Claude Code's own CLI does — for
        // the fish-shell/zsh-autosuggestions/Copilot-style convenience of
        // accepting an inline suggestion with the "continue typing right"
        // key, letting the user just press Enter after. Safe to claim here:
        // with the textarea empty there's no cursor to move, so Right Arrow
        // is otherwise a no-op in this state.
        // Only reachable when the textarea is empty, which is also the only
        // state the slash-autocomplete branch below can't be in (it requires
        // a `/prefix`) — these handlers never compete with it.
        if ((e.key === "Tab" || e.key === "ArrowRight") && textareaRef && textareaRef.value.length === 0) {
            const vm = props.viewModel;
            const suggestion = vm?.blockAtom()?.meta?.["term:next_prompt_suggestion"] as string | undefined;
            if (suggestion) {
                e.preventDefault();
                setComposerValue(suggestion);
                fireAndForget(() =>
                    ObjectService.UpdateObjectMeta(makeORef("block", vm!.blockId), {
                        "term:next_prompt_suggestion": null,
                    } as any)
                );
                return;
            }
        }
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
        // Undo an Esc-clear — platform-standard shortcut (Ctrl+Z on
        // Windows/Linux, Cmd+Z on macOS). Only intercepted when the
        // composer is empty and a cleared draft exists: in every other
        // state the event falls through to the browser's native undo for
        // ordinary typed edits.
        if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "z") {
            if (textareaRef && textareaRef.value.length === 0 && escClearedDraft != null) {
                e.preventDefault();
                undoComposer();
            }
            return;
        }
        if (e.key === "Escape") {
            // Esc semantics (SPEC_AGENT_PANE_FOLLOWUPS item #9):
            //  - textarea has text → clear it, stay focused
            //  - textarea is empty → send SIGINT to the agent CLI
            if (!textareaRef) return;
            if (textareaRef.value.trim().length > 0) {
                e.preventDefault();
                // Snapshot for Undo (Ctrl/Cmd+Z or the right-click menu) —
                // the programmatic clear below bypasses the browser's own
                // undo stack (direct .value assignment), so without this
                // an accidental Esc destroyed the draft irrecoverably.
                escClearedDraft = textareaRef.value;
                writeComposerValue("");
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
                    spellcheck={false}
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
                    // The pane body's own onContextMenu (blockframe.tsx) only
                    // ever offers Copy-on-selection and never Paste (agent
                    // panes aren't `view: "term"`) — without this, right-click
                    // here silently shows a useless disabled-Copy menu instead
                    // of letting the user paste. See
                    // docs/specs/REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07.md.
                    onContextMenu={(e) => void showTextInputContextMenu(e, composerUndoItems())}
                    rows={1}
                />
                {/* Pinned to the composer's right edge instead of the pane's
                    header (moved off blockframe.tsx — see
                    SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md).
                    Uses the same viewModel.voiceHandle() the header button
                    used to, so voice wiring (registerPane, transcript
                    writes) is unchanged — only the render location moved. */}
                <Show when={props.viewModel?.voiceHandle}>
                    <div class="agent-input-mic">
                        <MicButton
                            blockId={props.viewModel!.blockId}
                            handle={props.viewModel!.voiceHandle!()}
                            paneTitle="Speak into this agent (Ctrl+Shift+V)"
                        />
                    </div>
                </Show>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
