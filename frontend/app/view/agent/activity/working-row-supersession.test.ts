// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
import { workingRowSupersededByDock, type WorkingRowSupersessionInput } from "./working-row-supersession";

const STREAMING: TurnPhase = { kind: "Streaming", bufferSize: 0, toolsActive: 1, lastEventMs: 0 };

/** The case the feature exists for: a promoted tool is running and nothing
 *  else about this turn needs saying. Individual tests override one field. */
function busyWithPromotedTool(over: Partial<WorkingRowSupersessionInput> = {}): WorkingRowSupersessionInput {
    return {
        hasPromotedTool: true,
        showingLaunchActivity: false,
        turnPhase: STREAMING,
        compacting: null,
        reconnecting: null,
        ...over,
    };
}

describe("workingRowSupersededByDock", () => {
    it("supersedes the working row once a tool is promoted to the dock", () => {
        // The whole point of auto-backgrounding: the dock owns this work now,
        // with its own live countdown. "Working…" on top of that claims the
        // pane is blocked when backgrounding it is exactly what just happened.
        expect(workingRowSupersededByDock(busyWithPromotedTool())).toBe(true);
    });

    it("keeps the working row while no tool has been promoted", () => {
        // Ordinary in-flight turn — nothing in the dock to defer to.
        expect(workingRowSupersededByDock(busyWithPromotedTool({ hasPromotedTool: false }))).toBe(false);
    });

    describe("keeps the row for every state the dock cannot express", () => {
        // Each of these can co-occur with a promoted tool. The dock has no
        // vocabulary for any of them, so suppressing the row would drop the
        // information entirely rather than relocate it.

        it("launch activity — the pane isn't even up yet", () => {
            expect(workingRowSupersededByDock(busyWithPromotedTool({ showingLaunchActivity: true }))).toBe(false);
        });

        it("Interrupting — 'Stopping…' is about the turn, not the tool", () => {
            const turnPhase: TurnPhase = { kind: "Interrupting", reason: "user", sigintSentAt: 0 };
            expect(workingRowSupersededByDock(busyWithPromotedTool({ turnPhase }))).toBe(false);
        });

        it("rate-limited — the condition the user most needs to see", () => {
            const turnPhase: TurnPhase = { ...STREAMING, waitingReason: "rate_limited" };
            expect(workingRowSupersededByDock(busyWithPromotedTool({ turnPhase }))).toBe(false);
        });

        it("compacting", () => {
            expect(workingRowSupersededByDock(busyWithPromotedTool({ compacting: { startedAt: 0 } }))).toBe(false);
        });

        it("reconnecting", () => {
            expect(workingRowSupersededByDock(busyWithPromotedTool({ reconnecting: { attempt: 1 } }))).toBe(false);
        });
    });

    it("does not treat a plain Streaming phase without waitingReason as special", () => {
        // Guards the optional-field read: `waitingReason` is absent (not
        // null/false) on an ordinary Streaming phase, so a truthiness check
        // that mishandled `undefined` would silently keep the row forever.
        const turnPhase: TurnPhase = { ...STREAMING, waitingReason: undefined };
        expect(workingRowSupersededByDock(busyWithPromotedTool({ turnPhase }))).toBe(true);
    });

    it("only consults waitingReason on a Streaming phase", () => {
        // Idle carries no waitingReason field at all — reading it off a
        // non-Streaming variant must not throw or flip the result.
        expect(workingRowSupersededByDock(busyWithPromotedTool({ turnPhase: { kind: "Idle" } }))).toBe(true);
    });
});
