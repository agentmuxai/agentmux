// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the composer does to its textarea and to the pane: programmatic writes, input handling,
 * send, undo of an Esc-clear, and restoring a draft on mount. The textarea is UNCONTROLLED (the DOM
 * owns the value, for typing performance), so every write here sets `.value` directly and persists
 * the draft itself.
 */

import { markEnd, markStart } from "@/perf";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { MENU_ICON } from "@/app/util/menu-icons";
import { isBangCommand } from "../bang-command";
import { attachmentNoun, fileKind } from "../attachments/file-kind";
import { getComposerDraft, saveSentHistory, SENT_HISTORY_MAX, setComposerDraft } from "../reload-persistence";
import { agentDraft, rememberSent, saveAgentDraft } from "../agent-input-history";
import type { SlashCommand } from "../commands/types";
import type { ComposerCtx } from "./composer-context";

/** Show the dropdown only for a value that starts with `/` and has no space or newline: past the command name the user is filling in args. */
export function updateAutocomplete(c: ComposerCtx): void {
    const ta = c.textarea();
    if (!ta) return;
    if (c.composing) return;
    const val = ta.value;
    if (val.startsWith("/") && !val.includes(" ") && !val.includes("\n")) {
        const prefix = val.slice(1);
        if (c.autocompletePrefix() !== prefix) {
            c.setAutocompletePrefix(prefix);
            c.setAutocompleteIndex(0);
        }
    } else if (c.autocompletePrefix() !== null) {
        c.setAutocompletePrefix(null);
    }
}

/**
 * Every programmatic composer write goes through here (Esc-clear, undo-restore, history recall,
 * ghost-text accept, autocomplete accept, voice). Persisting here, not just on input, means an
 * Esc-clear to empty also clears the persisted draft instead of resurrecting the pre-clear text on
 * the next tab switch back.
 */
export function writeComposerValue(c: ComposerCtx, text: string): void {
    const ta = c.textarea();
    if (!ta) return;
    ta.value = text;
    if (c.draftBlockId) setComposerDraft(c.draftBlockId, text);
    if (c.historyAgent) saveAgentDraft(c.historyAgent, text);
}

export function acceptCompletion(c: ComposerCtx, cmd: SlashCommand): void {
    const ta = c.textarea();
    if (!ta) return;
    writeComposerValue(c, `/${cmd.name} `);
    c.setAutocompletePrefix(null);
    c.setIsBangCmd(false);
    ta.focus();
    c.props.onTyping?.();
}

/**
 * Replace the composer's content programmatically (history recall, queued un-send) and park the
 * caret. Deliberately does NOT dispatch an `input` event, so it never trips the history-cursor reset
 * in `handleInput`. It consumes the Esc-cleared snapshot: every caller is a fresh edit that
 * supersedes it, as typing does (the undo-restore reads the snapshot as its `text` argument first).
 */
export function setComposerValue(c: ComposerCtx, text: string, caret: "start" | "end" = "end"): void {
    const ta = c.textarea();
    if (!ta) return;
    c.escClearedDraft = null;
    writeComposerValue(c, text);
    const pos = caret === "start" ? 0 : text.length;
    ta.setSelectionRange(pos, pos);
    c.setIsBangCmd(isBangCommand(text));
    updateAutocomplete(c);
    c.props.onTyping?.();
}

/**
 * The typing handler is tightly scoped: one boolean check and, at most once per frame, a
 * requestAnimationFrame enqueue, with no layout reads (SPEC_INPUT_RESPONSIVENESS §7.1; the mark span
 * ends before the RAF enqueue so it measures only the synchronous cost). The ghost-text suggestion is
 * deliberately NOT cleared here: only a new turn and session end clear it, so deleting back to empty
 * shows it again (SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md §2).
 */
export function handleInput(c: ComposerCtx): void {
    const ta = c.textarea();
    // A manual edit exits history navigation: the next ArrowUp starts again from the newest sent message.
    c.hist.pos = c.sentHistory.length;
    // Any real keystroke supersedes the Esc-cleared snapshot, so Ctrl/Cmd+Z falls through to native undo.
    c.escClearedDraft = null;
    // Persist the draft synchronously (a Map.set needs no debounce, and must not depend on the optional onTyping).
    if (c.draftBlockId && ta) setComposerDraft(c.draftBlockId, ta.value);
    if (c.historyAgent && ta) saveAgentDraft(c.historyAgent, ta.value);
    markStart("agent-keystroke");
    updateAutocomplete(c);
    c.setIsBangCmd(isBangCommand(ta?.value ?? ""));
    const cb = c.props.onTyping;
    if (!cb) {
        markEnd("agent-keystroke", "done");
        return;
    }
    if (c.typingScrollPending) {
        markEnd("agent-keystroke", "coalesced");
        return;
    }
    c.typingScrollPending = true;
    markStart("agent-input-raf");
    requestAnimationFrame(() => {
        c.typingScrollPending = false;
        markEnd("agent-input-raf", "fired");
        markStart("agent-input-raf-cb");
        cb();
        markEnd("agent-input-raf-cb", "done");
    });
    markEnd("agent-keystroke", "scheduled");
}

