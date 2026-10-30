// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFooter - Minimal Claude Code-style input
 */


import { Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { getVoiceSession, type PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { markEnd, markStart } from "@/perf";
import { getSettingsKeyAtom, pushNotification } from "@/app/store/global";
import { focusManager } from "@/app/store/focusManager";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { MicButton } from "@/app/element/MicButton";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { AgentViewModel } from "../agent-model";
import { focusComposerWhenReady, takeComposerFocusRequest } from "../composer-focus";
import type { SlashCommand } from "../commands/types";
import { SlashAutocomplete } from "./SlashAutocomplete";
import { isBangCommand } from "../bang-command";
import { caretAtSelectionEdge } from "./composer-caret";
import { InputHistorySearch } from "./InputHistorySearch";
import { AttachmentTray } from "../attachments/AttachmentTray";
import { getAttachmentDraft } from "../attachments/attachment-draft";
import { useComposerRestore } from "../composer-restore";
import { getComposerDraft, getSentHistory, saveSentHistory, SENT_HISTORY_MAX, setComposerDraft } from "../reload-persistence";
import { agentDraft, agentHistory, historyAgentId, loadOlderAgentHistory, movedCursor, rememberSent, saveAgentDraft, watchAgentHistory } from "../agent-input-history";
import { attachmentNoun, fileKind } from "../attachments/file-kind";
import { isContainerPane, spliceComposerTokens } from "../hooks/useAgentDropAttach";
import { copyIntoWorkdir, type CopySource } from "@/app/drag/file-drop-actions";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { shortcutFor } from "@/app/keybindings";
import { META_SUGGESTION, META_SUGGESTION_GEN } from "@/app/store/meta-keys";
import { MENU_ICON } from "@/app/util/menu-icons";


// \u2500\u2500 Composer draft persistence \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
// Module-level (not per-component-instance), keyed by blockId, so a draft
// survives this component's own unmount \u2014 which now happens on every
// ordinary pane-stack tab switch (opening/closing Agent History, switching
// forks, the "+" new-tab button), not just when the pane truly closes. The
// textarea below is deliberately UNCONTROLLED (DOM owns the value, for perf
// \u2014 see AgentFooter's own comment on textareaRef), so its value is destroyed
// outright on unmount with nothing to restore from unless something outside
// the component's own lifetime holds a copy. Cleared only on send; NOT
// cleared in onCleanup, since every tab-switch-away is itself an unmount
// from this component's own vantage point and clearing there would erase
// the very draft this exists to preserve. A stale entry for a since-closed
// block is a bounded, harmless leak (one short string per block ever
// opened this session). It lives in reload-persistence.ts
// (getComposerDraft/setComposerDraft), which also keeps it across a reload.
// Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md \u00a73.4.


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
    // docs/specs/SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md §3.4.
    let textareaRef: HTMLTextAreaElement | undefined;

    // Stable for this component instance's whole lifetime (component body
    // runs once) — undefined only for callers that don't pass a viewModel
    // (older/test callers, per the prop's own doc comment), in which case
    // draft persistence below is a no-op rather than an error.
    const draftBlockId = props.viewModel?.blockId;
    // Files attached to the unsent message (SPEC_AGENT_PANE_IMAGE_ATTACHMENTS
    // _2026_09_26.md §5, SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md).
    // Module-level per block, like the composer draft.
    const attachmentDraft = draftBlockId ? getAttachmentDraft(draftBlockId) : undefined;
    let focusLastTile: (() => void) | undefined;
    const [sendHint, setSendHint] = createSignal<string | null>(null);
    let sendHintTimer: ReturnType<typeof setTimeout> | undefined;
    const showSendHint = (text: string) => {
        setSendHint(text);
        clearTimeout(sendHintTimer);
        sendHintTimer = setTimeout(() => setSendHint(null), 3000);
    };
    onCleanup(() => clearTimeout(sendHintTimer));

    // Ctrl/Cmd+V with files (a screenshot, or files copied in Explorer or
    // Finder): attach them. Never preventDefault — a textarea's default paste
    // only ever inserts text, so the browser still pastes any text part
    // natively (selection handling and undo intact). Spec §6.2.
    // Right-click Paste has no paste event: the native clipboard reader
    // hands over paths (copied files, or a temp file for image data), and
    // they go through the same pipeline as a drop. Spec §5.6.
    // Container panes' agents can't see the attachment store: their pasted
    // files are copied into the working folder and mentioned as `@name`, as
    // a drop does there (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §7).
    const pasteFailed = (err: unknown) => {
        // Same notice as a failed drop (useAgentDropAttach.ts).
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: "Couldn't attach the files",
            message: String((err as Error)?.message ?? err),
            timestamp: new Date().toISOString(),
            type: "warning",
            expiration: Date.now() + 8000,
        });
    };
    const inContainer = () => !!draftBlockId && isContainerPane(draftBlockId);
    // `attachments:enabled` off means files go to the working folder instead
    // of the tray, for a paste exactly as for a drop (useAgentDropAttach).
    const attachmentsSetting = getSettingsKeyAtom("attachments:enabled");
    const pasteToWorkdir = () => inContainer() || attachmentsSetting() === false;
    // The same copy a drop makes, notices and @mentions included. Paste
    // ignores `dnd:*`.
    const pasteIntoWorkdir = (source: CopySource) =>
        copyIntoWorkdir(draftBlockId!, source, {
            paneKind: "agent pane",
            // Any ancestor holding the composer's textarea will do.
            mentionIn: textareaRef?.parentElement ?? null,
            splice: spliceComposerTokens,
        });

    const pasteAttachmentsFromMenu = (paths: string[]) => {
        if (!attachmentDraft) return;
        if (pasteToWorkdir()) {
            void pasteIntoWorkdir({ paths });
            return;
        }
        void attachmentDraft.ingestPaths(paths).catch(pasteFailed);
    };

    const handlePaste = (e: ClipboardEvent) => {
        if (!attachmentDraft) return;
        const files = Array.from(e.clipboardData?.files ?? []);
        if (files.length === 0) return;
        if (pasteToWorkdir()) {
            void pasteIntoWorkdir({ files });
            return;
        }
        attachmentDraft.uploadFiles(files);
    };

    // ── Sent-message history (shell-style ArrowUp / ArrowDown recall) ──
    // Per block, capped, kept like the draft (reload-persistence.ts): it
    // survives a pane-tab switch and a reload. The cursor and stash are
    // closure `let`s for the footer's lifetime. `histPos === sentHistory.length` means
    // "not navigating — showing the live draft"; lower values point at a prior
    // sent message. `histDraft` stashes the in-progress text while navigating
    // so ArrowDown past the newest entry restores it.
    //
    // A defined agent's history is the agent's own, kept by srv
    // (agent-input-history.ts): it survives a restart, an upgrade and a new
    // pane. It loads after mount, pages back on demand and takes sends from
    // other panes, so the cursor moves with what arrives around it.
    const historyAgent = historyAgentId(props.viewModel?.blockAtom()?.meta);
    const sentHistory: string[] = historyAgent
        ? agentHistory(historyAgent)
        : draftBlockId
          ? getSentHistory(draftBlockId)
          : [];
    let histPos = sentHistory.length;
    let histDraft = "";
    const [historySearch, setHistorySearch] = createSignal(false); // Ctrl+R
    if (historyAgent) {
        onCleanup(
            watchAgentHistory(historyAgent, (where, n, at) => {
                histPos = movedCursor(histPos, sentHistory.length, where, n, at);
            })
        );
    }

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
    // so it flips the instant either changes. term:next_prompt_suggestion is
    // NOT cleared by editing the composer (handleInput/handleKeyDown below
    // deliberately leave it alone) — typing over it and deleting back to
    // empty shows the same suggestion again, rather than falling through to
    // band 3. It's cleared only by an actual new turn starting or the
    // session ending (useNextPromptSuggestion.ts's own guards). See
    // docs/specs/SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md.
    const voice = getVoiceSession();
    // Masks a specific stale WRITE (not a text value) from band 1 — set by
    // handleSend right as it synchronously empties the composer. Without
    // this, every send has a real window where the box is empty (so the
    // native placeholder wants to render) but the PREVIOUS turn's
    // suggestion is still sitting in meta, since useNextPromptSuggestion.ts
    // guard 1 clears it via an async fire-and-forget RPC, not synchronously
    // with the send — the old suggestion would briefly flash as placeholder
    // text before that RPC round-trips.
    //
    // Compares term:next_prompt_suggestion_gen, a monotonic counter
    // useNextPromptSuggestion.ts bumps on every write to
    // term:next_prompt_suggestion (fresh or cleared) — NOT the suggestion
    // text itself. An earlier version of this compared text values
    // directly: if a later turn's genuinely fresh suggestion happened to be
    // the exact same string masked at the previous send (a plausible
    // repeat, e.g. Haiku predicting "Run the tests" twice), value-equality
    // would suppress a legitimate current suggestion until the next send —
    // reagentx P1 on #2515, caught on re-review. The generation counter
    // can't collide that way: any real write (same text or not) bumps it.
    //
    // A signal, not a plain variable: the placeholder memo below must
    // actually re-run the instant this is set, and mutating a bare closure
    // variable doesn't trigger Solid's reactivity at all (a still-earlier
    // version of this fix did exactly that and silently never worked —
    // caught by its own regression test, not by inspection).
    const [suggestionGenMaskedAtSend, setSuggestionGenMaskedAtSend] = createSignal<number | undefined>(undefined);
    // The suggestion as the composer shows it: absent while masked. Tab/→ accept
    // reads this too, so a suggestion hidden at send time can't be inserted
    // while its clear is still on its way.
    const visibleSuggestion = createMemo((): string | undefined => {
        const meta = props.viewModel?.blockAtom()?.meta;
        const suggestion = meta?.[META_SUGGESTION] as string | undefined;
        const suggestionGen = meta?.[META_SUGGESTION_GEN] as number | undefined;
        return suggestion && suggestionGen !== suggestionGenMaskedAtSend() ? suggestion : undefined;
    });
    const placeholder = createMemo(() => {
        const vm = props.viewModel;
        const suggestion = visibleSuggestion();
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

    const writeComposerValue = (text: string): void => {
        if (!textareaRef) return;
        textareaRef.value = text;
        // Every programmatic composer write goes through here (Esc-clear,
        // undo-restore, history recall, ghost-text accept, autocomplete
        // accept, voice) — persisting here, not just in handleInput below,
        // means e.g. an Esc-clear-to-empty also clears the persisted draft
        // instead of resurrecting the pre-clear text on the next tab
        // switch back. See the composer draft note above.
        if (draftBlockId) setComposerDraft(draftBlockId, text);
        if (historyAgent) saveAgentDraft(historyAgent, text);
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
    const setComposerValue = (text: string, caret: "start" | "end" = "end"): void => {
        if (!textareaRef) return;
        // Consumes the Esc-cleared snapshot — the undo-restore call below
        // reads `escClearedDraft` as its `text` argument BEFORE this runs, so
        // nulling it here doesn't affect that write. Every other caller
        // (ghost-text accept, history recall) is a fresh edit that should
        // supersede a stale snapshot the same way typing does.
        escClearedDraft = null;
        writeComposerValue(text);
        const pos = caret === "start" ? 0 : text.length;
        textareaRef.setSelectionRange(pos, pos);
        setIsBangCmd(isBangCommand(text));
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
        // Persist the draft (see the composer draft note above) so it survives a
        // tab-switch-away/back. Synchronous, not RAF-batched like the
        // typing-scroll below — a Map.set() is O(1) with no layout/DOM
        // cost, so it needs no debouncing, and must not depend on
        // `props.onTyping` being provided (that callback is optional; the
        // draft guarantee isn't).
        if (draftBlockId && textareaRef) setComposerDraft(draftBlockId, textareaRef.value);
        if (historyAgent && textareaRef) saveAgentDraft(historyAgent, textareaRef.value);
        // Perf marks per SPEC_INPUT_RESPONSIVENESS §7.1. Target: handler
        // body P95 < 5 ms. The mark span ends BEFORE the RAF enqueue so
        // we measure only the synchronous handler cost.
        markStart("agent-keystroke");
        // Deliberately does NOT clear term:next_prompt_suggestion here.
        // Typing over the ghost text (or deleting back to empty) no longer
        // dismisses it for good — the suggestion in block meta is untouched
        // by editing, and is cleared only by the two lifecycle events that
        // own that responsibility: a new turn starting (guard 1,
        // useNextPromptSuggestion.ts) and session end (guard 4, same file).
        // The native <textarea placeholder> already only renders on an
        // empty value, so hiding it while the user types needs no code here
        // at all — and leaving the meta alone means deleting back to empty
        // shows the same suggestion again instead of falling through to
        // "Send message to <agent>...". See
        // docs/specs/SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md
        // §2 for why the RPC-write race this used to guard against is
        // already independently closed by isComposerEmptyRef below, not by
        // clearing meta on every first keystroke.
        updateAutocomplete();
        setIsBangCmd(isBangCommand(textareaRef?.value ?? ""));
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
    useComposerRestore(draftBlockId, () => textareaRef, setComposerValue);

    // Restore a draft left behind by a prior mount of this same block (a
    // tab switch away and back — see the composer draft note above). Direct .value
    // write, matching every other programmatic composer write in this file
    // (writeComposerValue) — deliberately does NOT dispatch `input`, so it
    // doesn't trip handleInput's history-cursor reset before the user has
    // touched anything.
    onMount(() => {
        if (!textareaRef) return;
        const restore = (draft: string | undefined) => {
            if (!draft || !textareaRef || textareaRef.value) return;
            textareaRef.value = draft;
            setIsBangCmd(isBangCommand(draft));
            updateAutocomplete();
        };
        const draft = draftBlockId ? getComposerDraft(draftBlockId) : undefined;
        if (draft) restore(draft);
        // After a restart or in a new pane, the agent's own draft, if the
        // composer is still empty when it arrives.
        else if (historyAgent) void agentDraft(historyAgent).then(restore);
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

    // ── Focus handle ──────────────────────────────────────────────────
    // Registers the live textarea on the AgentViewModel so giveFocus() (the
    // shared pane-selection focus contract — see focusManager.ts) can
    // actually focus it instead of the `return false` stub it used to be.
    // SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §3a: same ref-handoff shape
    // as the voice handle above, which is the precedent this follows.
    //
    // Also claims focus immediately if this pane is ALREADY the active
    // tab's selected pane at the moment the textarea becomes available —
    // covers a freshly created agent pane (mounts after already being
    // marked focused) and switching into a tab whose agent pane wasn't
    // mounted yet, neither of which `focusManager.refocusNode()`'s own
    // one-shot attempt can reach.
    //
    // Right after a launch (a one-shot request from the launch path) the
    // claim is replaced by a retrying one: a launch started from a modal
    // mounts this footer while the modal is still open, which
    // claimFocusOnMount deliberately skips, so it would never land.
    // SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3.
    onMount(() => {
        const vm = props.viewModel;
        const ta = textareaRef;
        if (!vm || !ta) return;
        vm.focusTargetRef.current = ta;
        const cancelLaunchFocus = takeComposerFocusRequest(vm.blockId) ? focusComposerWhenReady(ta, vm.blockId) : null;
        onCleanup(() => {
            cancelLaunchFocus?.();
            if (vm.focusTargetRef.current === ta) {
                vm.focusTargetRef.current = null;
            }
        });
        if (!cancelLaunchFocus) focusManager.claimFocusOnMount(vm.blockId, () => vm.giveFocus());
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
            icon: MENU_ICON.undo,
            enabled: escClearedDraft != null || (textareaRef?.value.length ?? 0) > 0,
            click: undoComposer,
        },
    ];

    const handleSend = () => {
        if (!textareaRef) return;
        const message = textareaRef.value;
        const processing = attachmentDraft?.processingCount() ?? 0;
        if (processing > 0) {
            const kinds = attachmentDraft!
                .items()
                .filter((i) => i.status === "processing")
                .map((i) => fileKind(i.info?.kind, i.name));
            showSendHint(`Waiting for ${processing} ${attachmentNoun(processing, kinds)} to finish processing`);
            return;
        }
        const attachments = attachmentDraft?.refs() ?? [];
        if (!message.trim() && attachments.length === 0) return;
        // The pane can show before its conversation has loaded (usePaneReveal's
        // reveal bound). A send then is refused by the init guard
        // (useAgentCommands) after this handler has cleared the draft, so the
        // message would be lost: keep it and say why (#4374).
        if (draftBlockId && paneSnapshot(draftBlockId)?.initPhase.kind === "InitPending") {
            showSendHint("Waiting for the conversation to load");
            return;
        }
        if (props.onSendMessage) {
            // agent-submit span per SPEC_INPUT_RESPONSIVENESS §7.1. Includes
            // the synchronous onSendMessage cost (WS send, slice dispatch).
            markStart("agent-submit");
            // Plain messages keep the one-argument call.
            if (attachments.length > 0) {
                props.onSendMessage(message, attachments);
                attachmentDraft?.clearAfterSend();
            } else {
                props.onSendMessage(message);
            }
            // Record in shell-style history (skip a consecutive duplicate) and
            // reset the navigation cursor back to the live (now empty) draft.
            if (historyAgent) {
                rememberSent(historyAgent, message);
            } else if (message.trim() && message !== sentHistory[sentHistory.length - 1]) {
                sentHistory.push(message);
                if (sentHistory.length > SENT_HISTORY_MAX) sentHistory.shift();
                if (draftBlockId) saveSentHistory(draftBlockId);
            }
            histPos = sentHistory.length;
            histDraft = "";
            // Mask whatever write is current in meta right now — see the
            // doc comment on suggestionGenMaskedAtSend above `placeholder`.
            // Order doesn't matter relative to writeComposerValue below
            // (that call doesn't touch any signal `placeholder` reads
            // either), but setting the signal is what actually makes
            // `placeholder` recompute — a plain variable write here would
            // silently do nothing.
            setSuggestionGenMaskedAtSend(
                props.viewModel?.blockAtom()?.meta?.[META_SUGGESTION_GEN] as number | undefined
            );
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
        // Backspace at the very start of the text, with nothing selected,
        // moves into the attachment tray (Slack's pattern) instead of doing
        // nothing. Spec §5.9.
        if (
            e.key === "Backspace" &&
            textareaRef &&
            textareaRef.selectionStart === 0 &&
            textareaRef.selectionEnd === 0 &&
            (attachmentDraft?.count() ?? 0) > 0 &&
            focusLastTile
        ) {
            e.preventDefault();
            focusLastTile();
            return;
        }
        // PageUp/PageDown: this textarea is a flex sibling of
        // .agent-document-scroll-region, not a descendant of it (see
        // .agent-composer-region in styles/_control-bar.scss) — and
        // .agent-view itself is overflow:hidden (agent-view.scss). So the
        // scrollable chat history (.agent-document) is never an ancestor of
        // this element. Left unhandled, the browser's default Page Up/Down
        // walks PAST the pane looking for the nearest scrollable ancestor
        // and scrolls whatever it finds outside .agent-view instead —
        // which reads as the whole chat pane flying off screen.
        //
        // EXCEPTION (codex P2 on PR #3190): a long draft grows the textarea
        // up to its own 200px cap, past which `.agent-input` becomes its own
        // scroll container (`max-height: 200px; overflow-y: auto` in
        // _pending-footer.scss). In that state the textarea itself IS the
        // nearest scrollable ancestor of the caret, and PageUp/PageDown
        // should page/scroll within it exactly like any other overflowing
        // textarea — swallowing the key here would silently break paging
        // through a long pasted prompt. Only intercept when the textarea has
        // nothing of its own left to scroll.
        if (e.key === "PageUp" || e.key === "PageDown") {
            const el = textareaRef;
            // This read isn't preceded by a style write in this handler
            // (unlike the typing/auto-grow path SPEC_INPUT_RESPONSIVENESS_
            // TERMINAL_AND_AGENT_2026_05_29.md §4 guards against), and
            // PageUp/PageDown fire far less often than every keystroke, so
            // it isn't the layout-thrashing pattern that rule targets. The
            // decision has to be synchronous — deferring to a rAF read
            // would make preventDefault() too late.
            if (el && el.scrollHeight > el.clientHeight) { // perf:allow-layout-read — see comment above, not the typing hot path
                return;
            }
            e.preventDefault();
            return;
        }
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
            // What the placeholder shows, not the raw meta: a suggestion masked
            // at send time is not offered.
            const suggestion = visibleSuggestion();
            if (suggestion) {
                e.preventDefault();
                setComposerValue(suggestion);
                // Deliberately does NOT clear term:next_prompt_suggestion —
                // same reasoning as handleInput above (§3.3 of
                // SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md):
                // accepting via Tab and then deleting the accepted text
                // should behave the same as typing it by hand and deleting
                // it, both ending in the same empty-box state. Cleared only
                // by the two lifecycle guards in useNextPromptSuggestion.ts.
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
        // forward toward the live draft. ArrowUp gates on the active selection
        // edge being at true absolute position 0 — not merely "on the first
        // visual line" — so multiline editing of a recalled message (or
        // extending/shrinking a selection with Shift held) still moves within
        // the text normally, and only crosses into history once there's truly
        // nowhere left to move. Position 0 has no ambiguity for ArrowUp: it's
        // always the very first character, regardless of how many lines
        // follow, so every recall parks the caret there — letting a repeated
        // ArrowUp step back continuously with one press per level.
        //
        // ArrowDown is NOT simply the mirror of that (gating on true end):
        // position 0 is only "nothing left below" when the entry is
        // single-line — a multi-line entry can have line two sitting right
        // below an untouched position 0, which native ArrowDown still needs
        // to reach. So ArrowDown fires at the true end (always safe, any line
        // count) OR at true position 0 when the currently-displayed entry has
        // no embedded newline — since most sent messages are single-line,
        // this still lets a repeated ArrowDown step forward continuously
        // right after an ArrowUp landed the caret at the front, without
        // stranding a multi-line entry's later lines behind an early jump.
        // The one placement exception either way: landing back on the live
        // draft, past the newest entry, parks the caret at the END instead of
        // the front — it's the user's own in-progress text to resume typing,
        // not a historical entry still being reviewed.
        if (historyAgent && e.key === "r" && e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey) {
            e.preventDefault();
            setHistorySearch(true);
            return;
        }
        if (textareaRef && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
            const empty = textareaRef.value.length === 0;
            const navigating = histPos < sentHistory.length;
            const { first: caretAtStart, last: caretAtEnd } = caretAtSelectionEdge(textareaRef);
            const currentIsSingleLine = !textareaRef.value.includes("\n");

            // Empty composer: ArrowUp un-queues a held message before history.
            if (e.key === "ArrowUp" && empty) {
                const recalled = props.onRecallLatestQueued?.();
                if (recalled) {
                    e.preventDefault();
                    setComposerValue(recalled.text);
                    if (recalled.attachments?.length) void attachmentDraft?.restoreRefs(recalled.attachments);
                    return;
                }
            }

            // Older: whenever the active selection edge is at true position 0
            // (so we don't fight multiline cursor movement OR an in-progress
            // Shift+ArrowUp selection that hasn't reached the start yet).
            // Covers an empty composer, a partially typed draft (stashed into
            // histDraft, restorable with ArrowDown), and continuing further
            // back while already navigating. Caret lands back at position 0
            // (not the end) so a repeated ArrowUp immediately steps back
            // again instead of first having to re-reach the start.
            // Near the oldest entry shown, read the page before it.
            if (e.key === "ArrowUp" && historyAgent && histPos <= 10 && caretAtStart) {
                void loadOlderAgentHistory(historyAgent);
            }
            if (e.key === "ArrowUp" && histPos > 0 && caretAtStart) {
                if (!navigating) histDraft = textareaRef.value; // stash the live draft
                histPos--;
                e.preventDefault();
                setComposerValue(sentHistory[histPos], "start");
                return;
            }

            // Newer: only while navigating, and only once there's truly
            // nowhere left below — the true end always qualifies; position 0
            // also qualifies, but ONLY for a single-line entry (see the block
            // comment above for why a multi-line entry can't use this
            // shortcut). Past the newest entry, lands on the live draft with
            // the caret at the END instead of the front.
            if (e.key === "ArrowDown" && navigating && (caretAtEnd || (caretAtStart && currentIsSingleLine))) {
                histPos++;
                e.preventDefault();
                const atLiveDraft = histPos >= sentHistory.length;
                setComposerValue(atLiveDraft ? histDraft : sentHistory[histPos], atLiveDraft ? "end" : "start");
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
                <Show when={attachmentDraft}>
                    <AttachmentTray
                        draft={attachmentDraft!}
                        focusComposer={() => textareaRef?.focus()}
                        registerFocusLast={(fn) => (focusLastTile = fn)}
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
                        if (text !== undefined) setComposerValue(text);
                        textareaRef?.focus();
                    }} />
                </Show>
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
                    onPaste={handlePaste}
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
                    onContextMenu={(e) =>
                        void showTextInputContextMenu(
                            e,
                            composerUndoItems(),
                            attachmentDraft ? { onPasteAttachments: pasteAttachmentsFromMenu } : undefined,
                        )
                    }
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
                            paneTitle={`Speak into this agent (${shortcutFor("pane:voice")})`}
                        />
                    </div>
                </Show>
            </div>
        </div>
    );
};

AgentFooter.displayName = "AgentFooter";
