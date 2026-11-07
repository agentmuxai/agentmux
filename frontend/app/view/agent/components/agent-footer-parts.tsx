// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent pane's composer, the views: what sits above the textarea, the textarea itself (uncontrolled, its
// events handed to the composer's functions) and the mic button, over the state of agent-footer-state.ts.

import { Show, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { MicButton } from "@/app/element/MicButton";
import { shortcutFor } from "@/app/keybindings";
import { AttachmentTray } from "../attachments/AttachmentTray";
import { SlashAutocomplete } from "./SlashAutocomplete";
import { InputHistorySearch } from "./InputHistorySearch";
import { acceptCompletion, composerUndoItems, handleInput, setComposerValue, updateAutocomplete } from "./composer-actions";
import { handleKeyDown } from "./composer-keys";
import type { AgentFooterProps, AgentFooterState } from "./agent-footer-state";

/** The attachment tray, the send hint, the history search and the slash dropdown, above the textarea. */
export function FooterOverlays(props: { s: AgentFooterState }): JSX.Element {
    const s = props.s;
    const c = s.c;
    return (
        <>
            <Show when={s.attachmentDraft}>
                <AttachmentTray
                    draft={s.attachmentDraft!}
                    focusComposer={() => s.focusTextarea()}
                    registerFocusLast={(fn) => (c.focusLastTile = fn)}
                />
            </Show>
            <Show when={s.sendHint()}>
                <div class="agent-attachment-send-hint" role="status">
                    {s.sendHint()}
                </div>
            </Show>
            <Show when={s.historySearch() && s.historyAgent}>
                <InputHistorySearch agentId={s.historyAgent!} onDone={(text) => {
                    s.setHistorySearch(false);
                    if (text !== undefined) setComposerValue(c, text);
                    s.focusTextarea();
                }} />
            </Show>
            <Show when={s.autocompletePrefix() !== null && s.completions().length > 0}>
                <SlashAutocomplete
                    completions={s.completions()}
                    selectedIndex={s.autocompleteIndex()}
                    onHover={(idx) => s.setAutocompleteIndex(idx)}
                    onSelect={(cmd) => acceptCompletion(c, cmd)}
                />
            </Show>
        </>
    );
}

/** The composer's textarea. */
export function FooterTextarea(props: { s: AgentFooterState }): JSX.Element {
    const s = props.s;
    const c = s.c;
    return (
        <textarea
            ref={s.setTextarea} data-raw-text-field="the prompt composer: a surface of its own that grows with its text and takes IME composition, paste and the history keys itself, not a form field"
            class={`agent-input${s.isBangCmd() ? " bang-command" : ""}`} aria-label="Message the agent"
            placeholder={s.placeholder()}
            spellcheck={false}
            onKeyDown={(e) => handleKeyDown(c, e)}
            onInput={() => handleInput(c)}
            onPaste={s.handlePaste}
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
                    s.attachmentDraft ? { onPasteAttachments: s.pasteAttachmentsFromMenu } : undefined,
                )
            }
            rows={1}
        />
    );
}

/** The mic button, pinned to the composer's right edge. */
export function FooterMic(props: { viewModel: NonNullable<AgentFooterProps["viewModel"]> }): JSX.Element {
    const vm = props.viewModel;
    return (
        <div class="agent-input-mic">
            <MicButton
                blockId={vm.blockId}
                handle={vm.voiceHandle!()}
                paneTitle={`Speak into this agent (${shortcutFor("pane:voice")})`}
            />
        </div>
    );
}
