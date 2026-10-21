// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { _resetDebugLogForTests, debugLog } from "./debuglog";

afterEach(() => {
    localStorage.removeItem("debug");
    _resetDebugLogForTests();
    vi.restoreAllMocks();
});

describe("debugLog", () => {
    it("prints nothing unless localStorage.debug names the namespace", () => {
        const out = vi.spyOn(console, "debug").mockImplementation(() => {});
        debugLog("wave:ws")("hidden");
        expect(out).not.toHaveBeenCalled();
    });

    it("prints for a matching namespace or wildcard, and not for others", () => {
        localStorage.setItem("debug", "wave:router, wave:t*");
        const out = vi.spyOn(console, "debug").mockImplementation(() => {});
        debugLog("wave:router")("a", 1);
        debugLog("wave:termwrap")("b");
        debugLog("wave:ws")("c");
        expect(out.mock.calls).toEqual([
            ["[wave:router]", "a", 1],
            ["[wave:termwrap]", "b"],
        ]);
    });
});
