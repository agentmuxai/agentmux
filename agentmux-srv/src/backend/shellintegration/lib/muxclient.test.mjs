// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

import { agentmuxFetch, readAgentmuxEnv } from "./muxclient.mjs";

describe("readAgentmuxEnv", () => {
    it("reads both vars from the given env object", () => {
        expect(readAgentmuxEnv({ AGENTMUX_LOCAL_URL: "http://127.0.0.1:1", AGENTMUX_AUTH_KEY: "k" })).toEqual({
            url: "http://127.0.0.1:1",
            authKey: "k",
        });
    });

    it("returns undefined fields rather than throwing when absent", () => {
        expect(readAgentmuxEnv({})).toEqual({ url: undefined, authKey: undefined });
    });

    it("defaults to process.env when no argument is given", () => {
        const prevUrl = process.env.AGENTMUX_LOCAL_URL;
        process.env.AGENTMUX_LOCAL_URL = "http://127.0.0.1:2";
        try {
            expect(readAgentmuxEnv().url).toBe("http://127.0.0.1:2");
        } finally {
            if (prevUrl === undefined) delete process.env.AGENTMUX_LOCAL_URL;
            else process.env.AGENTMUX_LOCAL_URL = prevUrl;
        }
    });
});

describe("agentmuxFetch", () => {
    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("GET: sends X-AuthKey, no Content-Type, no body", async () => {
        const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
        vi.stubGlobal("fetch", fetchMock);

        await agentmuxFetch("http://127.0.0.1:1", "k", "/api/v1/self");

        expect(fetchMock).toHaveBeenCalledWith("http://127.0.0.1:1/api/v1/self", {
            method: "GET",
            headers: { "X-AuthKey": "k" },
        });
    });

    it("POST with a body: sends Content-Type and a JSON-stringified body", async () => {
        const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
        vi.stubGlobal("fetch", fetchMock);

        await agentmuxFetch("http://127.0.0.1:1", "k", "/api/v1/pane/open", {
            method: "POST",
            body: { view: "editor", file: "/tmp/x" },
        });

        expect(fetchMock).toHaveBeenCalledWith("http://127.0.0.1:1/api/v1/pane/open", {
            method: "POST",
            headers: { "X-AuthKey": "k", "Content-Type": "application/json" },
            body: JSON.stringify({ view: "editor", file: "/tmp/x" }),
        });
    });

    it("strips a trailing slash from the base URL before joining the path", async () => {
        const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
        vi.stubGlobal("fetch", fetchMock);

        await agentmuxFetch("http://127.0.0.1:1/", "k", "/api/v1/self");

        expect(fetchMock).toHaveBeenCalledWith("http://127.0.0.1:1/api/v1/self", expect.anything());
    });

    it("returns the raw response plus ok/status convenience fields, without reading the body", async () => {
        const rawResp = { ok: false, status: 404, json: vi.fn(), text: vi.fn() };
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(rawResp));

        const result = await agentmuxFetch("http://127.0.0.1:1", "k", "/api/v1/self");

        expect(result).toEqual({ resp: rawResp, ok: false, status: 404 });
        expect(rawResp.json).not.toHaveBeenCalled();
        expect(rawResp.text).not.toHaveBeenCalled();
    });

    it("propagates a network failure (rejects, doesn't swallow it)", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("ECONNREFUSED")));
        await expect(agentmuxFetch("http://127.0.0.1:1", "k", "/api/v1/self")).rejects.toThrow("ECONNREFUSED");
    });
});
