// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createHidingStreamFlushQueue } from "./hiding-stream-flush-queue";
import type { StreamFlushQueue } from "./stream-flush-queue";

// A minimal fake matching the real StreamFlushQueue interface, with every
// method spied so tests can assert exactly what did/didn't reach it.
function fakeQueue(): StreamFlushQueue {
    return {
        pushNewNode: vi.fn(),
        pushUpdatedNode: vi.fn(),
        hasPendingNewOrUpdated: vi.fn(() => false),
        pushToolChunk: vi.fn(),
        pushShellCreate: vi.fn(),
        pushShellChunk: vi.fn(),
        pushShellExit: vi.fn(),
        scheduleFlush: vi.fn(),
        flushNow: vi.fn(),
        resetAll: vi.fn(),
        resetNodeQueues: vi.fn(),
        cancelScheduledFlush: vi.fn(),
    };
}

describe("createHidingStreamFlushQueue", () => {
    it("delegates every push method to the real queue when not hiding", () => {
        const real = fakeQueue();
        const wrapped = createHidingStreamFlushQueue(real, () => false);

        const node = { type: "markdown", id: "x", content: "hi", timestamp: 0 } as const;
        wrapped.pushNewNode(node);
        wrapped.pushUpdatedNode(node);
        wrapped.pushToolChunk("t1", { text: "x", timestamp: 0 } as never);
        wrapped.pushShellCreate({ type: "shell", id: "s1" } as never);
        wrapped.pushShellChunk("s1", { text: "x", timestamp: 0 } as never);
        wrapped.pushShellExit("s1", "exited-ok", 0, 0);

        expect(real.pushNewNode).toHaveBeenCalledWith(node);
        expect(real.pushUpdatedNode).toHaveBeenCalledWith(node);
        expect(real.pushToolChunk).toHaveBeenCalledTimes(1);
        expect(real.pushShellCreate).toHaveBeenCalledTimes(1);
        expect(real.pushShellChunk).toHaveBeenCalledTimes(1);
        expect(real.pushShellExit).toHaveBeenCalledTimes(1);
    });

    it("swallows every push method silently while hiding — the whole suppression mechanism", () => {
        const real = fakeQueue();
        const wrapped = createHidingStreamFlushQueue(real, () => true);

        const node = { type: "markdown", id: "x", content: "hi", timestamp: 0 } as const;
        wrapped.pushNewNode(node);
        wrapped.pushUpdatedNode(node);
        wrapped.pushToolChunk("t1", { text: "x", timestamp: 0 } as never);
        wrapped.pushShellCreate({ type: "shell", id: "s1" } as never);
        wrapped.pushShellChunk("s1", { text: "x", timestamp: 0 } as never);
        wrapped.pushShellExit("s1", "exited-ok", 0, 0);

        expect(real.pushNewNode).not.toHaveBeenCalled();
        expect(real.pushUpdatedNode).not.toHaveBeenCalled();
        expect(real.pushToolChunk).not.toHaveBeenCalled();
        expect(real.pushShellCreate).not.toHaveBeenCalled();
        expect(real.pushShellChunk).not.toHaveBeenCalled();
        expect(real.pushShellExit).not.toHaveBeenCalled();
    });

    it("re-checks the hiding predicate on every call, not just once at construction", () => {
        // isHiding is a closure the caller flips live — the wrapper must read
        // it fresh each call, not snapshot it when createHidingStreamFlushQueue
        // was built (which would make the whole mechanism a no-op after the
        // first read).
        const real = fakeQueue();
        let hiding = true;
        const wrapped = createHidingStreamFlushQueue(real, () => hiding);

        const node = { type: "markdown", id: "x", content: "hi", timestamp: 0 } as const;
        wrapped.pushNewNode(node);
        expect(real.pushNewNode).not.toHaveBeenCalled();

        hiding = false;
        wrapped.pushNewNode(node);
        expect(real.pushNewNode).toHaveBeenCalledTimes(1);
    });

    it("always delegates read-only and lifecycle methods, hiding or not — they carry no content to leak", () => {
        // hasPendingNewOrUpdated/scheduleFlush/flushNow/resetAll/
        // resetNodeQueues/cancelScheduledFlush are orthogonal to hiding: while
        // hiding, the real queue never received anything to flush (every push
        // was swallowed above), so delegating these is both safe and correct
        // — they naturally no-op on an empty queue rather than needing their
        // own gate.
        const real = fakeQueue();
        const wrapped = createHidingStreamFlushQueue(real, () => true);

        wrapped.hasPendingNewOrUpdated();
        wrapped.scheduleFlush();
        wrapped.flushNow();
        wrapped.resetAll();
        wrapped.resetNodeQueues();
        wrapped.cancelScheduledFlush();

        expect(real.hasPendingNewOrUpdated).toHaveBeenCalledTimes(1);
        expect(real.scheduleFlush).toHaveBeenCalledTimes(1);
        expect(real.flushNow).toHaveBeenCalledTimes(1);
        expect(real.resetAll).toHaveBeenCalledTimes(1);
        expect(real.resetNodeQueues).toHaveBeenCalledTimes(1);
        expect(real.cancelScheduledFlush).toHaveBeenCalledTimes(1);
    });
});
