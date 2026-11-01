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
            expect(said).toBe(unreachableText("turn on auto-approve"));
            expect(said).not.toMatch(/os error|srv:|ECONN|connection attempt/i);
        }
        expect(unreachableText("turn on auto-approve")).toContain("Nothing was changed");
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
            unreachableText("stop auto-approve"),
        );
    });

    it("says something for an error with no message", () => {
        expect(plainError(undefined, "pause the agent")).toBe("Couldn't pause the agent.");
        expect(plainError(new Error(""), "pause the agent")).toBe("Couldn't pause the agent.");
    });
});