/** Restore the Esc-cleared draft into an empty box, else fall through to the browser's own undo (a no-op if its stack is empty). */
export function undoComposer(c: ComposerCtx): void {
    const ta = c.textarea();
    if (!ta) return;
    ta.focus();
    if (c.escClearedDraft != null && ta.value.length === 0) {
        setComposerValue(c, c.escClearedDraft);
        return;
    }
    document.execCommand("undo");
}

/** The "Undo" entry for the composer's right-click menu: enabled with an Esc-cleared draft to restore or any text native undo could act on. */
export function composerUndoItems(c: ComposerCtx): ContextMenuItem[] {
    return [
        {
            label: "Undo",
            icon: MENU_ICON.undo,
            enabled: c.escClearedDraft != null || (c.textarea()?.value.length ?? 0) > 0,
            click: () => undoComposer(c),
        },
    ];
}

/** Record a sent message in the shell-style history, skipping a consecutive duplicate. */
function recordSent(c: ComposerCtx, message: string): void {
    if (c.historyAgent) {
        rememberSent(c.historyAgent, message);
    } else if (message.trim() && message !== c.sentHistory[c.sentHistory.length - 1]) {
        c.sentHistory.push(message);
        if (c.sentHistory.length > SENT_HISTORY_MAX) c.sentHistory.shift();
        if (c.draftBlockId) saveSentHistory(c.draftBlockId);
    }
}

/** Why a send must wait, or null. The pane can show before its conversation has loaded, and a send then is refused after this handler has cleared the draft, so the message is kept (#4374). */
function sendBlockedHint(c: ComposerCtx): string | null {
    const processing = c.attachmentDraft?.processingCount() ?? 0;
    if (processing > 0) {
        const kinds = c.attachmentDraft!
            .items()
            .filter((i) => i.status === "processing")
            .map((i) => fileKind(i.info?.kind, i.name));
        return `Waiting for ${processing} ${attachmentNoun(processing, kinds)} to finish processing`;
    }
    return null;
}

export function handleSend(c: ComposerCtx): void {
    const ta = c.textarea();
    if (!ta) return;
    const message = ta.value;
    const processingHint = sendBlockedHint(c);
    if (processingHint) {
        c.showSendHint(processingHint);
        return;
    }
    const attachments = c.attachmentDraft?.refs() ?? [];
    if (!message.trim() && attachments.length === 0) return;
    if (c.draftBlockId && paneSnapshot(c.draftBlockId)?.initPhase.kind === "InitPending") {
        c.showSendHint("Waiting for the conversation to load");
        return;
    }
    if (!c.props.onSendMessage) return;
    // agent-submit span (SPEC_INPUT_RESPONSIVENESS §7.1) includes the synchronous onSendMessage cost.
    markStart("agent-submit");
    // Plain messages keep the one-argument call.
    if (attachments.length > 0) {
        c.props.onSendMessage(message, attachments);
        c.attachmentDraft?.clearAfterSend();
    } else {
        c.props.onSendMessage(message);
    }
    recordSent(c, message);
    c.hist.pos = c.sentHistory.length;
    c.hist.draft = "";
    // Hide the suggestion that is current in meta right now (useComposerPlaceholder).
    c.maskCurrentSuggestion();
    writeComposerValue(c, "");
    // A sent message supersedes any pending Esc-cleared snapshot: the empty box must not resurrect old text via Ctrl/Cmd+Z.
    c.escClearedDraft = null;
    c.setAutocompletePrefix(null);
    c.setIsBangCmd(false);
    // Scroll the new user message into view (the document signal is flushed synchronously before this point).
    c.props.onTyping?.();
    markEnd("agent-submit", "sent");
}

/**
 * Restore a draft left behind by a prior mount of this block (a tab switch away and back). A direct
 * `.value` write, like every other programmatic write here, that does not dispatch `input` and so
 * does not trip the history-cursor reset before the user has touched anything. After a restart or in
 * a new pane, the agent's own draft is used if the composer is still empty when it arrives.
 */
export function restoreDraftOnMount(c: ComposerCtx): void {
    if (!c.textarea()) return;
    const restore = (draft: string | undefined) => {
        const ta = c.textarea();
        if (!draft || !ta || ta.value) return;
        ta.value = draft;
        c.setIsBangCmd(isBangCommand(draft));
        updateAutocomplete(c);
    };
    const draft = c.draftBlockId ? getComposerDraft(c.draftBlockId) : undefined;
    if (draft) restore(draft);
    else if (c.historyAgent) void agentDraft(c.historyAgent).then(restore);
}
