// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer's key handling, one function per concern. `handleKeyDown` asks them in a fixed order
 * and stops at the first that takes the key; each returns whether it did.
 */

import { caretAtSelectionEdge } from "./composer-caret";
import { loadOlderAgentHistory } from "../agent-input-history";
import type { ComposerCtx } from "./composer-context";
import { acceptCompletion, handleSend, setComposerValue, undoComposer, writeComposerValue } from "./composer-actions";

/** Backspace at the very start of the text, with nothing selected, moves into the attachment tray (Slack's pattern; spec §5.9). */
function keyBackspaceToTray(c: ComposerCtx, e: KeyboardEvent): boolean {
    const ta = c.textarea();
    if (
        e.key === "Backspace" &&
        ta &&
        ta.selectionStart === 0 &&
        ta.selectionEnd === 0 &&
        (c.attachmentDraft?.count() ?? 0) > 0 &&
        c.focusLastTile
    ) {
        e.preventDefault();
        c.focusLastTile();
        return true;
    }
    return false;
}

/**
 * PageUp/PageDown: the textarea is a flex sibling of the chat history, not a descendant, and
 * `.agent-view` is overflow:hidden, so left unhandled the browser scrolls whatever it finds outside
 * the pane. The exception: a long draft grows the textarea to its own 200px cap and it becomes its
 * own scroll container, which should page normally, so the key is only swallowed when the textarea
 * has nothing of its own left to scroll. The decision must be synchronous (a deferred read would make
 * preventDefault() too late).
 */
function keyPageScroll(c: ComposerCtx, e: KeyboardEvent): boolean {
    if (e.key !== "PageUp" && e.key !== "PageDown") return false;
    const el = c.textarea();
    if (el && el.scrollHeight > el.clientHeight) { // perf:allow-layout-read — not the typing hot path, and the decision must be synchronous
        return true;
    }
    e.preventDefault();
    return true;
}

/**
 * Ghost-text next-prompt suggestion: Tab accepts it into the real input, as Claude Code CLI does
 * (docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md); Right Arrow is an AgentMux addition
 * for the fish/zsh-autosuggestions convenience. Only reachable with the textarea empty, which the
 * slash dropdown cannot be in. The suggestion in meta is not cleared: accepting then deleting should
 * end in the same empty-box state as typing then deleting (SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md §3.3).
 */
function keyAcceptSuggestion(c: ComposerCtx, e: KeyboardEvent): boolean {
    const ta = c.textarea();
    if ((e.key === "Tab" || e.key === "ArrowRight") && ta && ta.value.length === 0) {
        const suggestion = c.visibleSuggestion();
        if (suggestion) {
            e.preventDefault();
            setComposerValue(c, suggestion);
            return true;
        }
    }
    return false;
}

/** The dropdown's keys take precedence while it is open with a match: arrows navigate, Tab fills the name in for editing, Enter fills it in and sends, Esc dismisses. */
function keyAutocomplete(c: ComposerCtx, e: KeyboardEvent): boolean {
    if (c.autocompletePrefix() === null || c.completions().length === 0) return false;
    const list = c.completions();
    if (e.key === "ArrowDown") {
        e.preventDefault();
        c.setAutocompleteIndex((i) => (i + 1) % list.length);
        return true;
    }
    if (e.key === "ArrowUp") {
        e.preventDefault();
        c.setAutocompleteIndex((i) => (i - 1 + list.length) % list.length);
        return true;
    }
    if (e.key === "Tab") {
        e.preventDefault();
        const match = list[c.autocompleteIndex()];
        if (match) acceptCompletion(c, match);
        return true;
    }
    if (e.key === "Enter") {
        e.preventDefault();
        const match = list[c.autocompleteIndex()];
        if (match) {
            acceptCompletion(c, match);
            handleSend(c);
        }
        return true;
    }
    if (e.key === "Escape") {
        e.preventDefault();
        c.setAutocompletePrefix(null);
        return true;
    }
    return false;
}

function keyHistorySearch(c: ComposerCtx, e: KeyboardEvent): boolean {
    if (c.historyAgent && e.key === "r" && e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey) {
        e.preventDefault();
        c.setHistorySearch(true);
        return true;
    }
    return false;
}

/**
 * ArrowUp / ArrowDown: queued-message recall, then sent-message history.
 *
 * On an EMPTY composer ArrowUp first un-queues the most recently queued ("send now") held message
 * (the Claude-Code-CLI gesture). Otherwise ArrowUp walks back through sent messages and ArrowDown
 * forward toward the live draft. ArrowUp gates on the active selection edge being at true position 0,
 * not "the first visual line", so multiline editing and Shift-selection still move normally. ArrowDown
 * is not the mirror: it fires at the true end, or at position 0 only when the shown entry has no
 * newline (a multi-line entry's later lines would be stranded behind an early jump). Landing back on
 * the live draft parks the caret at the END, since that is the user's own text to resume typing.
 */
