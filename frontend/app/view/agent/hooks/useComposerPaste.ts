// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Files into the agent composer: Ctrl/Cmd+V with files, and Paste from the right-click menu. They go to the
 * attachment tray, or, in a container pane or with `attachments:enabled` off, into the working folder as
 * `@name` mentions (the same copy a drop makes).
 */

import { copyIntoWorkdir, type CopySource } from "@/app/drag/file-drop-actions";
import { getSettingsKeyAtom, pushNotification } from "@/app/store/global";
import type { AttachmentDraft } from "../attachments/attachment-draft";
import { isContainerPane, spliceComposerTokens } from "./useAgentDropAttach";

export function useComposerPaste(opts: {
    blockId: string | undefined;
    draft: AttachmentDraft | undefined;
    textarea: () => HTMLTextAreaElement | undefined;
}): { handlePaste: (e: ClipboardEvent) => void; pasteAttachmentsFromMenu: (paths: string[]) => void } {
    const { blockId, draft } = opts;
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
    const inContainer = () => !!blockId && isContainerPane(blockId);
    // `attachments:enabled` off means files go to the working folder instead
    // of the tray, for a paste exactly as for a drop (useAgentDropAttach).
    const attachmentsSetting = getSettingsKeyAtom("attachments:enabled");
    const pasteToWorkdir = () => inContainer() || attachmentsSetting() === false;
    // The same copy a drop makes, notices and @mentions included. Paste
    // ignores `dnd:*`.
    const pasteIntoWorkdir = (source: CopySource) =>
        copyIntoWorkdir(blockId!, source, {
            paneKind: "agent pane",
            // Any ancestor holding the composer's textarea will do.
            mentionIn: opts.textarea()?.parentElement ?? null,
            splice: spliceComposerTokens,
        });

    const pasteAttachmentsFromMenu = (paths: string[]) => {
        if (!draft) return;
        if (pasteToWorkdir()) {
            void pasteIntoWorkdir({ paths });
            return;
        }
        void draft.ingestPaths(paths).catch(pasteFailed);
    };

    const handlePaste = (e: ClipboardEvent) => {
        if (!draft) return;
        const files = Array.from(e.clipboardData?.files ?? []);
        if (files.length === 0) return;
        if (pasteToWorkdir()) {
            void pasteIntoWorkdir({ files });
            return;
        }
        draft.uploadFiles(files);
    };
    return { handlePaste, pasteAttachmentsFromMenu };
}
