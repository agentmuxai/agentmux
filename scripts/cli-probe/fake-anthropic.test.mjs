// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { messageBody, startFakeAnthropic, streamEvents } from "./fake-anthropic.mjs";

let api;
afterEach(async () => {
    await api?.close();
    api = undefined;
});

const post = (path, body) =>
    fetch(api.url + path, { method: "POST", headers: { "content-type": "application/json", "x-api-key": "k" }, body: JSON.stringify(body) });

describe("fake Anthropic API", () => {
    it("reports back the model the client SENT, which is the point: the request is the ground truth", async () => {
        api = await startFakeAnthropic({ reply: "pong" });
        const res = await post("/v1/messages", { model: "claude-sonnet-5-5", max_tokens: 5, messages: [] });
        const json = await res.json();
        expect(json.model).toBe("claude-sonnet-5-5");
        expect(json.content[0].text).toBe("pong");
        expect(api.requests).toHaveLength(1);
        expect(api.requests[0]).toMatchObject({ method: "POST", path: "/v1/messages", model: "claude-sonnet-5-5", stream: false, hasApiKey: true });
    });

    it("streams a well-formed event sequence when asked to", async () => {
        api = await startFakeAnthropic();
        const res = await post("/v1/messages", { model: "m", stream: true, messages: [] });
        expect(res.headers.get("content-type")).toContain("text/event-stream");
        const text = await res.text();
        const types = [...text.matchAll(/^event: (.+)$/gm)].map((m) => m[1]);
        expect(types).toEqual([
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ]);
        // each data line is JSON naming the same type as its event line
        for (const m of text.matchAll(/event: (.+)\ndata: (.+)\n/g)) expect(JSON.parse(m[2]).type).toBe(m[1]);
        expect(api.requests[0].stream).toBe(true);
    });

    it("records the keys of the body (to see which settings the CLI passed on) and survives a non-JSON body", async () => {
        api = await startFakeAnthropic();
        await post("/v1/messages", { model: "m", max_tokens: 1, thinking: { type: "enabled" }, messages: [] });
        await fetch(api.url + "/v1/messages", { method: "POST", body: "not json" });
        expect(api.requests[0].bodyKeys).toEqual(["max_tokens", "messages", "model", "thinking"]);
        expect(api.requests[1]).toMatchObject({ model: undefined, bodyKeys: [] });
    });

    it("records, and 404s, routes it does not know - so a probe sees everything the CLI tries to reach", async () => {
        api = await startFakeAnthropic();
        const res = await fetch(api.url + "/api/oauth/profile?x=1");
        expect(res.status).toBe(404);
        expect((await res.json()).error.type).toBe("not_found_error");
        expect(api.requests[0]).toMatchObject({ method: "GET", path: "/api/oauth/profile" });
    });

    it("answers count_tokens", async () => {
        api = await startFakeAnthropic();
        const res = await post("/v1/messages/count_tokens", { model: "m", messages: [] });
        expect((await res.json()).input_tokens).toBe(10);
    });

    it("builds the same reply in both shapes", () => {
        expect(streamEvents("m", "hi")[2].delta.text).toBe("hi");
        expect(messageBody("m", "hi").content[0].text).toBe("hi");
    });
});
