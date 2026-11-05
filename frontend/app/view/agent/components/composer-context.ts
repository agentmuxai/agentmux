// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the composer's functions (composer-actions.ts, composer-keys.ts) share: the textarea, the
 * callbacks the pane passes, the history and Esc-undo state, and the signals the footer renders from.
 * `AgentFooter` builds one per instance; the functions take it first.
 */

import type { Accessor } from "solid-js";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { AttachmentDraft } from "../attachments/attachment-draft";
import type { SlashCommand } from "../commands/types";

/** The footer props the composer's functions call. Read at call time, so a prop swapped by the pane is seen. */
export interface ComposerCallbacks {
    onSendMessage?: (message: string, attachments?: AttachmentRef[]) => void | Promise<void>;
    onTyping?: () => void;
    onStopAgent?: () => void;
    onRecallLatestQueued?: () => { text: string; attachments?: AttachmentRef[] } | null;
}

export interface ComposerCtx {
    props: ComposerCallbacks;
    textarea: () => HTMLTextAreaElement | undefined;
    /** Undefined for callers that pass no viewModel; draft persistence is then a no-op. */
    draftBlockId: string | undefined;
    /** The defined agent whose history srv keeps, if any. */
    historyAgent: string | null;
    /** Files attached to the unsent message. */
    attachmentDraft: AttachmentDraft | undefined;
    /** Sent messages, oldest first: the agent's own, or this block's. `hist.pos === sentHistory.length` means "showing the live draft". */
    sentHistory: string[];
    hist: { pos: number; draft: string };
    /** Draft cleared by Esc, held for Undo. One level deep: Esc-clear is the only destructive programmatic edit. */
    escClearedDraft: string | null;
    /** True between compositionstart and compositionend. */
    composing: boolean;
    typingScrollPending: boolean;
    focusLastTile: (() => void) | undefined;
    autocompletePrefix: Accessor<string | null>;
    setAutocompletePrefix: (prefix: string | null) => void;
    autocompleteIndex: Accessor<number>;
    setAutocompleteIndex: (update: number | ((i: number) => number)) => void;
    completions: Accessor<SlashCommand[]>;
    setIsBangCmd: (bang: boolean) => void;
    setHistorySearch: (open: boolean) => void;
    showSendHint: (text: string) => void;
    /** What the placeholder shows, not the raw meta: a suggestion masked at send time is not offered. */
    visibleSuggestion: Accessor<string | undefined>;
    maskCurrentSuggestion: () => void;
}
