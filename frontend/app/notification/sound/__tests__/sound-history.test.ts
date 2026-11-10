// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";

import { __resetPlayedSounds, noteSound, recentSounds, SOUND_RING, subscribePlayedSounds } from "../sound-history";

describe("the ring of played sounds", () => {
    beforeEach(() => __resetPlayedSounds());

    it("keeps what played, oldest first", () => {
        noteSound({ blockId: "b1", event: "tool", tool: "Bash", at: 1 });
        noteSound({ blockId: "b2", event: "agent.turn.complete", at: 2 });
        expect(recentSounds().map((s) => [s.blockId, s.event])).toEqual([
            ["b1", "tool"],
            ["b2", "agent.turn.complete"],
        ]);
    });

    it("keeps only the newest SOUND_RING of them", () => {
        for (let n = 0; n < SOUND_RING + 7; n++) noteSound({ blockId: "b", event: "tool", at: n });
        expect(recentSounds()).toHaveLength(SOUND_RING);
        expect(recentSounds()[0].at).toBe(7);
        expect(recentSounds().at(-1)!.at).toBe(SOUND_RING + 6);
    });
});

describe("subscribing to the ring", () => {
    it("tells a listener each sound until it stops", () => {
        __resetPlayedSounds();
        let told = 0;
        const stop = subscribePlayedSounds(() => told++);
        noteSound({ blockId: "b", event: "tool", at: 1 });
        noteSound({ blockId: "b", event: "tool", at: 2 });
        stop();
        noteSound({ blockId: "b", event: "tool", at: 3 });
        expect(told).toBe(2);
    });
});
