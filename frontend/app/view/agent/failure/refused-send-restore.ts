// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Give a pane back the message it had refused (`open_in_other_pane`) when nothing will send it: the
 * failure row was dismissed, so Move here is gone. The text goes back into the composer, after what
 * the user has typed since, and its attachments back into the tray, so a message the pane took off
 * the conversation is never lost. The text comes last: the tray takes a moment to look its files up, and a
 * message sent from the composer before then would go out without them.
 */

import { getAttachmentDraft } from "../attachments/attachment-draft";
import { restoreToComposer } from "../composer-restore";
import { forgetRefusedSend, refusedSendOf } from "./refused-send";

export async function restoreRefusedSend(blockId: string): Promise<void> {
    const kept = refusedSendOf(blockId);
    if (!kept) return;
    forgetRefusedSend(blockId);
    // AgentMux's own document is not the person's to edit and send as theirs.
    if (kept.system) return;
    if (kept.attachments.length > 0) await getAttachmentDraft(blockId).restoreRefs(kept.attachments);
    restoreToComposer(blockId, kept.text);
}
