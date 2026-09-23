// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// cdp-client.mjs against a real local WebSocket server standing in for a CDP
// target. Each failure mode here is one that made a #3569 measurement script
// hang or report a failed call as success.

import { afterEach, describe, expect, it } from "vitest";
import { CdpError, PRODUCTION_CDP_PORT, assertDevPort, cdpSession, nodeWs } from "./cdp-client.mjs";
import { parsePs, parseWindows, processMemory } from "./process-memory.mjs";

// The Node build of ws, not the browser stub vitest would otherwise resolve.
const WebSocket = nodeWs();
const { WebSocketServer } = WebSocket;

const servers = [];
afterEach(async () => {
    while (servers.length) await new Promise((r) => servers.pop().close(() => r()));
});

/** A fake target: `handler(msg, socket)` decides how to answer each command. */
async function fakeTarget(handler) {
    const wss = new WebSocketServer({ port: 0, host: "127.0.0.1" });
    servers.push(wss);
    await new Promise((r) => wss.once("listening", r));
    wss.on("connection", (socket) => {
        socket.on("message", (raw) => handler(JSON.parse(raw), socket));
    });
    const ws = new WebSocket(`ws://127.0.0.1:${wss.address().port}`);
    await new Promise((r, e) => {
        ws.once("open", r);
        ws.once("error", e);
    });
    return { ws, wss };
}

describe("cdpSession", () => {
    it("resolves with the result of a successful command", async () => {
        const { ws } = await fakeTarget((m, s) => s.send(JSON.stringify({ id: m.id, result: { echo: m.method } })));
        const cdp = cdpSession(ws);
        expect(await cdp.send("Page.enable")).toEqual({ echo: "Page.enable" });
        await cdp.close();
    });

    it("rejects on an error response instead of resolving", async () => {
        const { ws } = await fakeTarget((m, s) =>
            s.send(JSON.stringify({ id: m.id, error: { code: -32000, message: "Tracing already started" } }))
        );
        const cdp = cdpSession(ws);
        await expect(cdp.send("Tracing.start")).rejects.toThrow("Tracing.start: Tracing already started");
        await cdp.close();
    });

    it("rejects everything pending when the socket closes", async () => {
        const { ws } = await fakeTarget((m, s) => {
            if (m.method === "Hang") s.close();
        });
        const cdp = cdpSession(ws);
        await expect(cdp.send("Hang")).rejects.toThrow(CdpError);
        expect(cdp.closed).toBe(true);
    });

    it("rejects a send after the socket has closed, rather than waiting forever", async () => {
        const { ws } = await fakeTarget((m, s) => s.close());
        const cdp = cdpSession(ws);
        await expect(cdp.send("First")).rejects.toThrow();
        await expect(cdp.send("Second")).rejects.toThrow("Second: CDP socket is not open");
    });

    it("times out a command that never gets an answer", async () => {
        const { ws } = await fakeTarget(() => {});
        const cdp = cdpSession(ws, { timeoutMs: 50 });
        await expect(cdp.send("Silent")).rejects.toThrow("Silent: no response within 50 ms");
        await cdp.close();
    });

    it("evaluate returns the value, and rejects on an in-page exception", async () => {
        const { ws } = await fakeTarget((m, s) => {
            const expr = m.params.expression;
            if (expr === "throw") {
                s.send(
                    JSON.stringify({
                        id: m.id,
                        result: { result: {}, exceptionDetails: { exception: { description: "Error: boom" } } },
                    })
                );
            } else {
                s.send(JSON.stringify({ id: m.id, result: { result: { value: 42 } } }));
            }
        });
        const cdp = cdpSession(ws);
        expect(await cdp.evaluate("40+2")).toBe(42);
        await expect(cdp.evaluate("throw")).rejects.toThrow("Error: boom");
        await cdp.close();
    });

    it("delivers events to listeners", async () => {
        const { ws } = await fakeTarget((m, s) => {
            s.send(JSON.stringify({ method: "Tracing.dataCollected", params: { value: [1] } }));
            s.send(JSON.stringify({ id: m.id, result: {} }));
        });
        const cdp = cdpSession(ws);
        const seen = [];
        cdp.on("Tracing.dataCollected", (p) => seen.push(p));
        await cdp.send("Tracing.end");
        expect(seen).toEqual([{ value: [1] }]);
        await cdp.close();
    });
});

describe("assertDevPort", () => {
    it("refuses the production instance unless allowed", () => {
        expect(() => assertDevPort(PRODUCTION_CDP_PORT)).toThrow(/production AgentMux instance/);
        expect(() => assertDevPort(PRODUCTION_CDP_PORT, { allowProduction: true })).not.toThrow();
        expect(() => assertDevPort(9223)).not.toThrow();
    });
});

describe("process memory", () => {
    it("parses PowerShell output", () => {
        const m = parseWindows("1234 104857600 52428800\r\n5678 2048 1024\r\n\r\n");
        expect(m.get(1234)).toEqual({ rssBytes: 104857600, privateBytes: 52428800 });
        expect(m.get(5678)).toEqual({ rssBytes: 2048, privateBytes: 1024 });
        expect(m.size).toBe(2);
    });

    it("parses ps output (KiB)", () => {
        const m = parsePs("  1234  102400\n 5678 4\n");
        expect(m.get(1234)).toEqual({ rssBytes: 102400 * 1024, privateBytes: null });
        expect(m.get(5678).rssBytes).toBe(4096);
    });

    it("reports this test process's own memory on the current OS", () => {
        const m = processMemory([process.pid]);
        expect(m.get(process.pid)?.rssBytes).toBeGreaterThan(1024 * 1024);
    });

    it("ignores ids that are not processes", () => {
        expect(processMemory([]).size).toBe(0);
        expect(processMemory([0, -1, 1.5]).size).toBe(0);
    });
});
