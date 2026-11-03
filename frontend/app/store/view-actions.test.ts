// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { countBasicTerms, registerViewActions, runViewAction, setBasicTermCount } from "./view-actions";

afterEach(() => vi.restoreAllMocks());

describe("view actions", () => {
    it("runs a registered action", async () => {
        const wake = vi.fn(async () => 1);
        registerViewActions({ hibernateHiddenBrowserPanes: vi.fn(async () => 0), wakeAllBrowserPanes: wake, redockThisFloater: vi.fn(async () => false) });
        runViewAction("wakeAllBrowserPanes");
        expect(wake).toHaveBeenCalledOnce();
    });

    it("logs a failing action instead of throwing", async () => {
        const err = vi.spyOn(console, "error").mockImplementation(() => {});
        registerViewActions({ hibernateHiddenBrowserPanes: vi.fn(async () => 0), wakeAllBrowserPanes: vi.fn(async () => 0), redockThisFloater: async () => { throw new Error("no window"); } });
        runViewAction("redockThisFloater");
        await vi.waitFor(() => expect(err).toHaveBeenCalled());
    });

    it("counts no terminals until the terminal view says how many", () => {
        expect(countBasicTerms()).toBe(0);
        setBasicTermCount(() => 3);
        expect(countBasicTerms()).toBe(3);
    });
});
