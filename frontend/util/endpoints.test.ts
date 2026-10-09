// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { getWebServerEndpoint, getWSServerEndpoint } from "./endpoints";

describe("backend endpoints", () => {
    afterEach(() => {
        delete window.__WAVE_SERVER_WEB_ENDPOINT__;
        delete window.__WAVE_SERVER_WS_ENDPOINT__;
    });

    it("gives a bare host:port http and ws, as the desktop host sets them", () => {
        window.__WAVE_SERVER_WEB_ENDPOINT__ = "127.0.0.1:8190";
        window.__WAVE_SERVER_WS_ENDPOINT__ = "127.0.0.1:8191";
        expect(getWebServerEndpoint()).toBe("http://127.0.0.1:8190");
        expect(getWSServerEndpoint()).toBe("ws://127.0.0.1:8191");
    });

    it("uses a full origin as it is, without a trailing slash", () => {
        window.__WAVE_SERVER_WEB_ENDPOINT__ = "https://app.example.com/";
        window.__WAVE_SERVER_WS_ENDPOINT__ = "wss://app.example.com";
        expect(getWebServerEndpoint()).toBe("https://app.example.com");
        expect(getWSServerEndpoint()).toBe("wss://app.example.com");
    });
});
