// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The message a pane's agent refused because the agent is open in another pane (`open_in_other_pane`).
 * A refused send is taken off the conversation, so without this the text would be gone once the
 * pane told the user why; Move here sends it, and the row shows what was not sent.
 *
 * Spec: docs/specs/SPEC_AGENT_SAME_PROCESS_DUPLICATE_PANE_RECOVERY_2026_10_01.md §4.5.
 */

import { createSignal } from "solid-js";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";

export interface RefusedSend {
    text: string;
    attachments: AttachmentRef[];
}

/** Whether `error` is srv's refusal because the agent is open in another pane of this AgentMux. */
export function isOpenInOtherPaneError(error: string): boolean {
    return error.includes("already open in another pane (block ");
}

const [refused, setRefused] = createSignal<ReadonlyMap<string, RefusedSend>>(new Map());

export function rememberRefusedSend(blockId: string, send: RefusedSend): void {
    setRefused((prev) => {
        // A second refusal adds to the first, so neither message is dropped; the kept message
        // resent and refused again stays as it is.
        const kept = prev.get(blockId);
        const merged =
            !kept || kept.text === send.text
                ? send
                : { text: [kept.text, send.text].join("\n\n"), attachments: [...kept.attachments, ...send.attachments] };
        return new Map(prev).set(blockId, merged);
    });
}

/** The message pane `blockId` last had refused, still waiting to be sent (reactive). */
export function refusedSendOf(blockId: string): RefusedSend | undefined {
    return refused().get(blockId);
}

export function forgetRefusedSend(blockId: string): void {
    setRefused((prev) => {
        if (!prev.has(blockId)) return prev;
        const next = new Map(prev);
        next.delete(blockId);
        return next;
    });
}
