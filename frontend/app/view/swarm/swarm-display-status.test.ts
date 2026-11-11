// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

const h = vi.hoisted(() => ({ phase: null as unknown }));
vi.mock("@/app/store/agentActivity", () => ({
    getBlockTurnPhase: () => (h.phase == null ? undefined : () => h.phase),
}));

import { phaseToDisplayStatus } from "./swarm-display-status";

const withPhase = (phase: TurnPhase | null) => {
    h.phase = phase;
};

describe("phaseToDisplayStatus", () => {
    beforeEach(() => withPhase(null));

    it("with no mounted pane, srv's word decides", () => {
        expect(phaseToDisplayStatus("b", "running")).toBe("working");
        expect(phaseToDisplayStatus("b", "idle")).toBe("idle");
    });

    it("a pane that reads idle or finished is still working while srv says the turn is active", () => {
        withPhase({ kind: "Idle" });
        expect(phaseToDisplayStatus("b", "running")).toBe("working");
        expect(phaseToDisplayStatus("b", "idle")).toBe("idle");
        withPhase({ kind: "Done", outcome: "completed", finishedAt: 1 });
        expect(phaseToDisplayStatus("b", "running")).toBe("working");
        expect(phaseToDisplayStatus("b", "idle")).toBe("idle");
    });

    it("a failed, stopped or interrupted turn stays what it is", () => {
        withPhase({ kind: "Done", outcome: "errored", finishedAt: 1 });
        expect(phaseToDisplayStatus("b", "running")).toBe("error");
        withPhase({ kind: "Done", outcome: "stopped", finishedAt: 1 });
        expect(phaseToDisplayStatus("b", "running")).toBe("idle");
    });

    it("a live phase wins: streaming is working, with tools running it is tools", () => {
        withPhase({ kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: 1 });
        expect(phaseToDisplayStatus("b", "idle")).toBe("working");
        withPhase({ kind: "Streaming", bufferSize: 0, toolsActive: 2, lastEventMs: 1 });
        expect(phaseToDisplayStatus("b", "idle")).toBe("tools");
    });
});
