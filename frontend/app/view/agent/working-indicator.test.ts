// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { workingFromPhase, type TurnPhase } from "@/app/store/agent-pane-state/types";
import { paneBusyForInput } from "./working-indicator";

const phase = (kind: TurnPhase["kind"], extra: Record<string, unknown> = {}): TurnPhase =>
    ({ kind, ...extra }) as TurnPhase;

describe("working indicator — mirrors the send gate", () => {
    // The invariant that matters: agent-view's send path decides whether to
    // queue with workingFromPhase() on the pane snapshot. If this predicate
    // ever disagrees with that for a bare phase, an indicator is lying.
    const ALL_PHASES: TurnPhase["kind"][] = [
        "Idle",
        "Submitting",
        "Streaming",
        "Interrupting",
        "Done",
        "Disconnected",
    ];

    it("agrees with workingFromPhase for every phase when not launching", () => {
        for (const kind of ALL_PHASES) {
            const p = phase(kind);
            expect(paneBusyForInput({ showingLaunchActivity: false, turnPhase: p })).toBe(
                workingFromPhase(p),
            );
        }
    });

    it("is busy while launching even though no turn is in flight", () => {
        // A message sent mid-launch is not answered immediately either.
        expect(
            paneBusyForInput({ showingLaunchActivity: true, turnPhase: phase("Idle") }),
        ).toBe(true);
    });

    it("is idle only when nothing is launching and no turn is running", () => {
        expect(
            paneBusyForInput({ showingLaunchActivity: false, turnPhase: phase("Idle") }),
        ).toBe(false);
        expect(
            paneBusyForInput({ showingLaunchActivity: false, turnPhase: phase("Done") }),
        ).toBe(false);
    });
});

describe("working indicator — the dock must not silence it", () => {
    it("stays busy for a Streaming turn regardless of any dock activity", () => {
        // The regression this module exists to prevent. A tool call promoted to
        // the ActivityDock at TOOL_PROMOTION_MS is still a call in flight: the
        // turn is blocked, input still queues. The old workingRowSupersededByDock
        // path hid the "Working…" row at exactly that moment while the progress
        // bar kept running — so at 30s into every long Bash call the two
        // indicators disagreed and the row under-reported a closed gate.
        //
        // There is deliberately no dock input to this predicate. If someone adds
        // one, this test is the thing that should stop them.
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                turnPhase: phase("Streaming"),
            }),
        ).toBe(true);
    });

    it("stays busy for a rate-limited Streaming turn", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                turnPhase: phase("Streaming", { waitingReason: "rate_limit" }),
            }),
        ).toBe(true);
    });

    it("stays busy while interrupting", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                turnPhase: phase("Interrupting"),
            }),
        ).toBe(true);
    });
});
