// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { MuxObjUpdate: "waveobj:update" } }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://localhost" }));
vi.mock("@/util/fetchutil", () => ({ fetch: () => new Promise(() => {}) }));
vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));

describe("backendCallLogLevel (#47484)", () => {
    it("sends a fast call on the instance panel's timer to the debug level", async () => {
        const { backendCallLogLevel } = await import("./mos");
        expect(backendCallLogLevel("window.ListClosedWindows", 3)).toBe("debug");
        expect(backendCallLogLevel("window.PlaceWindowSlots", 2)).toBe("debug");
    });

    it("logs a slow polled call at INFO again", async () => {
        const { backendCallLogLevel } = await import("./mos");
        expect(backendCallLogLevel("window.ListClosedWindows", 250)).toBe("log");
        expect(backendCallLogLevel("window.PlaceWindowSlots", 900)).toBe("log");
    });

    it("keeps every other call at INFO, however fast", async () => {
        const { backendCallLogLevel } = await import("./mos");
        expect(backendCallLogLevel("object.GetObject", 1)).toBe("log");
        expect(backendCallLogLevel("window.CloseWindow", 1)).toBe("log");
    });
});
