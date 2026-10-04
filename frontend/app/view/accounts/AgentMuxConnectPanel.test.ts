// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    MuxBusStatusCommand: vi.fn(),
    MuxBusCloudConfigCommand: vi.fn(),
    MuxBusLoginCommand: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

const SIGNED_OUT = { connected: false, email: "", cognitoDomain: "", expiresAt: 0, valid: false, needsReauth: false };

const cloudConfig = (clientId: string) => ({
    source: clientId ? "discovered" : "default",
    api: "https://relay.example.test",
    ws: "wss://ws.example.test",
    console: "",
    cognitoDomain: clientId ? "https://auth.example.test" : "",
    clientId,
    region: "",
    userPoolId: "",
});

// The controller's one cloud config lives at module level, so each test gets
// a fresh module with its own build env.
async function controller(compiledClientId: string) {
    vi.stubEnv("VITE_MUXBUS_CLIENT_ID", compiledClientId);
    vi.stubEnv("VITE_MUXBUS_COGNITO_DOMAIN", "https://auth.compiled.test");
    vi.resetModules();
    const { useMuxBusStatus } = await import("./AgentMuxConnectPanel");
    return useMuxBusStatus();
}

// SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.5.
describe("useMuxBusStatus isConfigured", () => {
    beforeEach(() => {
        rpc.MuxBusStatusCommand.mockResolvedValue(SIGNED_OUT);
        rpc.MuxBusLoginCommand.mockResolvedValue({ success: false, email: "", error: "sign-in cancelled" });
    });
    afterEach(() => {
        vi.unstubAllEnvs();
        vi.clearAllMocks();
    });

    it("a build with a compiled client id is configured without asking srv, and signs in with it", async () => {
        const muxbus = await controller("compiled-id");
        expect(muxbus.isConfigured()).toBe(true);
        await muxbus.refresh();
        await muxbus.connect();
        expect(rpc.MuxBusCloudConfigCommand).not.toHaveBeenCalled();
        expect(rpc.MuxBusLoginCommand).toHaveBeenCalledWith(expect.anything(), {
            cognitoDomain: "https://auth.compiled.test",
            clientId: "compiled-id",
        });
    });

    it("a build without one is configured once muxbus.cloudconfig returns a client id", async () => {
        rpc.MuxBusCloudConfigCommand.mockResolvedValue(cloudConfig("discovered-id"));
        const muxbus = await controller("");
        expect(muxbus.isConfigured()).toBe(false);
        await muxbus.refresh();
        await vi.waitFor(() => expect(muxbus.isConfigured()).toBe(true));
        await muxbus.connect();
        expect(rpc.MuxBusLoginCommand).toHaveBeenCalledWith(expect.anything(), {
            cognitoDomain: "https://auth.example.test",
            clientId: "discovered-id",
        });
    });

    it("a build without one stays unconfigured when srv has only its defaults", async () => {
        rpc.MuxBusCloudConfigCommand.mockResolvedValue(cloudConfig(""));
        const muxbus = await controller("");
        await muxbus.refresh();
        await muxbus.connect();
        expect(muxbus.isConfigured()).toBe(false);
        expect(muxbus.error()).toMatch(/not configured/);
        expect(rpc.MuxBusLoginCommand).not.toHaveBeenCalled();
    });
});

describe("useMuxBusStatus across controllers", () => {
    afterEach(() => {
        vi.unstubAllEnvs();
        vi.clearAllMocks();
    });

    // The status bar's MuxBus dot and the version panel each own a controller;
    // a sign-in through one must update the other at once.
    it("a successful sign-in through one controller refreshes the others", async () => {
        const SIGNED_IN = { ...SIGNED_OUT, connected: true, valid: true, email: "a@example.test", expiresAt: 9e9 };
        vi.stubEnv("VITE_MUXBUS_CLIENT_ID", "compiled-id");
        vi.stubEnv("VITE_MUXBUS_COGNITO_DOMAIN", "https://auth.compiled.test");
        vi.resetModules();
        const { useMuxBusStatus } = await import("./AgentMuxConnectPanel");
        rpc.MuxBusStatusCommand.mockResolvedValue(SIGNED_OUT);
        const panel = useMuxBusStatus();
        const dot = useMuxBusStatus();
        await dot.refresh();
        expect(dot.status()?.connected).toBe(false);

        rpc.MuxBusStatusCommand.mockResolvedValue(SIGNED_IN);
        rpc.MuxBusLoginCommand.mockResolvedValue({ success: true, email: "a@example.test" });
        await panel.connect();
        await vi.waitFor(() => expect(dot.status()?.connected).toBe(true));
    });
});
