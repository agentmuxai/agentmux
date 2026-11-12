// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The browser host's startup: srv is reached on the page's own origin, and window.api is the browser AppApi.

import { afterEach, describe, expect, it } from "vitest";
import { endpointsFor, hostModule } from "./web-host-module";

afterEach(() => {
    delete (window as any).api;
    delete window.__WAVE_SERVER_WEB_ENDPOINT__;
    delete window.__WAVE_SERVER_WS_ENDPOINT__;
});

describe("the browser host module", () => {
    it("derives the WebSocket endpoint from the page's origin", () => {
        expect(endpointsFor("http://localhost:8180")).toEqual({
            web: "http://localhost:8180",
            ws: "ws://localhost:8180",
        });
        expect(endpointsFor("https://box.example")).toEqual({ web: "https://box.example", ws: "wss://box.example" });
    });

    it("points srv's endpoints at the page's own origin and installs window.api", async () => {
        await hostModule.setup();
        expect(window.__WAVE_SERVER_WEB_ENDPOINT__).toBe(window.location.origin);
        expect(window.__WAVE_SERVER_WS_ENDPOINT__).toBe(window.location.origin.replace(/^http/, "ws"));
        expect(window.api.getHostCaps()).toBeDefined();
        expect(window.api.getAuthKey()).toBe("");
    });
});
