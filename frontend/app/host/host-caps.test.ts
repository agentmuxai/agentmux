// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";

import { CEF_HOST_CAPS, hostHas, NO_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";

const realApi = window.api;
afterEach(() => {
    window.api = realApi;
});

describe("host capabilities", () => {
    it("CEF and no-caps hosts list the same capabilities", () => {
        expect(Object.keys(NO_HOST_CAPS).sort()).toEqual(Object.keys(CEF_HOST_CAPS).sort());
        expect(Object.values(CEF_HOST_CAPS).every(Boolean)).toBe(true);
        expect(Object.values(NO_HOST_CAPS).some(Boolean)).toBe(false);
    });

    it("hostHas reads the current host's capabilities", () => {
        window.api = makeTestHostApi({}, { autostart: true });
        expect(hostHas("autostart")).toBe(true);
        expect(hostHas("multiWindow")).toBe(false);
    });

    it("the test host's listen resolves to an unsubscribe function, like a real host's", async () => {
        const unlisten = await makeTestHostApi().listen("any-event", () => {});
        expect(typeof unlisten).toBe("function");
        expect(() => unlisten()).not.toThrow();
    });

    it("the test host no-ops anything not overridden", async () => {
        const api = makeTestHostApi({ getPlatform: () => "linux" });
        expect(api.getPlatform()).toBe("linux");
        expect(api.getHostCaps()).toEqual(NO_HOST_CAPS);
        await expect(Promise.resolve(api.openNewWindow())).resolves.toBeUndefined();
    });
});
