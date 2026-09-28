// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isMuxBusSessionOk, muxbusNeedsSignInAgain } from "./muxbus-session";

describe("muxbus session state", () => {
    it("a dead sign-in is not ok even while its token lasts", () => {
        const dead = { connected: true, valid: true, needsReauth: true };
        expect(isMuxBusSessionOk(dead)).toBe(false);
        expect(muxbusNeedsSignInAgain(dead)).toBe(true);
    });

    it("a live, unexpired sign-in is ok", () => {
        const live = { connected: true, valid: true, needsReauth: false };
        expect(isMuxBusSessionOk(live)).toBe(true);
        expect(muxbusNeedsSignInAgain(live)).toBe(false);
    });

    it("expired, signed-out and unknown are not ok and not dead", () => {
        for (const s of [
            { connected: true, valid: false, needsReauth: false },
            { connected: false, valid: false },
            null,
        ]) {
            expect(isMuxBusSessionOk(s)).toBe(false);
            expect(muxbusNeedsSignInAgain(s)).toBe(false);
        }
    });
});
