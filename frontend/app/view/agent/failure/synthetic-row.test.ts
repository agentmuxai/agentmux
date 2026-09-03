// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { decideSyntheticRow } from "./synthetic-row";

const failure = (turnAttempted: boolean): PaneFailure => ({
    data: { code: "auth", title: "t", detail: "", retryable: true },
    at: 1,
    turnAttempted,
});
const synthetic = () => failure(false);
const real = () => failure(true);

const decide = (i: Partial<Parameters<typeof decideSyntheticRow>[0]>) =>
    decideSyntheticRow({
        canRetry: true,
        current: null,
        previous: null,
        syntheticDismissed: false,
        ...i,
    });

describe("decideSyntheticRow (PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02)", () => {
    describe("while the agent is still unauthenticated (canRetry true)", () => {
        it("raises the row when nothing is showing", () => {
            expect(decide({}).action).toBe("raise");
        });

        it("yields to a REAL failure — never overwrites the richer row", () => {
            expect(decide({ current: real() }).action).toBe("none");
        });

        it("leaves its own row alone once raised", () => {
            expect(decide({ current: synthetic(), previous: synthetic() }).action).toBe("none");
        });

        it("does NOT re-raise after the user dismisses the synthetic row", () => {
            // Re-raising here is what would make the row undismissable, which
            // is why the effect cannot simply track the failure signal.
            const d = decide({ current: null, previous: synthetic() });
            expect(d.action).toBe("none");
            expect(d.syntheticDismissed).toBe(true);
        });

        it("keeps honouring that dismissal on later evaluations", () => {
            expect(decide({ syntheticDismissed: true }).action).toBe("none");
        });

        it("DOES raise after a REAL failure is dismissed — the flagship stacked case (reagent P1)", () => {
            // The regression this closes: in the PR's own repro a real
            // persisted auth failure renders while canRetry is independently
            // true. Dismissing it used to leave the pane with NO login
            // affordance at all — strictly worse than the pre-PR bar, which
            // was undismissable and always remained in exactly this scenario.
            const d = decide({ current: null, previous: real() });
            expect(d.action).toBe("raise");
            expect(d.syntheticDismissed).toBe(false);
        });

        it("a REAL failure dismissal beats an earlier synthetic dismissal in the same episode (manoz)", () => {
            // The sequence that short-circuited before: dismiss synthetic ->
            // real failure arrives -> dismiss real. `syntheticDismissed` was
            // sticky and checked FIRST, so step 4 returned "none" and the pane
            // was left with no login CTA — reagent's P1 state, reached by a
            // different route than the one it was reported for.
            //
            // Deliberate rule, not an accident of branch order: the two
            // dismissals dismiss different things. Waving away "Not signed in"
            // says "stop offering me this login"; waving away a real failure
            // report dismisses an ERROR and says nothing about the CTA.
            const dismissedSynthetic = decide({ current: null, previous: synthetic() });
            expect(dismissedSynthetic.syntheticDismissed).toBe(true);

            // Real failure arrives and is yielded to.
            const realShowing = decide({
                current: real(),
                previous: null,
                syntheticDismissed: dismissedSynthetic.syntheticDismissed,
            });
            expect(realShowing.action).toBe("none");
            expect(realShowing.syntheticDismissed).toBe(true); // still sticky here

            // User dismisses the REAL failure — must raise despite the sticky flag.
            const afterRealDismissed = decide({
                current: null,
                previous: real(),
                syntheticDismissed: realShowing.syntheticDismissed,
            });
            expect(afterRealDismissed.action).toBe("raise");
            expect(afterRealDismissed.syntheticDismissed).toBe(false); // memory reset
        });

        it("the synthetic row is still dismissable AFTER that fallback raise", () => {
            // The reset must not make the row undismissable again — dismissing
            // the re-raised synthetic row still sticks.
            const d = decide({ current: null, previous: synthetic(), syntheticDismissed: false });
            expect(d.action).toBe("none");
            expect(d.syntheticDismissed).toBe(true);
        });
    });

    describe("once the agent is authenticated again (canRetry false)", () => {
        it("retracts its own row", () => {
            expect(decide({ canRetry: false, current: synthetic() }).action).toBe("retract");
        });

        it("does NOT retract a real failure — that lifecycle is not ours", () => {
            expect(decide({ canRetry: false, current: real() }).action).toBe("none");
        });

        it("does nothing when no row is showing", () => {
            expect(decide({ canRetry: false, current: null }).action).toBe("none");
        });

        it("resets the dismissal memory so a later episode starts clean", () => {
            expect(decide({ canRetry: false, syntheticDismissed: true }).syntheticDismissed).toBe(false);
        });
    });

    it("terminates: raising then re-evaluating does not raise again", () => {
        // Guards the obvious loop — raise sets `current`, which must then be a
        // no-op rather than another raise.
        const first = decide({});
        expect(first.action).toBe("raise");
        const second = decide({ current: synthetic(), previous: null, syntheticDismissed: first.syntheticDismissed });
        expect(second.action).toBe("none");
    });
});
