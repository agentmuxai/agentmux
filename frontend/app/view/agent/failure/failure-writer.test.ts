// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// raiseFailure and clearFailure put the right command on the pane, naming who or why.

import type { AgentFailure } from "@/types/rpc/AgentFailure";
import { describe, expect, it, vi } from "vitest";
import { clearFailure, raiseFailure } from "./failure-writer";

const f = { code: "auth", title: "Not signed in", detail: "", retryable: true } as AgentFailure;

describe("raiseFailure", () => {
    it("sends the source with the failure, and only the optional parts it was given", () => {
        const dispatchPane = vi.fn();
        raiseFailure({ dispatchPane }, { source: "live", failure: f, at: 5 });
        expect(dispatchPane).toHaveBeenCalledWith({ type: "FailureObserved", failure: f, at: 5, source: "live" });
    });

    it("carries turnAttempted false and the seed's mount time, and is a system dispatch when asked", () => {
        const dispatchPane = vi.fn();
        raiseFailure({ dispatchPane }, { source: "seed", failure: f, at: 5, turnAttempted: false, since: 3, system: true });
        expect(dispatchPane).toHaveBeenCalledWith(
            { type: "FailureObserved", failure: f, at: 5, source: "seed", turnAttempted: false, since: 3 },
            "system",
        );
    });
});

describe("clearFailure", () => {
    it("says why, and names the class to clear when it is only that one", () => {
        const dispatchPane = vi.fn();
        clearFailure({ dispatchPane }, { reason: "recovered", onlyCode: "auth" });
        expect(dispatchPane).toHaveBeenCalledWith({ type: "FailureCleared", reason: "recovered", onlyCode: "auth" });
    });

    it("clears whatever shows when no class is named", () => {
        const dispatchPane = vi.fn();
        clearFailure({ dispatchPane }, { reason: "dismissed", system: true });
        expect(dispatchPane).toHaveBeenCalledWith({ type: "FailureCleared", reason: "dismissed" }, "system");
    });
});
