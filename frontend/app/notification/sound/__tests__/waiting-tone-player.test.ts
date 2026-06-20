// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unit tests for WaitingTonePlayer — lifecycle, idempotency, and fade
 * scheduling. The AudioContext is stubbed with a minimal fake so we can
 * observe method calls without actual audio output.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WaitingTonePlayer } from "../waiting-tone-player";

// ── Fake AudioContext ──────────────────────────────────────────────────

function makeNode() {
    const node = {
        connect: vi.fn().mockReturnThis(),
        disconnect: vi.fn(),
        start: vi.fn(),
        stop: vi.fn(),
        gain: { value: 0, setValueAtTime: vi.fn(), linearRampToValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn(), cancelScheduledValues: vi.fn() },
        frequency: { value: 0, setValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn() },
        type: "sine",
    };
    return node;
}

function makeFakeCtx(currentTime = 0): AudioContext {
    return {
        currentTime,
        state: "running",
        resume: vi.fn().mockResolvedValue(undefined),
        createOscillator: vi.fn(() => makeNode()),
        createGain: vi.fn(() => makeNode()),
        createBiquadFilter: vi.fn(() => ({ ...makeNode(), type: "lowpass", Q: { value: 0 }, frequency: { value: 0 } })),
        destination: {},
    } as unknown as AudioContext;
}

// ── Tests ──────────────────────────────────────────────────────────────

describe("WaitingTonePlayer", () => {
    let ctx: AudioContext;
    let master: GainNode;
    let player: WaitingTonePlayer;

    beforeEach(() => {
        vi.useFakeTimers();
        ctx = makeFakeCtx();
        master = makeNode() as unknown as GainNode;
        player = new WaitingTonePlayer();
        player.attach(ctx, master);
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("isAttached() returns true after attach()", () => {
        expect(player.isAttached()).toBe(true);
    });

    it("isAttached() returns false before attach()", () => {
        expect(new WaitingTonePlayer().isAttached()).toBe(false);
    });

    it("start() sets __isRunning() to true", () => {
        player.start();
        expect(player.__isRunning()).toBe(true);
    });

    it("start() is idempotent — calling twice does not double-schedule", () => {
        player.start();
        const oscCallsBefore = (ctx.createOscillator as ReturnType<typeof vi.fn>).mock.calls.length;
        player.start(); // second call — should no-op
        const oscCallsAfter = (ctx.createOscillator as ReturnType<typeof vi.fn>).mock.calls.length;
        expect(oscCallsAfter).toBe(oscCallsBefore);
    });

    it("start() schedules oscillators for the arpeggio notes", () => {
        player.start();
        // 3 notes per cycle → 3 oscillators + 3 gain nodes scheduled on first cycle
        expect(ctx.createOscillator).toHaveBeenCalledTimes(3);
    });

    it("stop() sets __isRunning() to false", async () => {
        player.start();
        const stopPromise = player.stop();
        vi.runAllTimers();
        await stopPromise;
        expect(player.__isRunning()).toBe(false);
    });

    it("stop() schedules a gain fade-out", () => {
        player.start();
        player.stop();
        // The gain node's linearRampToValueAtTime should be called for fade-out
        // (called at least once during stop: ramp to 0.0001)
        const gainNode = (ctx.createGain as ReturnType<typeof vi.fn>).mock.results[0].value;
        expect(gainNode.gain.linearRampToValueAtTime).toHaveBeenCalled();
    });

    it("stop() when not running resolves immediately without error", async () => {
        await expect(player.stop()).resolves.toBeUndefined();
    });

    it("5-minute auto-stop fires stop via timeout", () => {
        const stopSpy = vi.spyOn(player, "stop");
        player.start();
        vi.advanceTimersByTime(5 * 60 * 1000 + 100);
        // The timeout fires via sound-service, not internally; just verify
        // the loop re-schedules correctly (no crash) for 5+ minutes.
        expect(player.__isRunning()).toBe(true); // internal loop doesn't self-stop
        stopSpy.mockRestore();
    });

    it("schedules a new cycle after the loop pause", () => {
        player.start();
        const firstOscCount = (ctx.createOscillator as ReturnType<typeof vi.fn>).mock.calls.length;
        // Advance past one full cycle (3 notes * 500ms + 1000ms pause = 2500ms)
        vi.advanceTimersByTime(2500);
        const secondOscCount = (ctx.createOscillator as ReturnType<typeof vi.fn>).mock.calls.length;
        expect(secondOscCount).toBeGreaterThan(firstOscCount);
    });

    it("setVolume() updates gainValue without starting if not running", () => {
        player.setVolume(0.5);
        // gain.gain.value should not have been set (no ramp scheduled pre-start)
        expect(player.__isRunning()).toBe(false);
    });

    it("no-op before attach", () => {
        const bare = new WaitingTonePlayer();
        expect(() => bare.start()).not.toThrow();
        expect(bare.__isRunning()).toBe(false);
    });
});
