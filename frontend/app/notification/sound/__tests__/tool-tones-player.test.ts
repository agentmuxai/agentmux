// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolTonesPlayer.play()'s return value: the context time of the syllable
 * the caller will actually hear. The activity flash times itself from it
 * (SPEC_AGENT_ACTIVITY_FLASH_SOUND_SYNC_2026_09_24.md §3.6), including for a
 * call coalesced into a syllable another pane already started.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TOOL_TONE_COALESCE_MS, ToolTonesPlayer } from "../tool-tones-player";

function makeNode(): Record<string, unknown> {
    const param = () => ({
        value: 0,
        setValueAtTime: vi.fn(),
        exponentialRampToValueAtTime: vi.fn(),
    });
    const node: Record<string, unknown> = {
        gain: param(),
        frequency: param(),
        Q: param(),
        type: "",
        start: vi.fn(),
        stop: vi.fn(),
    };
    node.connect = vi.fn(() => node);
    return node;
}

function makeCtx(currentTime: number): AudioContext & { currentTime: number } {
    return {
        currentTime,
        createOscillator: vi.fn(() => makeNode()),
        createGain: vi.fn(() => makeNode()),
        createBiquadFilter: vi.fn(() => makeNode()),
    } as unknown as AudioContext & { currentTime: number };
}

describe("ToolTonesPlayer.play return value", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("returns null until attached", () => {
        expect(new ToolTonesPlayer().play(makeCtx(1), "Read")).toBeNull();
    });

    it("returns the scheduled start time of a syllable it plays", () => {
        const player = new ToolTonesPlayer();
        const ctx = makeCtx(2.5);
        player.attach(ctx, makeNode() as unknown as GainNode);
        expect(player.play(ctx, "Read")).toBe(2.5);
    });

    it("returns the audible syllable's start time for a coalesced call, not null", () => {
        const player = new ToolTonesPlayer();
        const ctx = makeCtx(2.5);
        player.attach(ctx, makeNode() as unknown as GainNode);
        const oscillatorsBefore = () => (ctx.createOscillator as ReturnType<typeof vi.fn>).mock.calls.length;

        expect(player.play(ctx, "Read")).toBe(2.5);
        const played = oscillatorsBefore();
        vi.advanceTimersByTime(5);
        ctx.currentTime = 2.505;
        // Another pane's Read, 5 ms later: no second syllable, but the caller
        // hears the first one, which started at 2.5.
        expect(player.play(ctx, "Read")).toBe(2.5);
        expect(oscillatorsBefore()).toBe(played);
    });

    it("schedules and returns a fresh start time once the coalesce window has passed", () => {
        const player = new ToolTonesPlayer();
        const ctx = makeCtx(2.5);
        player.attach(ctx, makeNode() as unknown as GainNode);
        player.play(ctx, "Read");
        vi.advanceTimersByTime(TOOL_TONE_COALESCE_MS);
        ctx.currentTime = 2.53;
        expect(player.play(ctx, "Read")).toBe(2.53);
    });
});
