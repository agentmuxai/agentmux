// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isStopping, workingFromPhase, type TurnPhase } from "./types";

const phase = (kind: TurnPhase["kind"]) => ({ kind }) as TurnPhase;

describe("isStopping", () => {
    it("is true only while a stop is in flight", () => {
        expect(isStopping(phase("Interrupting"))).toBe(true);
        for (const kind of ["Idle", "Submitting", "Streaming", "Done", "Disconnected"] as const) {
            expect(isStopping(phase(kind))).toBe(false);
        }
    });

    it("a stopping pane still counts as working", () => {
        expect(workingFromPhase(phase("Interrupting"))).toBe(true);
    });
});