function keyHistoryWalk(c: ComposerCtx, e: KeyboardEvent): boolean {
    const ta = c.textarea();
    if (!ta || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return false;
    const { hist, sentHistory } = c;
    const empty = ta.value.length === 0;
    const navigating = hist.pos < sentHistory.length;
    const { first: caretAtStart, last: caretAtEnd } = caretAtSelectionEdge(ta);
    const currentIsSingleLine = !ta.value.includes("\n");

    if (e.key === "ArrowUp" && empty) {
        const recalled = c.props.onRecallLatestQueued?.();
        if (recalled) {
            e.preventDefault();
            setComposerValue(c, recalled.text);
            if (recalled.attachments?.length) void c.attachmentDraft?.restoreRefs(recalled.attachments);
            return true;
        }
    }
    // Near the oldest entry shown, read the page before it.
    if (e.key === "ArrowUp" && c.historyAgent && hist.pos <= 10 && caretAtStart) {
        void loadOlderAgentHistory(c.historyAgent);
    }
    // Older: a partially typed draft is stashed (restorable with ArrowDown), and the caret lands at
    // position 0 so a repeated ArrowUp steps back again at once.
    if (e.key === "ArrowUp" && hist.pos > 0 && caretAtStart) {
        if (!navigating) hist.draft = ta.value;
        hist.pos--;
        e.preventDefault();
        setComposerValue(c, sentHistory[hist.pos], "start");
        return true;
    }
    // Newer: only while navigating, and only once there is truly nowhere left below.
    if (e.key === "ArrowDown" && navigating && (caretAtEnd || (caretAtStart && currentIsSingleLine))) {
        hist.pos++;
        e.preventDefault();
        const atLiveDraft = hist.pos >= sentHistory.length;
        setComposerValue(c, atLiveDraft ? hist.draft : sentHistory[hist.pos], atLiveDraft ? "end" : "start");
        return true;
    }
    return false;
}

/**
 * Undo an Esc-clear (Ctrl+Z, or Cmd+Z on macOS). Intercepted only when the composer is empty and a
 * cleared draft exists; in every other state the key falls through to the browser's native undo.
 * The key is consumed here either way.
 */
function keyUndo(c: ComposerCtx, e: KeyboardEvent): boolean {
    if (!((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "z")) return false;
    const ta = c.textarea();
    if (ta && ta.value.length === 0 && c.escClearedDraft != null) {
        e.preventDefault();
        undoComposer(c);
    }
    return true;
}

/**
 * Esc (SPEC_AGENT_PANE_FOLLOWUPS item #9): with text, clear it and stay focused; empty, send SIGINT
 * to the agent CLI. The clear is snapshotted for Undo, since the direct `.value` write bypasses the
 * browser's undo stack.
 */
function keyEscape(c: ComposerCtx, e: KeyboardEvent): boolean {
    if (e.key !== "Escape") return false;
    const ta = c.textarea();
    if (!ta) return true;
    if (ta.value.trim().length > 0) {
        e.preventDefault();
        c.escClearedDraft = ta.value;
        writeComposerValue(c, "");
        c.setIsBangCmd(false);
        // Clearing exits history navigation. It dispatches no `input`, so handleInput's reset would not fire.
        c.hist.pos = c.sentHistory.length;
        c.hist.draft = "";
        // Kick the RAF-debounced typing scroll so the footer's auto-grow collapses and the document stays anchored.
        c.props.onTyping?.();
    } else {
        e.preventDefault();
        c.props.onStopAgent?.();
    }
    return true;
}

export function handleKeyDown(c: ComposerCtx, e: KeyboardEvent): void {
    // IME composition: while a CJK / Vietnamese / etc. IME is converting, Enter/Tab/Arrows/Esc confirm
    // or navigate composition candidates and the IME owns them. `keyCode === 229` is Safari's synthetic
    // keydown for IME events without `isComposing`; both checks are load-bearing (SPEC_INPUT_RESPONSIVENESS §6.2).
    if (e.isComposing || e.keyCode === 229) return;
    if (keyBackspaceToTray(c, e)) return;
    if (keyPageScroll(c, e)) return;
    if (keyAcceptSuggestion(c, e)) return;
    if (keyAutocomplete(c, e)) return;
    if (keyHistorySearch(c, e)) return;
    if (keyHistoryWalk(c, e)) return;
    if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend(c);
        return;
    }
    if (keyUndo(c, e)) return;
    keyEscape(c, e);
}
