// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's inbox for the transcript messages srv sends it.
 *
 * Each message used to be decoded, ordered and parsed in the callback of the message itself, so a
 * burst (several agents streaming, or a broadcast answered at once) ran as many separate tasks, none
 * of them held to the app-wide stream scheduler's per-frame caps (stream-scheduler.ts). The inbox
 * keeps the messages in arrival order and hands them on in one go, in one Solid batch, when the
 * scheduler runs the pane's turn in a frame. A pane the user is on is due the next frame; a pane
 * that is shown but not focused waits about 30 Hz; a pane on a hidden tab takes the slow one-a-frame
 * slot. Nothing is dropped and the order never changes: a pane that waits simply hands on more
 * messages the next time.
 *
 * The inbox decides when, not what: the messages go on to the transcript cursor (transcript-cursor.ts),
 * which still orders, dedupes and reads gaps.
 */

import { batch } from "solid-js";
import { createHidingStreamFlushQueue } from "./hiding-stream-flush-queue";
import type { StreamFlushQueue } from "./stream-flush-queue";
import { paneFlushRate } from "./stream-flush-rate";
import { cancelStreamFlush, requestStreamFlush, type FlushRate } from "./stream-scheduler";

export interface StreamInbox<T> {
    /** Keep `message` for the pane's next turn in a frame. */
    push(message: T): void;
    /** Hand on everything kept so far, now, in order. */
    flushNow(): void;
    /** Messages kept and not yet handed on. */
    size(): number;
    /** The pane is going away: what is kept is dropped and no turn stays requested. */
    dispose(): void;
}

/**
 * A tool's or shell's output chunks reach the pane by their own subscription and are dropped by the reducer when
 * the node they belong to is not in the document yet. That node comes from a transcript record, which the inbox may still
 * be keeping. So a chunk (or a shell's create or exit) first has the inbox hand on what it holds: a record that arrived
 * before the chunk is parsed before it, as it was when each message was parsed on arrival.
 */
export function withInboxFirst(queue: StreamFlushQueue, handOnKept: () => void): StreamFlushQueue {
    return {
        ...queue,
        pushToolChunk(toolId, chunk) {
            handOnKept();
            queue.pushToolChunk(toolId, chunk);
        },
        pushShellCreate(node) {
            handOnKept();
            queue.pushShellCreate(node);
        },
        pushShellChunk(shellId, chunk) {
            handOnKept();
            queue.pushShellChunk(shellId, chunk);
        },
        pushShellExit(shellId, status, exitCode, exitedAt) {
            handOnKept();
            queue.pushShellExit(shellId, status, exitCode, exitedAt);
        },
    };
}

/** What `createPaneInbox` hands the hook: the pane's flush queue (chunks hand the inbox on first) and the subscription. */
export interface PaneInbox<T> {
    queue: StreamFlushQueue;
    /** Subscribe to the block's file subject, keeping each message in the inbox for `deliver`. Returns the teardown. */
    subscribe(subject: { subscribe(fn: (msg: T) => void): { unsubscribe(): void } }, deliver: (msg: T) => void): () => void;
}

/**
 * One pane's inbox, made with its queue: `hide` gates every push as before (the memory reinjection's hidden turn),
 * the inbox drains at the pane's own rate (`paneFlushRate`) and writes what it parsed through `rawQueue` in the same turn.
 */
export function createPaneInbox<T>(blockId: string, rawQueue: StreamFlushQueue, hide: () => boolean): PaneInbox<T> {
    let inbox: StreamInbox<T> | undefined;
    const queue = createHidingStreamFlushQueue(withInboxFirst(rawQueue, () => inbox?.flushNow()), hide);
    return {
        queue,
        subscribe(subject, deliver) {
            inbox = createStreamInbox<T>(blockId, deliver, () => paneFlushRate(blockId), () => rawQueue.flushNow());
            const mine = inbox;
            const subscription = subject.subscribe((msg) => mine.push(msg));
            return () => {
                subscription.unsubscribe();
                mine.dispose();
                if (inbox === mine) inbox = undefined;
            };
        },
    };
}

let inboxSeq = 0;

/**
 * `deliver` gets each message in arrival order. `blockId` names the pane to the scheduler as
 * `<blockId>#inbox-<n>`, so work waiting here counts as the block's work waiting
 * (`hasPendingStreamFlushForBlock`). `rate` is read afresh each frame. `afterHandOn` runs once
 * after each hand-on (the pane's flush queue lands what the messages parsed into).
 */
export function createStreamInbox<T>(
    blockId: string,
    deliver: (message: T) => void,
    rate?: () => FlushRate,
    afterHandOn?: () => void,
): StreamInbox<T> {
    let kept: T[] = [];
    let disposed = false;
    const schedulerId = `${blockId}#inbox-${++inboxSeq}`;

    function handOn(): void {
        if (kept.length === 0) return;
        const batchOfMessages = kept;
        kept = [];
        batch(() => {
            for (const message of batchOfMessages) {
                try {
                    deliver(message);
                } catch (err) {
                    // One message's failure is reported (window.onerror and the render trail still see it)
                    // without losing the messages after it.
                    queueMicrotask(() => {
                        throw err;
                    });
                }
            }
            // What the messages produced lands in the same turn: one parse pass and one document write
            // per pane per frame, in one scheduler slot, not a second request that waits for another frame.
            afterHandOn?.();
        });
    }

    return {
        push(message) {
            if (disposed) return;
            kept.push(message);
            requestStreamFlush(schedulerId, handOn, rate);
        },
        flushNow() {
            cancelStreamFlush(schedulerId);
            handOn();
        },
        size: () => kept.length,
        dispose() {
            disposed = true;
            kept = [];
            cancelStreamFlush(schedulerId);
        },
    };
}
