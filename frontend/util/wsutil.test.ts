// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";
import { newWebSocket } from "./wsutil";

describe("newWebSocket", () => {
    const made: { url: string; protocols?: string | string[] }[] = [];

    beforeEach(() => {
        made.length = 0;
        vi.stubGlobal(
            "WebSocket",
            class {
                constructor(url: string, protocols?: string | string[]) {
                    made.push({ url, protocols });
                }
            }
        );
    });

    afterEach(() => {
        vi.unstubAllGlobals();
        window.api = undefined as unknown as AppApi;
    });

    it("puts the key in the query string, as before", () => {
        window.api = makeTestHostApi({ getSessionToken: () => "tok" });
        newWebSocket("ws://h/ws?tabid=1", { "X-AuthKey": "k 1" });
        expect(made).toEqual([{ url: "ws://h/ws?tabid=1&authkey=k%201", protocols: undefined }]);
    });

    it("offers a signed-in browser's session as the second protocol, never in the URL", () => {
        window.api = makeTestHostApi({ getSessionToken: () => "tok" });
        newWebSocket("ws://h/ws?tabid=1", null);
        expect(made).toEqual([{ url: "ws://h/ws?tabid=1", protocols: ["amx-session", "tok"] }]);
    });

    it("offers no protocol when the host has no session, or there is no host yet", () => {
        window.api = makeTestHostApi({ getSessionToken: () => undefined });
        newWebSocket("ws://h/ws", null);
        window.api = undefined as unknown as AppApi;
        newWebSocket("ws://h/ws", null);
        expect(made.map((m) => m.protocols)).toEqual([undefined, undefined]);
    });
});
