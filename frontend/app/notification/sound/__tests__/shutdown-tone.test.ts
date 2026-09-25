// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shutdown-pending chime (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5): a
 * falling A4 → F4 → D4, not the question tone's rising major triad, and gated
 * by the master switch and its own setting.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const settings: Record<string, unknown> = {};
vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => () => settings[key],
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { get blockFocusAtom() { return () => null; } },
}));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ addEventListener: () => () => {} }));

import { playShutdownTone, SHUTDOWN_ARPEGGIO_HZ } from "../shutdown-tone";
import { __getSoundPlayer, __resetSoundService, playShutdownPendingTone } from "../sound-service";

function makeNode() {
    return {
        connect: vi.fn().mockReturnThis(),
        disconnect: vi.fn(),
        start: vi.fn(),
        stop: vi.fn(),
        gain: { value: 0, setValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn() },
        frequency: { value: 0, setValueAtTime: vi.fn() },
        type: "sine",
    };
}

function makeFakeCtx() {
    const oscillators: ReturnType<typeof makeNode>[] = [];
    const ctx = {
        currentTime: 0,
        state: "running",
        resume: vi.fn().mockResolvedValue(undefined),
        createOscillator: vi.fn(() => {
            const n = makeNode();
            oscillators.push(n);
            return n;
        }),
        createGain: vi.fn(() => makeNode()),
        createBiquadFilter: vi.fn(() => ({ ...makeNode(), type: "lowpass", Q: { value: 0 }, frequency: { value: 0 } })),
    } as unknown as AudioContext;
    return { ctx, oscillators };
}

describe("shutdown tone", () => {
    beforeEach(() => {
        for (const k of Object.keys(settings)) delete settings[k];
        __resetSoundService();
    });

    it("falls A4 → F4 → D4, one note each", () => {
        const { ctx, oscillators } = makeFakeCtx();
        playShutdownTone(ctx, makeNode() as unknown as GainNode, 0.25);
        const hz = oscillators.map((o) => o.frequency.setValueAtTime.mock.calls[0][0]);
        expect(hz).toEqual([...SHUTDOWN_ARPEGGIO_HZ]);
        expect(hz).toEqual([440.0, 349.23, 293.66]);
        expect(hz[0] > hz[1] && hz[1] > hz[2]).toBe(true);
    });

    it("plays only with the master switch and its own setting on", () => {
        const { ctx } = makeFakeCtx();
        const player = __getSoundPlayer();
        vi.spyOn(player, "getAudioContext").mockReturnValue(ctx);
        vi.spyOn(player, "getMasterGain").mockReturnValue(makeNode() as unknown as GainNode);

        expect(playShutdownPendingTone()).toBe(true);
        settings["notify:sound:agent.shutdown.pending"] = false;
        expect(playShutdownPendingTone()).toBe(false);
        settings["notify:sound:agent.shutdown.pending"] = true;
        settings["notify:sounds:enabled"] = false;
        expect(playShutdownPendingTone()).toBe(false);
    });
});
