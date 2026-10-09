// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { onSrvInfo, srvInfo, UI_VERSION, versionSkew } from "./srv-info";

describe("srvinfo", () => {
    it("knows its own version from the build", () => {
        expect(UI_VERSION).toMatch(/^\d+\.\d+\.\d+/);
    });

    it("records what srv reports, and sees no skew at the same version", () => {
        onSrvInfo({ version: UI_VERSION, platform: "linux", userName: "u", hostName: "h", homeDir: "/home/u/.agentmux" });
        expect(srvInfo()).toEqual({
            version: UI_VERSION,
            platform: "linux",
            userName: "u",
            hostName: "h",
            homeDir: "/home/u/.agentmux",
        });
        expect(versionSkew()).toBeNull();
    });

    it("reports srv's version when it differs", () => {
        onSrvInfo({ version: "0.0.1" });
        expect(versionSkew()).toBe("0.0.1");
        expect(srvInfo()?.homeDir).toBeNull();
    });

    it("ignores a payload without a version", () => {
        onSrvInfo({ version: UI_VERSION });
        onSrvInfo({ platform: "darwin" });
        onSrvInfo(null);
        expect(srvInfo()?.version).toBe(UI_VERSION);
    });
});
