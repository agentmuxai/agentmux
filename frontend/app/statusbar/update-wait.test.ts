// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { updateWait } from "./update-wait";

const turn = (...agents: string[]) => agents.map((agent) => ({ agent, why: "turn" }));

describe("updateWait: what a restart says while it waits for agents", () => {
    it("is not a wait with nobody to wait for: nothing can say 'waiting for 0 agents'", () => {
        expect(updateWait([], 0)).toBeNull();
        expect(updateWait(undefined, 0)).toBeNull();
        expect(updateWait(undefined, -1)).toBeNull();
    });

    it("names one, two and three agents", () => {
        expect(updateWait(turn("Vmer"), 1)).toEqual({
            lines: ["Waiting for Vmer to finish its turn."],
            interrupts: "Restart anyway interrupts Vmer.",
        });
        expect(updateWait(turn("Vmer", "Deke"), 2)).toEqual({
            lines: ["Waiting for Vmer and Deke to finish their turn."],
            interrupts: "Restart anyway interrupts Vmer and Deke.",
        });
        expect(updateWait(turn("Vmer", "Deke", "Posa"), 3)).toEqual({
            lines: ["Waiting for Vmer, Deke and Posa to finish their turn."],
            interrupts: "Restart anyway interrupts Vmer, Deke and Posa.",
        });
    });

    it("counts more than three", () => {
        expect(updateWait(turn("A", "B", "C", "D"), 4)).toEqual({
            lines: ["Waiting for 4 agents to finish their turn."],
            interrupts: "Restart anyway interrupts their turns.",
        });
    });

    it("says what holds a turn up when its request is being retried", () => {
        expect(updateWait([{ agent: "Vmer", why: "network_retry" }], 1)).toEqual({
            lines: ["Waiting for Vmer, which is retrying after a network error."],
            interrupts: "Restart anyway interrupts Vmer.",
        });
        expect(updateWait([...turn("Deke"), { agent: "Vmer", why: "api_retry" }], 2)).toEqual({
            lines: ["Waiting for Deke to finish its turn.", "Waiting for Vmer, which is retrying after an API error."],
            interrupts: "Restart anyway interrupts Deke and Vmer.",
        });
        expect(updateWait([...turn("A", "B", "C"), { agent: "D", why: "network_retry" }, { agent: "E", why: "api_retry" }], 5)?.lines).toEqual([
            "Waiting for 5 agents to finish their turn.",
            "2 of them are retrying after an error.",
        ]);
    });

    it("counts when srv gave no names, or a pane has none", () => {
        // An srv that only counts.
        expect(updateWait(undefined, 1)).toEqual({ lines: ["Waiting for 1 agent to finish its turn."], interrupts: "Restart anyway interrupts its turn." });
        expect(updateWait([], 2)?.lines).toEqual(["Waiting for 2 agents to finish their turn."]);
        // A pane without a name is never written as an empty name.
        expect(updateWait([...turn("Vmer"), { agent: "", why: "turn" }], 2)?.lines).toEqual(["Waiting for 2 agents to finish their turn."]);
        expect(updateWait([{ agent: "", why: "network_retry" }], 1)?.lines).toEqual(["Waiting for 1 agent to finish its turn.", "It is retrying after an error."]);
    });
});
