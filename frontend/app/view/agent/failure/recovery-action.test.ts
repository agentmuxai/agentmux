// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { postLoginRecoveryFor } from "./recovery-action";

const authFailure: AgentFailure = {
    code: "auth",
    title: "Not authenticated",
    detail: "401",
    retryable: true,
};

const paneFailure = (turnAttempted?: boolean): PaneFailure => ({
    data: authFailure,
    at: 1,
    ...(turnAttempted === undefined ? {} : { turnAttempted }),
});

describe("postLoginRecoveryFor (PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02)", () => {
    // Both outcomes of this branch have already shipped as P1s on PR #2951 —
    // one from getting it wrong in each direction. These pin both.

    it("retries the turn for a real, turn-following auth failure", () => {
        expect(postLoginRecoveryFor(paneFailure(true))).toBe("retry-turn");
    });

    it("sends the startup sequence for the pre-launch case — never a retry", () => {
        // codex P1: retrying here resends an old transcript message on an
        // agent that never ran a turn.
        //
        // WHAT THIS FILE DOES NOT COVER, stated so the count isn't misread:
        // the OTHER P1 on this branch (reagent — the first fix for the above
        // returned early and did NEITHER action) happened at the CALL SITE,
        // `onRecovered` in agent-view.tsx, not here. A pure function returning
        // a two-member union cannot express "did nothing", so no test in this
        // file could have caught it, before or after the extraction. The
        // extraction made the DECISION testable; it did not make the ACTION
        // testable. Call-site wiring is compiler- and review-guarded.
        // (manoz, reviewing 708c7c1c6, caught an earlier version of this file
        // asserting that via a tautology and naming it as if it covered the
        // real failure.)
        expect(postLoginRecoveryFor(paneFailure(false))).toBe("send-startup");
    });

    it("defaults to retry-turn when the flag is absent (pre-existing behaviour)", () => {
        expect(postLoginRecoveryFor(paneFailure(undefined))).toBe("retry-turn");
    });

    it("defaults to retry-turn for null/undefined — recovery with no failure showing", () => {
        expect(postLoginRecoveryFor(null)).toBe("retry-turn");
        expect(postLoginRecoveryFor(undefined)).toBe("retry-turn");
    });
});
