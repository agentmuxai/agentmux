// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 *
 * The composer's behaviour is in composer-actions.ts (writes, input, send, undo) and
 * composer-keys.ts (key handling), over the state in composer-context.ts. Its draft is kept per
 * block outside this component (reload-persistence.ts), so it survives this component's own
 * unmount, which happens on every ordinary pane-stack tab switch; the textarea is UNCONTROLLED
 * (the DOM owns the value, for typing performance), so nothing else would hold a copy. The draft is
 * cleared only on send, never on cleanup.
 * Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.4.
 */

import { Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { MicButton } from "@/app/element/MicButton";
import type { AgentViewModel } from "../agent-model";
import type { SlashCommand } from "../commands/types";
import { SlashAutocomplete } from "./SlashAutocomplete";
import { InputHistorySearch } from "./InputHistorySearch";
import { AttachmentTray } from "../attachments/AttachmentTray";
import { getAttachmentDraft } from "../attachments/attachment-draft";
import { useComposerRestore } from "../composer-restore";
import { getSentHistory } from "../reload-persistence";
import { agentHistory, historyAgentId, movedCursor, watchAgentHistory } from "../agent-input-history";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { shortcutFor } from "@/app/keybindings";
import { useComposerPaste } from "../hooks/useComposerPaste";
import { useComposerFocusHandle, useComposerVoiceHandle } from "../hooks/useComposerHandles";
import { useComposerPlaceholder } from "../hooks/useComposerPlaceholder";
import type { ComposerCtx } from "./composer-context";
import { acceptCompletion, composerUndoItems, handleInput, restoreDraftOnMount, setComposerValue, updateAutocomplete } from "./composer-actions";
import { handleKeyDown } from "./composer-keys";

interface AgentFooterProps {
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

export const AgentFooter = (props: AgentFooterProps): JSX.Element => {
    // Uncontrolled textarea: the DOM owns the value, and the value is read through the ref on send, so
    // the component tree does not re-render on every keystroke. Auto-resize is CSS alone
    // (`field-sizing: content` on .agent-input): a JS `scrollHeight` read forced a synchronous layout on
    // every keystroke (~22ms in the agent pane, docs/analysis/agent-typing-lag-trace-2026-04-12.md).
    // The one onInput handler is tightly scoped (docs/specs/SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md §3.4).
    let textareaRef: HTMLTextAreaElement | undefined;

    // Stable for this instance's whole lifetime; undefined for callers that pass no viewModel, in which
    // case draft persistence is a no-op rather than an error.
    const draftBlockId = props.viewModel?.blockId;
    // Files attached to the unsent message (SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §5,
    // SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md), per block like the composer draft.
    const attachmentDraft = draftBlockId ? getAttachmentDraft(draftBlockId) : undefined;
    const [sendHint, setSendHint] = createSignal<string | null>(null);
    let sendHintTimer: ReturnType<typeof setTimeout> | undefined;
    const showSendHint = (text: string) => {
        setSendHint(text);
        clearTimeout(sendHintTimer);
        sendHintTimer = setTimeout(() => setSendHint(null), 3000);
    };
    onCleanup(() => clearTimeout(sendHintTimer));

    const { handlePaste, pasteAttachmentsFromMenu } = useComposerPaste({ blockId: draftBlockId, draft: attachmentDraft, textarea: () => textareaRef });

    // Sent-message history (shell-style ArrowUp / ArrowDown recall), per block and capped, kept like the
    // draft (reload-persistence.ts). A defined agent's history is the agent's own, kept by srv
    // (agent-input-history.ts): it survives a restart, an upgrade and a new pane, loads after mount,
    // pages back on demand and takes sends from other panes, so the cursor moves with what arrives.
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

    // The slash dropdown: the current `/prefix` (without the slash) while the text matches `^/\w*$`, null when hidden.
    const [autocompletePrefix, setAutocompletePrefix] = createSignal<string | null>(null);
    const [autocompleteIndex, setAutocompleteIndex] = createSignal(0);
    const [isBangCmd, setIsBangCmd] = createSignal(false);
    const completions = createMemo<SlashCommand[]>(() => {
        const p = autocompletePrefix();
        if (p === null || !props.getCompletions) return [];
        return props.getCompletions(p);
    });

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

    return (
        <div class="agent-footer">
            <div class="agent-input-container">
                <Show when={attachmentDraft}>
                    <AttachmentTray
                        draft={attachmentDraft!}
                        focusComposer={() => textareaRef?.focus()}
                        registerFocusLast={(fn) => (c.focusLastTile = fn)}
                    />
                </Show>
                <Show when={sendHint()}>
                    <div class="agent-attachment-send-hint" role="status">
                        {sendHint()}
                    </div>
                </Show>
                <Show when={historySearch() && historyAgent}>
                    <InputHistorySearch agentId={historyAgent!} onDone={(text) => {
                        setHistorySearch(false);
                        if (text !== undefined) setComposerValue(c, text);
                        textareaRef?.focus();
                    }} />
                </Show>
                <Show when={autocompletePrefix() !== null && completions().length > 0}>
                    <SlashAutocomplete
                        completions={completions()}
                        selectedIndex={autocompleteIndex()}
                        onHover={(idx) => setAutocompleteIndex(idx)}
                        onSelect={(cmd) => acceptCompletion(c, cmd)}
                    />
                </Show>
                <textarea
                    ref={textareaRef} data-raw-text-field="the prompt composer: a surface of its own that grows with its text and takes IME composition, paste and the history keys itself, not a form field"
                    class={`agent-input${isBangCmd() ? " bang-command" : ""}`} aria-label="Message the agent"
                    placeholder={placeholder()}
                    spellcheck={false}
                    onKeyDown={(e) => handleKeyDown(c, e)}
                    onInput={() => handleInput(c)}
                    onPaste={handlePaste}
                    onCompositionStart={() => {
                        c.composing = true;
                    }}
                    onCompositionEnd={() => {
                        c.composing = false;
                        // The value updated during the composition but slash matching was skipped; run it now.
                        updateAutocomplete(c);
                    }}
                    // The pane body's own onContextMenu (blockframe.tsx) only offers Copy-on-selection and
                    // never Paste (agent panes aren't `view: "term"`), so without this right-click shows a
                    // useless disabled-Copy menu (docs/specs/REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07.md).
                    onContextMenu={(e) =>
                        void showTextInputContextMenu(
                            e,
                            composerUndoItems(c),
                            attachmentDraft ? { onPasteAttachments: pasteAttachmentsFromMenu } : undefined,
                        )
                    }
                    rows={1}
                />
                {/* Pinned to the composer's right edge instead of the pane's header (moved off
                    blockframe.tsx, SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md).
                    Uses the same viewModel.voiceHandle() the header button used to, so voice wiring is unchanged. */}
                <Show when={props.viewModel?.voiceHandle}>
                    <div class="agent-input-mic">
                        <MicButton
                            blockId={props.viewModel!.blockId}
                            handle={props.viewModel!.voiceHandle!()}
                            paneTitle={`Speak into this agent (${shortcutFor("pane:voice")})`}
                        />
                    </div>
                </Show>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
