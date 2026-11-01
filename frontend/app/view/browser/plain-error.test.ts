// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isRawNetworkFailure, plainError, unreachableText } from "./plain-error";

const RAW_TIMEOUT =
    "read from srv: A connection attempt failed because the connected party did not properly respond after a period of time, or established connection failed because connected host has failed to respond. (os error 10060)";

describe("plainError", () => {
    it("never shows the system's own words for a connection that failed", () => {
        for (const raw of [
            RAW_TIMEOUT,
            "connect to srv: No connection could be made because the target machine actively refused it. (os error 10061)",
            "write to srv: Broken pipe (os error 32)",
            "connect ECONNREFUSED 127.0.0.1:5000",
            "Failed to fetch",
            "request to http://127.0.0.1/x failed, reason: socket hang up",
        ]) {
            expect(isRawNetworkFailure(raw), raw).toBe(true);
            const said = plainError(new Error(raw), "turn on auto-approve");
            expect(said).toMatch(/^(Couldn't reach AgentMux|AgentMux didn't answer)/);
            expect(said).not.toMatch(/os error|srv:|ECONN|connection attempt/i);
        }
    });

    it("claims nothing was changed only when the request never went out", () => {
        for (const never of ["connect to srv: refused (os error 10061)", "connect ECONNREFUSED 127.0.0.1:5000", "Failed to fetch"]) {
            expect(plainError(new Error(never), "turn on auto-approve"), never).toBe(unreachableText("turn on auto-approve"));
        }
        expect(unreachableText("turn on auto-approve")).toContain("Nothing was changed");
        // It may have gone out and got no answer, or the connection broke: not known.
        for (const sent of [RAW_TIMEOUT, "write to srv: Broken pipe (os error 32)", "read ECONNRESET", "socket hang up"]) {
            const said = plainError(new Error(sent), "turn on auto-approve");
            expect(said, sent).toBe(unreachableText("turn on auto-approve", true));
            expect(said).not.toContain("Nothing was changed");
            expect(said).toContain("isn't certain");
        }
    });

    it("shows srv's own reason as it is, with the caller's lead-in", () => {
        const reason = "the page moved since you were shown it";
        expect(isRawNetworkFailure(reason)).toBe(false);
        expect(plainError(new Error(reason), "turn on auto-approve")).toBe(reason);
        expect(plainError(reason, "stop auto-approve", (m) => `Couldn't stop auto-approve: ${m}`)).toBe(
            `Couldn't stop auto-approve: ${reason}`,
        );
    });

    it("puts a raw failure in the caller's words, not behind the lead-in", () => {
        expect(plainError(new Error(RAW_TIMEOUT), "stop auto-approve", (m) => `Couldn't stop auto-approve: ${m}`)).toBe(
            unreachableText("stop auto-approve", true),
        );
    });

    it("leaves a failure the host already worded alone, behind no lead-in", () => {
        const wrap = (m: string) => `Couldn't stop auto-approve: ${m}`;
        const notSent = "Couldn't reach AgentMux to stop auto-approve. Nothing was changed. Try again in a moment.";
        const noAnswer = "AgentMux didn't answer in time when asked to pause the agent, so it isn't certain it was done. Check this pane; if it isn't, try again: repeating it is safe.";
        expect(plainError(new Error(notSent), "stop auto-approve", wrap)).toBe(notSent);
        expect(plainError(new Error(noAnswer), "pause the agent", wrap)).toBe(noAnswer);
    });

    it("says something for an error with no message", () => {
        expect(plainError(undefined, "pause the agent")).toBe("Couldn't pause the agent.");
        expect(plainError(new Error(""), "pause the agent")).toBe("Couldn't pause the agent.");
    });
});
