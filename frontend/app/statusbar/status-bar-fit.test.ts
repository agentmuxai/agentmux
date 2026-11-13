// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { AGENT_COUNTS_ID, slotDropAt, TOKEN_USAGE_ID } from "./status-bar-fit";

describe("slotDropAt", () => {
    it("never drops the agent entry while anything waits, and drops it at its own tier otherwise", () => {
        expect(slotDropAt(AGENT_COUNTS_ID, 3, true)).toBeUndefined();
        expect(slotDropAt(AGENT_COUNTS_ID, 3, false)).toBe(3);
    });
    it("keeps the token count while AgentMux's own model calls fail, so its dot stays visible, and drops it at its tier otherwise", () => {
        expect(slotDropAt(TOKEN_USAGE_ID, 2, false, true)).toBeUndefined();
        expect(slotDropAt(TOKEN_USAGE_ID, 2, false, false)).toBe(2);
        expect(slotDropAt(TOKEN_USAGE_ID, 2, true)).toBe(2);
    });
    it("does not keep another slot for a failure", () => {
        expect(slotDropAt("agentmux:gpu", 2, false, true)).toBe(2);
    });
    it("leaves every other slot at its tier", () => {
        expect(slotDropAt("agentmux:gpu", 2, true)).toBe(2);
        expect(slotDropAt("agentmux:backend", undefined, true)).toBeUndefined();
    });
});
