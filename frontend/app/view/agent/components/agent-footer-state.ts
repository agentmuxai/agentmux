// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent pane's composer, the state: the footer's props, the context the composer's functions share, and the
// signals its views draw from, created in the order the footer always has (agent-footer-parts.tsx draws them).

import { createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { AgentViewModel } from "../agent-model";
import type { SlashCommand } from "../commands/types";
import { getAttachmentDraft } from "../attachments/attachment-draft";
import { useComposerRestore } from "../composer-restore";
import { getSentHistory } from "../reload-persistence";
import { agentHistory, historyAgentId, movedCursor, watchAgentHistory } from "../agent-input-history";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { useComposerPaste } from "../hooks/useComposerPaste";
import { useComposerFocusHandle, useComposerVoiceHandle } from "../hooks/useComposerHandles";
import { useComposerPlaceholder } from "../hooks/useComposerPlaceholder";
import type { ComposerCtx } from "./composer-context";
import { restoreDraftOnMount, setComposerValue } from "./composer-actions";

export interface AgentFooterProps {
    /**
     * Display name for the composer placeholder ("Send message to …").
     * The caller derives this from `block().meta.agentName ?? agentId`
     * so the placeholder shows the human-readable name (e.g. "Claude")
     * instead of the hex agent UUID. Same fallback chain used elsewhere
     * in `agent-view.tsx` (see the log() call on line 380).
     */
    agentName: string;
    /** `attachments`: ready images from the tray, in order (may be empty). */
    onSendMessage?: (message: string, attachments?: AttachmentRef[]) => void | Promise<void>;
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
    onRecallLatestQueued?: () => { text: string; attachments?: AttachmentRef[] } | null;
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

export type AgentFooterState = ReturnType<typeof createAgentFooter>;

/** The short hint shown above the composer after a send is refused, which goes away by itself after three seconds. */
function createSendHint() {
    const [sendHint, setSendHint] = createSignal<string | null>(null);
    let sendHintTimer: ReturnType<typeof setTimeout> | undefined;
    const showSendHint = (text: string) => {
        setSendHint(text);
        clearTimeout(sendHintTimer);
        sendHintTimer = setTimeout(() => setSendHint(null), 3000);
    };
    onCleanup(() => clearTimeout(sendHintTimer));
    return { sendHint, showSendHint };
}

/**
 * Sent-message history (shell-style ArrowUp / ArrowDown recall), per block and capped, kept like the
 * draft (reload-persistence.ts). A defined agent's history is the agent's own, kept by srv
 * (agent-input-history.ts): it survives a restart, an upgrade and a new pane, loads after mount,
 * pages back on demand and takes sends from other panes, so the cursor moves with what arrives.
 */
function createSentHistory(props: AgentFooterProps, draftBlockId: string | undefined) {
    const historyAgent = historyAgentId(props.viewModel?.blockAtom()?.meta);
    const sentHistory: string[] = historyAgent
        ? agentHistory(historyAgent)
        : draftBlockId
          ? getSentHistory(draftBlockId)
          : [];
    const hist = { pos: sentHistory.length, draft: "" };
    const [historySearch, setHistorySearch] = createSignal(false); // Ctrl+R
    if (historyAgent) {
        onCleanup(
            watchAgentHistory(historyAgent, (where, n, at) => {
                hist.pos = movedCursor(hist.pos, sentHistory.length, where, n, at);
            })
        );
    }
    return { historyAgent, sentHistory, hist, historySearch, setHistorySearch };
}

/** The slash dropdown: the current `/prefix` (without the slash) while the text matches `^/\w*$`, null when hidden. */
function createSlashDropdown(props: AgentFooterProps) {
    const [autocompletePrefix, setAutocompletePrefix] = createSignal<string | null>(null);
    const [autocompleteIndex, setAutocompleteIndex] = createSignal(0);
    const [isBangCmd, setIsBangCmd] = createSignal(false);
    const completions = createMemo<SlashCommand[]>(() => {
        const p = autocompletePrefix();
        if (p === null || !props.getCompletions) return [];
        return props.getCompletions(p);
    });
    return { autocompletePrefix, setAutocompletePrefix, autocompleteIndex, setAutocompleteIndex, isBangCmd, setIsBangCmd, completions };
}

export function createAgentFooter(props: AgentFooterProps) {
    // Uncontrolled textarea: the DOM owns the value, and the value is read through the ref on send, so
    // the component tree does not re-render on every keystroke. Auto-resize is CSS alone
    // (`field-sizing: content` on .agent-input): a JS `scrollHeight` read forced a synchronous layout on
    // every keystroke (~22ms in the agent pane, docs/analysis/agent-typing-lag-trace-2026-04-12.md).
    // The one onInput handler is tightly scoped (docs/specs/SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md §3.4).
    let textareaRef: HTMLTextAreaElement | undefined;
    const setTextarea = (el: HTMLTextAreaElement) => {
        textareaRef = el;
    };

    // Stable for this instance's whole lifetime; undefined for callers that pass no viewModel, in which
    // case draft persistence is a no-op rather than an error.
    const draftBlockId = props.viewModel?.blockId;
    // Files attached to the unsent message (SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §5,
    // SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md), per block like the composer draft.
    const attachmentDraft = draftBlockId ? getAttachmentDraft(draftBlockId) : undefined;
    const { sendHint, showSendHint } = createSendHint();

    const { handlePaste, pasteAttachmentsFromMenu } = useComposerPaste({ blockId: draftBlockId, draft: attachmentDraft, textarea: () => textareaRef });

    const { historyAgent, sentHistory, hist, historySearch, setHistorySearch } = createSentHistory(props, draftBlockId);
    const dropdown = createSlashDropdown(props);
    const { autocompletePrefix, setAutocompletePrefix, autocompleteIndex, setAutocompleteIndex, completions, setIsBangCmd } = dropdown;

    const { visibleSuggestion, placeholder, maskCurrentSuggestion } = useComposerPlaceholder({ viewModel: props.viewModel, agentName: () => props.agentName });

    const c: ComposerCtx = {
        props,
        textarea: () => textareaRef,
        draftBlockId,
        historyAgent,
        attachmentDraft,
        sentHistory,
        hist,
        escClearedDraft: null,
        composing: false,
        typingScrollPending: false,
        focusLastTile: undefined,
        autocompletePrefix,
        setAutocompletePrefix,
        autocompleteIndex,
        setAutocompleteIndex: setAutocompleteIndex as ComposerCtx["setAutocompleteIndex"],
        completions,
        setIsBangCmd,
        setHistorySearch,
        showSendHint,
        visibleSuggestion,
        maskCurrentSuggestion,
    };

    // Exposes an isEmpty() check on this uncontrolled textarea to the parent: useNextPromptSuggestion.ts
    // needs it at RPC-response time (outside this component) to avoid writing a stale ghost-text
    // suggestion after the user already started typing.
    onMount(() => {
        props.isComposerEmptyRef?.(() => (textareaRef?.value.length ?? 0) === 0);
    });
    useComposerRestore(draftBlockId, () => textareaRef, (text) => setComposerValue(c, text));
    onMount(() => restoreDraftOnMount(c));

    useComposerVoiceHandle(props.viewModel, () => textareaRef);
    useComposerFocusHandle(props.viewModel, () => textareaRef);

    return {
        c,
        setTextarea,
        focusTextarea: () => textareaRef?.focus(),
        attachmentDraft,
        sendHint,
        historySearch,
        setHistorySearch,
        historyAgent,
        autocompletePrefix,
        autocompleteIndex,
        setAutocompleteIndex,
        completions,
        isBangCmd: dropdown.isBangCmd,
        placeholder,
        handlePaste,
        pasteAttachmentsFromMenu,
    };
}
