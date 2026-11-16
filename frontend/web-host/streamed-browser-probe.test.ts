// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { streamedBrowserAvailable } from "./streamed-browser-probe";

const answering = (status: number) => async () => ({ status });

describe("streamedBrowserAvailable", () => {
    it("is true only for the answer a switched-on server gives a plain request", async () => {
        expect(await streamedBrowserAvailable("tok", answering(426))).toBe(true);
    });

    it("is false when the server was started without it, refuses the tab, or cannot be asked", async () => {
        expect(await streamedBrowserAvailable("tok", answering(404))).toBe(false);
        expect(await streamedBrowserAvailable("tok", answering(401))).toBe(false);
        expect(await streamedBrowserAvailable(undefined, answering(200))).toBe(false);
        expect(await streamedBrowserAvailable("tok", () => Promise.reject(new Error("offline")))).toBe(false);
    });

    it("sends the tab's session token as the header the server reads", async () => {
        let seen: Record<string, string> = {};
        await streamedBrowserAvailable("tok123", async (_url: string, init: RequestInit) => {
            seen = init.headers as Record<string, string>;
            return { status: 426 };
        });
        expect(Object.values(seen)).toEqual(["tok123"]);
    });
});
