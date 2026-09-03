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
        expect(postLoginRecoveryFor(paneFailure(false))).toBe("send-startup");
    });

    it("still does SOMETHING for the pre-launch case — never neither", () => {
        // reagent P1 (second re-review): the first fix for the codex P1 simply
        // returned early, which left a never-launched agent authenticated with
        // a controller that never got its startup payload. "send-startup" is
        // the whole point of this branch existing rather than a bare guard.
        expect(postLoginRecoveryFor(paneFailure(false))).not.toBe("retry-turn");
        expect(["retry-turn", "send-startup"]).toContain(postLoginRecoveryFor(paneFailure(false)));
    });

    it("defaults to retry-turn when the flag is absent (pre-existing behaviour)", () => {
        expect(postLoginRecoveryFor(paneFailure(undefined))).toBe("retry-turn");
    });

    it("defaults to retry-turn for null/undefined — recovery with no failure showing", () => {
        expect(postLoginRecoveryFor(null)).toBe("retry-turn");
        expect(postLoginRecoveryFor(undefined)).toBe("retry-turn");
    });
});
