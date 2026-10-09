// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { noteSrvInfoMessage, onSrvInfo, srvInfo, UI_VERSION, versionSkew } from "./srv-info";

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

describe("noteSrvInfoMessage", () => {
    it("records srvinfo straight off the socket, and ignores other messages", () => {
        onSrvInfo({ version: UI_VERSION });
        noteSrvInfoMessage({ command: "eventrecv", data: { event: "config", data: { version: "1.0.0" } } });
        noteSrvInfoMessage({ command: "rpcresponse" });
        noteSrvInfoMessage(null);
        expect(srvInfo()?.version).toBe(UI_VERSION);

        noteSrvInfoMessage({ command: "eventrecv", data: { event: "srvinfo", data: { version: "8.8.8", hostName: "h" } } });
        expect(srvInfo()).toMatchObject({ version: "8.8.8", hostName: "h" });
    });
});

describe("a srv too old to send srvinfo", () => {
    it("counts as a mismatch once an RPC has answered without srvinfo", async () => {
        vi.resetModules();
        const m = await import("./srv-info");
        expect(m.versionSkew()).toBeNull();
        m.noteSrvInfoDue();
        expect(m.versionSkew()).toBe(m.OLDER_SRV);
        m.onSrvInfo({ version: m.UI_VERSION });
        expect(m.versionSkew()).toBeNull();
    });
});
