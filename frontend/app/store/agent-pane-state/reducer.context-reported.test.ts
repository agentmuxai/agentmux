// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `ContextReported`: an agent's own report of its context fill
// (view/agent/acp-context.ts), as OpenCode's `usage_update` gives it.

import { describe, expect, it } from "vitest";
import { plausibleReading } from "./context-reading";
import { update } from "./reducer";
import { streaming } from "./reducer-test-fixtures";

describe("ContextReported", () => {
    it("sets the reading on the window reported for its model", () => {
        let s = streaming();
        s = update(s, { type: "ContextWindowsReported", windows: { "opencode/big-pickle": 200_000 } }).state;
        s = update(s, { type: "ContextReported", tokens: 15_695, model: "opencode/big-pickle" }, 5_000).state;
        expect(plausibleReading(s.context)).toMatchObject({
            tokens: 15_695,
            model: "opencode/big-pickle",
            window: 200_000,
            windowSource: "reported",
            source: "live",
            at: 5_000,
        });
        expect(s.lastContextModel).toBe("opencode/big-pickle");
    });

    it("starts no call: the turn's tokens and phase stay as they were", () => {
        const before = update(streaming(), { type: "TokensIn", input: 900, model: "opencode/big-pickle" }).state;
        const after = update(before, { type: "ContextReported", tokens: 15_695, model: "opencode/big-pickle" }).state;
        expect(after.turnTokens).toBe(before.turnTokens);
        expect(after.turnPhase).toBe(before.turnPhase);
        expect(after.activity).toBe(before.activity);
    });

    it("with no model, keeps the last one seen; with none seen, the window is unknown", () => {
        let s = update(streaming(), { type: "ContextReported", tokens: 12_000 }).state;
        expect(s.context).toMatchObject({ tokens: 12_000, model: null, window: null });
        s = update(s, { type: "ContextWindowsReported", windows: { "opencode/big-pickle": 200_000 } }).state;
        s = update(s, { type: "ContextReported", tokens: 13_000, model: "opencode/big-pickle" }).state;
        s = update(s, { type: "ContextReported", tokens: 14_000 }).state;
        expect(s.context).toMatchObject({ tokens: 14_000, model: "opencode/big-pickle", window: 200_000 });
    });

    it("is a live reading, so a history seed can't replace it", () => {
        let s = update(streaming(), { type: "ContextReported", tokens: 15_000, model: "m" }).state;
        s = update(s, { type: "ReconcileContextFromHistory", tokens: 90_000, model: "m", at: 1, reportedWindows: null }).state;
        expect(s.context?.tokens).toBe(15_000);
    });
});
