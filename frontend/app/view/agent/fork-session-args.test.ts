// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveForkSessionArgs } from "./fork-session-args";

describe("resolveForkSessionArgs", () => {
    it("plain reattach (no forkSession) resumes any provider unaffected", () => {
        for (const providerId of ["claude", "codex", "gemini", "muxcode"]) {
            expect(resolveForkSessionArgs({ continueSessionId: "sid-1" }, providerId)).toEqual({
                continueSessionId: "sid-1",
                appendForkFlag: false,
            });
        }
    });

    it("no overrides at all resolves to a plain fresh start for any provider", () => {
        expect(resolveForkSessionArgs(undefined, "claude")).toEqual({
            continueSessionId: "",
            appendForkFlag: false,
        });
    });

    it("Claude fork with a real session id appends the flag and keeps the session id", () => {
        expect(resolveForkSessionArgs({ continueSessionId: "sid-1", forkSession: true }, "claude")).toEqual({
            continueSessionId: "sid-1",
            appendForkFlag: true,
        });
    });

    // reagent's review of PR #2725: a fork requested with no session to
    // fork from must not push a bare --fork-session flag.
    it("Claude fork with an empty session id does not append the flag", () => {
        expect(resolveForkSessionArgs({ continueSessionId: "", forkSession: true }, "claude")).toEqual({
            continueSessionId: "",
            appendForkFlag: false,
        });
        expect(resolveForkSessionArgs({ forkSession: true }, "claude")).toEqual({
            continueSessionId: "",
            appendForkFlag: false,
        });
    });

    // Codex's review of PR #2725: a fork requested for a non-Claude
    // provider must fall back to a true fresh start, not a plain resume
    // of the parent's live session.
    it("non-Claude fork drops the session id entirely rather than plain-resuming", () => {
        for (const providerId of ["codex", "gemini", "muxcode"]) {
            expect(resolveForkSessionArgs({ continueSessionId: "sid-1", forkSession: true }, providerId)).toEqual({
                continueSessionId: "",
                appendForkFlag: false,
            });
        }
    });
});
