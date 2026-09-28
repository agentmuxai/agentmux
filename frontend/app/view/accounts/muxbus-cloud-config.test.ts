// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { MuxBusCloudConfigResp } from "@/types/rpc/MuxBusCloudConfigResp";
import { describe, expect, it, vi } from "vitest";
import { createMuxBusCloudConfig } from "./muxbus-cloud-config";

const COMPILED = { cognitoDomain: "https://auth.compiled.test", clientId: "compiled-id" };
const NONE = { cognitoDomain: "https://auth.compiled.test", clientId: "" };

const resp = (over: Partial<MuxBusCloudConfigResp>): MuxBusCloudConfigResp => ({
    source: "default",
    api: "https://relay.example.test",
    ws: "wss://ws.example.test",
    console: "",
    cognitoDomain: "",
    clientId: "",
    region: "",
    userPoolId: "",
    ...over,
});

const DISCOVERED = resp({
    source: "discovered",
    cognitoDomain: "https://auth.example.test",
    clientId: "discovered-id",
});

// SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.5: configured when the build
// has a client id OR `muxbus.cloudconfig` returns one.
describe("muxbus cloud config", () => {
    it("a build with a compiled client id is configured and never asks srv", async () => {
        const fetch = vi.fn(async () => DISCOVERED);
        const config = createMuxBusCloudConfig(COMPILED, fetch);
        expect(config.signIn()).toEqual(COMPILED);
        await config.load();
        expect(fetch).not.toHaveBeenCalled();
        expect(config.signIn()).toEqual(COMPILED);
    });

    it("a build without one is configured once srv discovers a client id", async () => {
        const fetch = vi.fn(async () => DISCOVERED);
        const config = createMuxBusCloudConfig(NONE, fetch);
        expect(config.signIn()).toBeNull();
        await config.load();
        expect(config.signIn()).toEqual({ cognitoDomain: "https://auth.example.test", clientId: "discovered-id" });
        // Known now: later loads don't ask again.
        await config.load();
        expect(fetch).toHaveBeenCalledTimes(1);
    });

    it("stays unconfigured on srv's defaults or an error, and asks again next time", async () => {
        const fetch = vi
            .fn<() => Promise<MuxBusCloudConfigResp>>()
            .mockResolvedValueOnce(resp({ source: "default" }))
            .mockRejectedValueOnce(new Error("unknown command"))
            .mockResolvedValueOnce(DISCOVERED);
        const config = createMuxBusCloudConfig(NONE, fetch);
        await config.load();
        expect(config.signIn()).toBeNull();
        await expect(config.load()).resolves.toBeUndefined();
        expect(config.signIn()).toBeNull();
        await config.load();
        expect(config.signIn()?.clientId).toBe("discovered-id");
        expect(fetch).toHaveBeenCalledTimes(3);
    });

    it("concurrent loads share one request", async () => {
        let resolve!: (r: MuxBusCloudConfigResp) => void;
        const fetch = vi.fn(() => new Promise<MuxBusCloudConfigResp>((r) => (resolve = r)));
        const config = createMuxBusCloudConfig(NONE, fetch);
        const a = config.load();
        const b = config.load();
        resolve(DISCOVERED);
        await Promise.all([a, b]);
        expect(fetch).toHaveBeenCalledTimes(1);
        expect(config.signIn()?.clientId).toBe("discovered-id");
    });
});
