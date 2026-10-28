// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn() }));
vi.mock("@/util/fetchutil", () => ({ fetch: (...a: unknown[]) => h.fetch(...a) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));
vi.mock("./auth-headers", () => ({ authHeaders: () => ({}) }));

import { fetchMuxFile } from "./mux-file";

function reply(headers: Record<string, string>, body: string) {
    const info = btoa(JSON.stringify({ zoneid: "z", name: "term", size: 9000, opts: {}, meta: {} }));
    return new Response(body, { status: 200, headers: { "X-ZoneFileInfo": info, ...headers } });
}

describe("fetchMuxFile", () => {
    beforeEach(() => h.fetch.mockReset());

    it("reports where the body starts when srv says", async () => {
        h.fetch.mockResolvedValue(reply({ "X-ZoneFileOffset": "8000" }, "abc"));
        const got = await fetchMuxFile("z", "term", 10);
        expect(got.dataOffset).toBe(8000);
        expect(got.data.byteLength).toBe(3);
    });

    it("has no offset from an older srv or an unreadable header", async () => {
        h.fetch.mockResolvedValueOnce(reply({}, "abc"));
        expect((await fetchMuxFile("z", "term")).dataOffset).toBeUndefined();
        h.fetch.mockResolvedValueOnce(reply({ "X-ZoneFileOffset": "soon" }, "abc"));
        expect((await fetchMuxFile("z", "term")).dataOffset).toBeUndefined();
    });
});
