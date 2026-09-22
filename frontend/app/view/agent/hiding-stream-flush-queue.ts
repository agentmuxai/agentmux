// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `createHidingStreamFlushQueue` — the actual hiding mechanism for
 * SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md, resolving
 * the gap that spec's §3.3 flagged ("newly discovered, third pass"):
 * suppressing only the OUTGOING message isn't enough — the model's real
 * response to a hidden turn must not render either, or it leaks as a
 * visible assistant message with no visible prompt.
 *
 * Design: `stream-flush-queue.ts`'s own header comment establishes that
 * `StreamFlushQueue` is ALREADY the single, mandatory choke point every
 * document-write producer in `useAgentStream.ts` goes through — "give
 * every new producer a pushXxx method here instead of scheduling its own
 * flush." That existing discipline is exactly what makes hiding safe to
 * implement as ONE wrapper at this one interface, instead of scattering
 * `if (hiding) return` checks across every individual frame-type branch in
 * `useAgentStream.ts` (fragile — a missed branch is a silent leak; a single
 * choke point can't be partially bypassed by construction).
 *
 * Only the six PUSH methods are gated. Every read-only/lifecycle method
 * (`hasPendingNewOrUpdated`, `scheduleFlush`, `flushNow`, `resetAll`,
 * `resetNodeQueues`, `cancelScheduledFlush`) delegates unconditionally —
 * they carry no content to leak, and since nothing was ever pushed while
 * hiding, they naturally no-op on the real queue's empty pending arrays
 * rather than needing their own gate.
 */

import type { StreamFlushQueue } from "./stream-flush-queue";

/**
 * Wraps `real` so every push is silently swallowed whenever `isHiding()`
 * returns true at call time. `isHiding` is a closure, re-checked on every
 * call — never snapshotted — so the caller can flip a plain boolean live
 * across the lifetime of one hidden turn without reconstructing the queue.
 */
export function createHidingStreamFlushQueue(real: StreamFlushQueue, isHiding: () => boolean): StreamFlushQueue {
    return {
        pushNewNode(node) {
            if (isHiding()) return;
            real.pushNewNode(node);
        },
        pushUpdatedNode(node) {
            if (isHiding()) return;
            real.pushUpdatedNode(node);
        },
        pushToolChunk(toolId, chunk) {
            if (isHiding()) return;
            real.pushToolChunk(toolId, chunk);
        },
        pushShellCreate(node) {
            if (isHiding()) return;
            real.pushShellCreate(node);
        },
        pushShellChunk(shellId, chunk) {
            if (isHiding()) return;
            real.pushShellChunk(shellId, chunk);
        },
        pushShellExit(shellId, status, exitCode, exitedAt) {
            if (isHiding()) return;
            real.pushShellExit(shellId, status, exitCode, exitedAt);
        },

        // Always delegate — see module doc comment.
        hasPendingNewOrUpdated: () => real.hasPendingNewOrUpdated(),
        scheduleFlush: () => real.scheduleFlush(),
        flushNow: () => real.flushNow(),
        resetAll: () => real.resetAll(),
        resetNodeQueues: () => real.resetNodeQueues(),
        cancelScheduledFlush: () => real.cancelScheduledFlush(),
    };
}
