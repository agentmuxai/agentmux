// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ordered, paced `blockinput` sender for a terminal block.
 *
 * Extracted verbatim from TermViewModel.sendDataToController so the agent
 * pane's shell drawer gets the same large-paste handling as a terminal pane
 * (docs/specs/SPEC_AGENT_SHELL_DRAWER_CONTEXT_MENU_PASTE_AND_REGIONS_2026_09_25.md §3.4).
 *
 * Uses the `blockinput` wscommand, not the controllerinput RPC: blockinput is
 * processed inline & synchronously in the WS receive loop, so consecutive
 * keystrokes reach the PTY in TCP order. The RPC path is dispatched
 * concurrently by the engine and can reorder fast typing.
 */

import { pushNotification, removeNotificationById } from "@/app/store/flash-notifications";
import { sendWSCommand } from "@/app/store/ws";
import { sleep, stringToBase64 } from "@/util/util";

/** Max bytes per blockinput WebSocket frame. Keeps individual frames small
 *  enough to avoid PTY write-buffer saturation on Windows ConPTY and large
 *  single-write failures. Input above this is split and sent with a small
 *  inter-chunk delay to avoid flooding the backend channel. */
export const PASTE_CHUNK_BYTES = 4096;
export const PASTE_CHUNK_DELAY_MS = 5;
/** Chunked sends of at least this many KB show a "Pasting…" progress toast. */
const PASTE_TOAST_MIN_KB = 8;

/** Monotonic across all senders so overlapping pastes never share a toast id. */
let pasteSeq = 0;

export class BlockInputSender {
    // Serializes ALL input on this block while a chunked paste is in flight:
    // every blockinput frame is sent with `seq: None`, so PTY order == send
    // order. A chunked paste sleeps between chunks, so a keystroke or a second
    // paste arriving mid-paste must be queued behind it — otherwise it would
    // slip between chunks on the wire and corrupt the pasted content.
    private chain: Promise<void> = Promise.resolve();
    // >0 while chunked work is queued/running; gates the fast path below.
    private inFlight = 0;

    constructor(private readonly blockId: string) {}

    private sendSingleFrame(data: string) {
        const cmd: BlockInputWSCommand = {
            wscommand: "blockinput",
            blockid: this.blockId,
            inputdata64: stringToBase64(data),
        };
        sendWSCommand(cmd);
    }

    send(data: string) {
        const encoded = new TextEncoder().encode(data);
        const isLarge = encoded.length > PASTE_CHUNK_BYTES;
        if (!isLarge && this.inFlight === 0) {
            // Fast path: small input and no chunked send in flight → send now.
            this.sendSingleFrame(data);
            return;
        }
        // Either a large paste, OR small input that must NOT overtake an
        // in-flight chunked send. Queue on the chain to preserve wire order.
        // `.catch` keeps the chain alive if one item fails (a rejected promise
        // would otherwise skip every subsequent `.then`).
        this.inFlight++;
        this.chain = this.chain
            .then(() => (isLarge ? this.sendChunked(encoded) : this.sendSingleFrame(data)))
            .catch(() => {})
            .finally(() => {
                this.inFlight--;
            });
    }

    private async sendChunked(encoded: Uint8Array) {
        const decoder = new TextDecoder();
        const kb = Math.round(encoded.length / 1024);
        // Unique per paste so back-to-back/overlapping pastes don't share (and
        // prematurely dismiss) one another's progress toast.
        const toastId = `paste-progress-${this.blockId}-${++pasteSeq}`;
        const showToast = kb >= PASTE_TOAST_MIN_KB;
        if (showToast) {
            pushNotification({
                id: toastId,
                icon: "clipboard",
                title: "Pasting…",
                message: `Sending ${kb} KB in chunks`,
                timestamp: new Date().toISOString(),
                type: "info",
            });
        }
        // try/finally so the toast is always removed even if sendWSCommand throws
        // mid-paste — otherwise the "Pasting…" notification would leak forever.
        try {
            for (let offset = 0; offset < encoded.length; offset += PASTE_CHUNK_BYTES) {
                const slice = encoded.subarray(offset, offset + PASTE_CHUNK_BYTES);
                const isLast = offset + PASTE_CHUNK_BYTES >= encoded.length;
                // Stream-decode so a multi-byte UTF-8 character straddling a chunk
                // boundary is not split into two U+FFFD replacements: with
                // `{ stream: true }` the decoder defers the trailing partial bytes
                // to the next chunk. The concatenation of all chunks reconstructs
                // the original bytes exactly; the final chunk flushes (stream:false).
                const text = decoder.decode(slice, { stream: !isLast });
                if (text.length > 0) {
                    this.sendSingleFrame(text);
                }
                if (!isLast) {
                    await sleep(PASTE_CHUNK_DELAY_MS);
                }
            }
        } finally {
            if (showToast) {
                removeNotificationById(toastId);
            }
        }
    }
}
