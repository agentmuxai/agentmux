// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { backoffMs, createGitHubClient, GitHubError, retryDelayMs } from "./github-rest.mjs";

const NOW = 1_800_000_000_000;

/** A client whose fetch answers from `script` in order (a Response, or an Error to throw). */
function scripted(script, opts = {}) {
    const calls = [];
    const waits = [];
    const logs = [];
    const client = createGitHubClient({
        token: "t",
        fetch: async (url, init) => {
            calls.push({ url, init });
            const next = script.shift();
            if (next === undefined) throw new Error("fetch called more times than scripted");
            if (next instanceof Error) throw next;
            return next;
        },
        sleep: async (ms) => waits.push(ms),
        log: (line) => logs.push(line),
        now: () => NOW,
        random: () => 0,
        ...opts,
    });
    return { client, calls, waits, logs };
}

const status = (code, headers = {}, body = "") => new Response(body, { status: code, headers });
const ok = (body) => new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });

describe("createGitHubClient request", () => {
    it("retries a 500 and returns the response that follows", async () => {
        const { client, waits, logs } = scripted([status(500), ok({ a: 1 })]);
        expect(await client.json("/x")).toEqual({ a: 1 });
        expect(waits).toEqual([2000]);
        expect(logs).toEqual(["GET /x -> HTTP 500; retry 2/4 in 2.0s"]);
    });

    it("retries each of 502, 503 and 504", async () => {
        const { client, waits } = scripted([status(502), status(503), status(504), ok([])]);
        expect(await client.json("/x")).toEqual([]);
        expect(waits).toEqual([2000, 4000, 8000]);
    });

    it("gives up after four attempts with the last HTTP error", async () => {
        const { client, calls } = scripted([status(500), status(500), status(500), status(500, {}, "boom")]);
        const err = await client.request("/x", { method: "POST" }).catch((e) => e);
        expect(err).toBeInstanceOf(GitHubError);
        expect(err).toMatchObject({ status: 500, method: "POST", path: "/x" });
        expect(err.message).toBe("POST /x -> HTTP 500: boom");
        expect(calls).toHaveLength(4);
    });

    it("does not retry a 404 or a 422", async () => {
        for (const code of [404, 422]) {
            const { client, calls } = scripted([status(code)]);
            await expect(client.request("/x")).rejects.toMatchObject({ status: code });
            expect(calls).toHaveLength(1);
        }
    });

    it("does not retry a 403 that is not a rate limit", async () => {
        const { client, calls } = scripted([status(403, {}, "Resource not accessible by integration")]);
        await expect(client.request("/x")).rejects.toMatchObject({ status: 403 });
        expect(calls).toHaveLength(1);
    });

    it("retries when fetch throws, and rethrows naming the call once out of attempts", async () => {
        const { client } = scripted([new TypeError("fetch failed"), ok({ a: 1 })]);
        expect(await client.json("/x")).toEqual({ a: 1 });

        const failing = scripted([1, 2, 3, 4].map(() => new TypeError("fetch failed")));
        const err = await failing.client.request("/x").catch((e) => e);
        expect(err.message).toBe("GET /x failed: fetch failed");
        expect(err.cause).toBeInstanceOf(TypeError);
    });

    it("waits as long as retry-after says, within maxWaitMs", async () => {
        const { client, waits } = scripted([status(429, { "retry-after": "3" }), ok({})]);
        await client.request("/x");
        expect(waits).toEqual([3000]);
    });

    it("does not wait longer than maxWaitMs", async () => {
        const { client, calls } = scripted([status(429, { "retry-after": "600" })]);
        await expect(client.request("/x")).rejects.toMatchObject({ status: 429 });
        expect(calls).toHaveLength(1);
    });

    it("waits for a spent rate limit to reset", async () => {
        const reset = String(NOW / 1000 + 5);
        const { client, waits } = scripted([status(403, { "x-ratelimit-remaining": "0", "x-ratelimit-reset": reset }), ok({})]);
        await client.request("/x");
        expect(waits).toEqual([6000]);
    });

    it("never retries with retry: false", async () => {
        const { client, calls } = scripted([status(500)]);
        await expect(client.request("/x", {}, { retry: false })).rejects.toMatchObject({ status: 500 });
        expect(calls).toHaveLength(1);
    });

    it("sends the token, the API version and the caller's own headers", async () => {
        const { client, calls } = scripted([ok({})]);
        await client.request("/repos/o/r/statuses/abc", { method: "POST", headers: { "Content-Type": "application/json" } });
        expect(calls[0].url).toBe("https://api.github.com/repos/o/r/statuses/abc");
        expect(calls[0].init.headers).toMatchObject({
            Authorization: "Bearer t",
            "X-GitHub-Api-Version": "2022-11-28",
            "Content-Type": "application/json",
        });
    });
});

describe("createGitHubClient paginate", () => {
    const page = (n, from = 0) => Array.from({ length: n }, (_, i) => ({ i: from + i }));

    it("reads pages until a short one", async () => {
        const { client, calls } = scripted([ok(page(100)), ok(page(3, 100))]);
        expect(await client.paginate("/items")).toHaveLength(103);
        expect(calls.map((c) => c.url)).toEqual([
            "https://api.github.com/items?per_page=100&page=1",
            "https://api.github.com/items?per_page=100&page=2",
        ]);
    });

    it("appends to an existing query string", async () => {
        const { client, calls } = scripted([ok([])]);
        await client.paginate("/items?state=open");
        expect(calls[0].url).toBe("https://api.github.com/items?state=open&per_page=100&page=1");
    });

    it("retries a page that fails transiently instead of dropping it", async () => {
        const { client } = scripted([ok(page(100)), status(502), ok(page(1, 100))]);
        expect(await client.paginate("/items")).toHaveLength(101);
    });
});

describe("retryDelayMs", () => {
    const res = (code, headers = {}) => status(code, headers);

    it("backs off 2 s, 4 s, 8 s with up to a second of jitter", () => {
        expect([1, 2, 3].map((a) => backoffMs(a, () => 0))).toEqual([2000, 4000, 8000]);
        expect(backoffMs(1, () => 0.999)).toBe(2999);
    });

    it("backs off a 429 that names no wait", () => {
        expect(retryDelayMs(res(429), 1, { random: () => 0 })).toBe(2000);
    });

    it("never retries other client errors", () => {
        for (const code of [400, 401, 403, 404, 409, 422]) expect(retryDelayMs(res(code), 1)).toBeNull();
    });

    it("ignores a reset it cannot read", () => {
        expect(retryDelayMs(res(403, { "x-ratelimit-remaining": "0", "x-ratelimit-reset": "soon" }), 1)).toBeNull();
    });
});
