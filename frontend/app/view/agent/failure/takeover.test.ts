// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/store/global", () => ({ getApi: () => undefined }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));

import { requestAgentTakeover } from "./takeover";

function respond(status: number, body: unknown) {
    return vi.fn(async () => new Response(JSON.stringify(body), { status }));
}

describe("requestAgentTakeover", () => {
    it("posts the block to the takeover route with the auth key", async () => {
        const fetchImpl = respond(200, { ok: true, released: true, from_channel: "stable" });
        const r = await requestAgentTakeover("block-1", { fetchImpl, endpoint: "http://srv", authKey: "k" });
        expect(r).toEqual({ released: true, fromChannel: "stable" });
        const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
        expect(url).toBe("http://srv/api/v1/agent/takeover");
        expect(init.method).toBe("POST");
        expect((init.headers as Record<string, string>)["X-AuthKey"]).toBe("k");
        expect(JSON.parse(init.body as string)).toEqual({ block_id: "block-1" });
    });

    it("reports nothing to take over as released: false", async () => {
        const r = await requestAgentTakeover("b", { fetchImpl: respond(200, { ok: true, released: false }), authKey: "k" });
        expect(r.released).toBe(false);
    });

    it("rejects with the srv's own reason", async () => {
        const fetchImpl = respond(409, { error: "The AgentMux instance running Agent3 (channel x) is too old to hand it over." });
        await expect(requestAgentTakeover("b", { fetchImpl, authKey: "k" })).rejects.toThrow("too old to hand it over");
    });

    it("falls back to the status when the body is not JSON", async () => {
        const fetchImpl = vi.fn(async () => new Response("nope", { status: 504 }));
        await expect(requestAgentTakeover("b", { fetchImpl, authKey: "k" })).rejects.toThrow("HTTP 504");
    });
});
