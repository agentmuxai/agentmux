// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Give a pane back the message it had refused (`open_in_other_pane`) when nothing will send it: the
 * failure row was dismissed, so Move here is gone. The text goes back into the composer, after what
 * the user has typed since, and its attachments back into the tray, so a message the pane took off
 * the conversation is never lost.
 */

import { getAttachmentDraft } from "../attachments/attachment-draft";
import { restoreToComposer } from "../composer-restore";
import { forgetRefusedSend, refusedSendOf } from "./refused-send";

export function restoreRefusedSend(blockId: string): void {
    const kept = refusedSendOf(blockId);
    if (!kept) return;
    forgetRefusedSend(blockId);
    restoreToComposer(blockId, kept.text);
    if (kept.attachments.length > 0) void getAttachmentDraft(blockId).restoreRefs(kept.attachments);
}
