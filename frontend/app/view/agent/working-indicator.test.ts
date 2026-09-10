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
            expect(paneBusyForInput({ showingLaunchActivity: false, turnPhase: p, compacting: null, reconnecting: null })).toBe(
                workingFromPhase(p),
            );
        }
    });

    it("is busy while launching even though no turn is in flight", () => {
        // A message sent mid-launch is not answered immediately either.
        expect(
            paneBusyForInput({ showingLaunchActivity: true, turnPhase: phase("Idle"), compacting: null, reconnecting: null }),
        ).toBe(true);
    });

    it("is idle only when nothing is launching and no turn is running", () => {
        expect(
            paneBusyForInput({ showingLaunchActivity: false, turnPhase: phase("Idle"), compacting: null, reconnecting: null }),
        ).toBe(false);
        expect(
            paneBusyForInput({ showingLaunchActivity: false, turnPhase: phase("Done"), compacting: null, reconnecting: null }),
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
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Streaming"),
            }),
        ).toBe(true);
    });

    it("stays busy for a rate-limited Streaming turn", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Streaming", { waitingReason: "rate_limit" }),
            }),
        ).toBe(true);
    });

    it("stays busy while interrupting", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Interrupting"),
            }),
        ).toBe(true);
    });
});

describe("working indicator — busy without a turn in flight", () => {
    // Divergence B from the report (§3.2). These two states can be set while
    // turnPhase is Idle. The working row read them; the progress bar and the
    // composer strip did not — so during a reconnect the row said busy and the
    // bar said idle. Both are in the predicate now, so all three agree.
    const idle = { showingLaunchActivity: false, turnPhase: phase("Idle") };

    it("is busy while reconnecting, even with no turn running", () => {
        // The least ambiguous case in the whole predicate: `reconnecting` is
        // set ONLY after the process has already crashed or exited, so there is
        // literally nothing alive to answer a message typed now.
        expect(
            paneBusyForInput({ ...idle, compacting: null, reconnecting: { attempt: 1 } }),
        ).toBe(true);
    });

    it("is busy while compacting, even with no turn running", () => {
        expect(
            paneBusyForInput({ ...idle, compacting: { trigger: "auto" }, reconnecting: null }),
        ).toBe(true);
    });

    it("treats only null as not-busy, not merely falsy", () => {
        // These are opaque state objects; a caller passing through a falsy-but-
        // present value must not read as idle.
        expect(paneBusyForInput({ ...idle, compacting: 0, reconnecting: null })).toBe(true);
        expect(paneBusyForInput({ ...idle, compacting: null, reconnecting: "" })).toBe(true);
        expect(
            paneBusyForInput({ ...idle, compacting: undefined, reconnecting: undefined }),
        ).toBe(false);
    });
});
