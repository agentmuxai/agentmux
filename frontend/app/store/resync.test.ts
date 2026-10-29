// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ resync: vi.fn() }));
vi.mock("./mos", () => ({ resyncMuxObjects: h.resync }));

import { MIN_GAP_MS, QUIET_MS, __resetResync, noteEventsDroppedMessage } from "./resync";

const dropped = { command: "eventrecv", data: { event: "ws:eventsdropped", data: { dropped: 5 } } };

describe("noteEventsDroppedMessage", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        h.resync.mockReset().mockResolvedValue(0);
        __resetResync();
    });
    afterEach(() => vi.useRealTimers());

    it("re-fetches once, after the burst has gone quiet", async () => {
        for (let i = 0; i < 5; i++) noteEventsDroppedMessage(dropped);
        await vi.advanceTimersByTimeAsync(QUIET_MS - 1);
        expect(h.resync).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(2);
        expect(h.resync).toHaveBeenCalledTimes(1);
    });

    it("waits out the minimum gap before re-fetching again", async () => {
        noteEventsDroppedMessage(dropped);
        await vi.advanceTimersByTimeAsync(QUIET_MS + 1);
        expect(h.resync).toHaveBeenCalledTimes(1);

        noteEventsDroppedMessage(dropped);
        await vi.advanceTimersByTimeAsync(MIN_GAP_MS - 10);
        expect(h.resync).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(20);
        expect(h.resync).toHaveBeenCalledTimes(2);
    });

    it("ignores every other message", async () => {
        noteEventsDroppedMessage(null);
        noteEventsDroppedMessage({ command: "eventrecv", data: { event: "srvinfo" } });
        noteEventsDroppedMessage({ command: "message", data: { event: "ws:eventsdropped" } });
        await vi.advanceTimersByTimeAsync(MIN_GAP_MS);
        expect(h.resync).not.toHaveBeenCalled();
    });
});
