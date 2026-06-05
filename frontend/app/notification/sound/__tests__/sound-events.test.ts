// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import {
    __resetSoundListeners,
    notify,
    subscribeSoundEvents,
    type SoundEvent,
} from "../sound-events";

describe("sound-events bus", () => {
    afterEach(() => {
        __resetSoundListeners();
    });

    it("delivers a notify() to every subscriber", () => {
        const a = vi.fn();
        const b = vi.fn();
        subscribeSoundEvents(a);
        subscribeSoundEvents(b);

        notify("agent.turn.complete", { sourceBlockId: "blk-1" });

        expect(a).toHaveBeenCalledTimes(1);
        expect(b).toHaveBeenCalledTimes(1);
        const ev: SoundEvent = a.mock.calls[0][0];
        expect(ev.id).toBe("agent.turn.complete");
        expect(ev.sourceBlockId).toBe("blk-1");
    });

    it("unsubscribe stops delivery", () => {
        const a = vi.fn();
        const unsub = subscribeSoundEvents(a);
        notify("agent.turn.complete");
        unsub();
        notify("agent.turn.complete");
        expect(a).toHaveBeenCalledTimes(1);
    });

    it("a throwing listener does not poison the others", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const thrower = vi.fn(() => {
            throw new Error("boom");
        });
        const good = vi.fn();
        subscribeSoundEvents(thrower);
        subscribeSoundEvents(good);

        expect(() => notify("agent.turn.complete")).not.toThrow();
        expect(thrower).toHaveBeenCalledTimes(1);
        expect(good).toHaveBeenCalledTimes(1);
        expect(warn).toHaveBeenCalled();
        warn.mockRestore();
    });
});
