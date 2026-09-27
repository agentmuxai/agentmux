// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Drop file(s) onto an agent pane: the pane's file-drop hook
 * (SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3). The window-level
 * controller (app/drag/file-drop.ts) hit-tests, draws the indicator and
 * dispatches; this decides what the drop means for this pane.
 *
 * Files go to the composer's attachment tray: the backend copies and
 * processes them, and they're sent with the next message
 * (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md). Container agents can't
 * see the attachment store, and attachments can be turned off: then each file
 * is copied into the agent's working folder and an `@filename` token is
 * spliced into the composer (SPEC_PANE_FILE_DROP_2026_05_30.md §3.1, §3.6).
 * Without host paths (virtual files, a host without native file drop) the
 * files' bytes are used instead.
 */

import { onCleanup, onMount } from "solid-js";
import { copyIntoWorkdir, fileCount, notifyDrop, paneWorkdir, type CopySource } from "@/app/drag/file-drop-actions";
import { registerFileDropTarget, type FileDropHook } from "@/app/drag/file-drop";
import { getSettingsKeyAtom, MOS } from "@/app/store/global";
import { getAttachmentDraft } from "../attachments/attachment-draft";

interface Opts {
    blockId: string;
    /** The agent-view root; holds the composer the `@name` tokens go into. */
    rootRef: () => HTMLElement | undefined;
}

/**
 * Find the composer textarea inside the agent pane and splice the given
 * tokens at the current caret. If the textarea isn't mounted (rare race
 * during pane init), returns false so the caller can decide to queue.
 */
export function spliceComposerTokens(root: HTMLElement, tokens: string[]): boolean {
    const ta = root.querySelector<HTMLTextAreaElement>("textarea.agent-input");
    if (!ta) return false;
    const joined = tokens.join(" ");
    const caretAtStart = ta.selectionStart === 0;
    const before = ta.value.slice(0, ta.selectionStart);
    const after = ta.value.slice(ta.selectionEnd);
    // Pad with a leading space if the caret isn't at the very start AND the
    // preceding character isn't already whitespace, so "summarise" + "@x.csv"
    // doesn't read as "summarise@x.csv".
    const needsLead = !caretAtStart && before.length > 0 && !/\s$/.test(before);
    const insert = (needsLead ? " " : "") + joined + (after.startsWith(" ") ? "" : " ");
    const newVal = before + insert + after;
    const newCaret = before.length + insert.length;
    // Use the native setter so React/SolidJS reactive bindings observe the change.
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    if (setter) {
        setter.call(ta, newVal);
    } else {
        ta.value = newVal;
    }
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    ta.setSelectionRange(newCaret, newCaret);
    ta.focus();
    return true;
}

/** A container agent's pane: its agent sees only the bind-mounted working folder. */
export function isContainerPane(blockId: string): boolean {
    const block = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId));
    return block?.meta?.["agentMode"] === "container";
}

/** Register this agent pane as a file-drop target for as long as it's mounted. */
export function useAgentDropAttach(opts: Opts): void {
    const enabledAtom = getSettingsKeyAtom("dnd:enabled");
    const insertTokenAtom = getSettingsKeyAtom("dnd:agentinserttoken");
    const concurrencyAtom = getSettingsKeyAtom("dnd:concurrency");
    const attachmentsAtom = getSettingsKeyAtom("attachments:enabled");

    const enabled = () => (enabledAtom() ?? true) !== false;
    const insertToken = () => (insertTokenAtom() ?? true) !== false;
    const concurrency = () => {
        const v = concurrencyAtom();
        return typeof v === "number" && v > 0 ? v : undefined;
    };
    // Container panes copy into the working folder (bind-mounted at /workspace).
    const toTray = () => (attachmentsAtom() ?? true) !== false && !isContainerPane(opts.blockId);

    const copy = (source: CopySource) =>
        copyIntoWorkdir(opts.blockId, source, {
            paneKind: "agent pane",
            concurrency: concurrency(),
            mentionIn: insertToken() ? (opts.rootRef() ?? null) : null,
            splice: spliceComposerTokens,
        });

    const hook: FileDropHook = {
        accept(drag) {
            if (!enabled()) return { ok: false, reason: "File drop is turned off (dnd:enabled)" };
            const what = drag.count > 0 ? fileCount(drag.count) : "files";
            if (toTray()) return { ok: true, message: `Drop ${what} to attach`, icon: "fa-paperclip" };
            const cwd = paneWorkdir(opts.blockId);
            return cwd
                ? { ok: true, message: `Copy ${what} to ${cwd}`, icon: "fa-copy" }
                : { ok: false, reason: "No working folder for this agent" };
        },
        async drop({ paths, files }) {
            if (!toTray()) {
                await copy(paths.length > 0 ? { paths } : { files });
                return;
            }
            const draft = getAttachmentDraft(opts.blockId);
            if (paths.length === 0) {
                draft.uploadFiles(files);
                return;
            }
            try {
                // Anything the tray doesn't take (an older, images-only
                // backend's non-images) is copied like before.
                const rest = await draft.ingestPaths(paths);
                if (rest.length > 0) await copy({ paths: rest });
            } catch (err) {
                notifyDrop.attachFailed(err);
            }
        },
    };

    onMount(() => {
        const dispose = registerFileDropTarget(opts.blockId, hook);
        onCleanup(dispose);
    });
}
