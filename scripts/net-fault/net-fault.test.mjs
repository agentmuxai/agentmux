// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import net from "node:net";
import { afterEach, describe, expect, it } from "vitest";
import {
    createState,
    decide,
    formatMode,
    hostMatches,
    nextChangeIn,
    parseDuration,
    parseMode,
    refusesEverything,
    setMode,
} from "./modes.mjs";
import { splitHostPort, startProxy } from "./proxy.mjs";

describe("modes", () => {
    it("parses durations, with seconds as the bare unit", () => {
        expect(parseDuration("500ms")).toBe(500);
        expect(parseDuration("30s")).toBe(30_000);
        expect(parseDuration("2m")).toBe(120_000);
        expect(parseDuration("1.5")).toBe(1500);
        expect(() => parseDuration("soon")).toThrow(/not a duration/);
        expect(() => parseDuration("0s")).toThrow(/positive/);
    });

    it("parses every mode, and a flap's down mode defaults to refuse", () => {
        for (const m of ["pass", "refuse", "blackhole", "dns"]) expect(parseMode(m)).toEqual({ kind: m });
        expect(parseMode("flap:30s/20s")).toEqual({ kind: "flap", upMs: 30_000, downMs: 20_000, down: "refuse" });
        expect(parseMode("FLAP:1m/500ms:blackhole")).toEqual({
            kind: "flap",
            upMs: 60_000,
            downMs: 500,
            down: "blackhole",
        });
        expect(formatMode(parseMode("flap:30s/20s"))).toBe("flap:30s/20s");
        expect(formatMode(parseMode("flap:1m/500ms:dns"))).toBe("flap:60s/500ms:dns");
        expect(() => parseMode("offline")).toThrow(/unknown mode/);
        expect(() => parseMode("flap:30s/20s:pass")).toThrow(/down mode/);
    });

    it("matches host globs case-insensitively, with * spanning dots", () => {
        expect(hostMatches("api.anthropic.com", "API.Anthropic.com")).toBe(true);
        expect(hostMatches("*.anthropic.com", "api.anthropic.com")).toBe(true);
        expect(hostMatches("*.anthropic.com", "anthropic.com")).toBe(false);
        expect(hostMatches("*anthropic*", "a.b.anthropic.com")).toBe(true);
        expect(hostMatches("api.?nthropic.com", "api.anthropic.com")).toBe(true);
        expect(hostMatches("api.anthropic.com", "apixanthropic.com")).toBe(false);
    });

    it("tries rules in order, falls back to the default, and replaces a rule for the same glob in place", () => {
        const s = createState("pass", 0);
        setMode(s, "refuse", "api.example.com", 0);
        setMode(s, "dns", "*.example.com", 0);
        expect(decide(s, "api.example.com", 0)).toBe("refuse");
        expect(decide(s, "bus.example.com", 0)).toBe("dns");
        expect(decide(s, "other.org", 0)).toBe("pass");
        setMode(s, "pass", "API.example.com", 0);
        expect(s.rules.map((r) => r.host)).toEqual(["api.example.com", "*.example.com"]);
        expect(decide(s, "api.example.com", 0)).toBe("pass");
    });

    it("resolves a flap by its phase, up first, and says when it next changes", () => {
        const s = createState("flap:30s/20s", 1000);
        expect(decide(s, "h", 1000)).toBe("pass");
        expect(nextChangeIn(s, 1000)).toBe(30_000);
        expect(decide(s, "h", 31_000)).toBe("refuse");
        expect(nextChangeIn(s, 31_000)).toBe(20_000);
        expect(decide(s, "h", 51_000)).toBe("pass");
        expect(nextChangeIn(createState("refuse"), 0)).toBeNull();
    });

    it("refuses everything only when the default and every rule refuse right now", () => {
        const s = createState("refuse", 0);
        expect(refusesEverything(s, 0)).toBe(true);
        setMode(s, "pass", "bus.example.com", 0);
        expect(refusesEverything(s, 0)).toBe(false);
        const flap = createState("flap:1s/1s", 0);
        expect(refusesEverything(flap, 0)).toBe(false);
        expect(refusesEverything(flap, 1500)).toBe(true);
    });

    it("splits CONNECT targets, IPv6 included", () => {
        expect(splitHostPort("API.example.com:443")).toEqual({ host: "api.example.com", port: 443 });
        expect(splitHostPort("[::1]:8443")).toEqual({ host: "::1", port: 8443 });
        expect(splitHostPort("example.com")).toEqual({ host: "example.com", port: 443 });
    });
});

/** A CONNECT through the proxy: the answer's status line, a socket error code, or a timeout. */
function connectVia(proxyPort, target, timeoutMs = 1500) {
    return new Promise((resolve) => {
        const socket = net.connect(proxyPort, "127.0.0.1");
        let buf = "";
        const done = (result) => {
            clearTimeout(timer);
            socket.removeAllListeners("data");
            resolve({ socket, ...result });
        };
        const timer = setTimeout(() => done({ kind: "timeout" }), timeoutMs);
        socket.on("connect", () => socket.write(`CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\n\r\n`));
        socket.on("data", (d) => {
            buf += d.toString("latin1");
            const end = buf.indexOf("\r\n\r\n");
            if (end < 0) return;
            const status = Number(buf.split(" ")[1]);
            if (status !== 200) socket.once("end", () => done({ kind: "status", status, text: buf }));
            else done({ kind: "status", status, text: buf });
        });
        socket.on("error", (err) => done({ kind: "error", code: err.code }));
        socket.on("close", () => done({ kind: "closed", text: buf }));
    });
}

/** The next data on `socket` within `ms`, or null. */
function nextData(socket, ms = 300) {
    return new Promise((resolve) => {
        const timer = setTimeout(() => {
            socket.off("data", on);
            resolve(null);
        }, ms);
        const on = (d) => {
            clearTimeout(timer);
            resolve(d.toString());
        };
        socket.once("data", on);
    });
}

const closedOrReset = (socket) =>
    new Promise((resolve) => {
        if (socket.destroyed) return resolve("closed");
        socket.once("error", (e) => resolve(e.code));
        socket.once("close", () => resolve("closed"));
    });

describe("fault proxy", () => {
    let proxy;
    let echo;
    let echoTarget;
    const lines = [];
    const sockets = [];
    const track = (r) => (sockets.push(r.socket), r);

    async function start(mode = "pass") {
        echo = net.createServer((s) => {
            s.on("error", () => {});
            s.pipe(s);
        });
        await new Promise((r) => echo.listen(0, "127.0.0.1", r));
        echoTarget = `127.0.0.1:${echo.address().port}`;
        proxy = await startProxy({ port: 0, controlPort: 0, mode, log: (l) => lines.push(l) });
    }

    afterEach(async () => {
        for (const s of sockets.splice(0)) s.destroy();
        await proxy?.close();
        await new Promise((r) => (echo ? echo.close(r) : r()));
        lines.length = 0;
    });

    it("pass tunnels both ways, and logs the decision", async () => {
        await start();
        const r = track(await connectVia(proxy.port, echoTarget));
        expect(r).toMatchObject({ kind: "status", status: 200 });
        r.socket.write("ping");
        expect(await nextData(r.socket)).toBe("ping");
        expect(lines.some((l) => l.includes(`CONNECT ${echoTarget} [pass] -> tunnel open`))).toBe(true);
    });

    it("refuse for one host resets that CONNECT and leaves the others alone", async () => {
        await start();
        await proxy.setMode("refuse", "local*");
        const refused = track(await connectVia(proxy.port, "localhost:443"));
        expect(["ECONNRESET", "EPIPE"]).toContain(refused.code ?? refused.kind);
        const passed = track(await connectVia(proxy.port, echoTarget));
        expect(passed.status).toBe(200);
    });

    it("dns answers 502 with the lookup error Node prints", async () => {
        await start("dns");
        const r = track(await connectVia(proxy.port, "api.example.com:443"));
        expect(r.status).toBe(502);
        expect(r.text).toContain("getaddrinfo ENOTFOUND api.example.com");
    });

    it("blackhole never answers, and a pending CONNECT goes through once back to pass", async () => {
        await start("blackhole");
        const pending = connectVia(proxy.port, echoTarget, 3000);
        await new Promise((r) => setTimeout(r, 300));
        expect(lines.at(-1)).toContain("[blackhole] -> no answer");
        await proxy.setMode("pass");
        const r = track(await pending);
        expect(r.status).toBe(200);
    });

    it("blackhole freezes an open tunnel without closing it, and pass thaws it", async () => {
        await start();
        const r = track(await connectVia(proxy.port, echoTarget));
        await proxy.setMode("blackhole");
        r.socket.write("held");
        expect(await nextData(r.socket)).toBeNull();
        await proxy.setMode("pass");
        expect(await nextData(r.socket, 1000)).toBe("held");
    });

    it("switching to refuse cuts tunnels that are already open", async () => {
        await start();
        const r = track(await connectVia(proxy.port, echoTarget));
        const ended = closedOrReset(r.socket);
        await proxy.setMode("refuse", "127.0.0.1");
        expect(["ECONNRESET", "closed"]).toContain(await ended);
    });

    it("refusing every host stops listening, so connects get ECONNREFUSED, and pass listens again", async () => {
        await start();
        const port = proxy.port;
        await proxy.setMode("refuse");
        expect(proxy.status().listening).toBe(false);
        const refused = track(await connectVia(port, echoTarget, 5000));
        expect(refused.code).toBe("ECONNREFUSED");
        await proxy.setMode("pass");
        const r = track(await connectVia(port, echoTarget));
        expect(r.status).toBe(200);
    }, 15_000);

    it("a flap switches by itself on its timer", async () => {
        await start();
        await proxy.setMode("flap:250ms/30s:dns", "127.0.0.1");
        const up = track(await connectVia(proxy.port, echoTarget));
        expect(up.status).toBe(200);
        await new Promise((r) => setTimeout(r, 400));
        const down = track(await connectVia(proxy.port, echoTarget));
        expect(down.status).toBe(502);
        expect(lines.some((l) => l.includes("127.0.0.1=dns (flap:250ms/30s:dns)"))).toBe(true);
    });

    it("the control endpoint sets modes and reports status, and rejects a bad mode", async () => {
        await start();
        const base = `http://127.0.0.1:${proxy.controlPort}`;
        const set = await fetch(`${base}/mode`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ mode: "refuse", host: "api.example.com" }),
        });
        expect(set.status).toBe(200);
        const status = await (await fetch(`${base}/status`)).json();
        expect(status.default).toEqual({ mode: "pass", now: "pass" });
        expect(status.rules).toEqual([{ host: "api.example.com", mode: "refuse", now: "refuse" }]);
        const viaQuery = await fetch(`${base}/mode?mode=blackhole`, { method: "POST" });
        expect((await viaQuery.json()).default.mode).toBe("blackhole");
        const bad = await fetch(`${base}/mode`, { method: "POST", body: JSON.stringify({ mode: "offline" }) });
        expect(bad.status).toBe(400);
        expect((await bad.json()).error).toMatch(/unknown mode/);
        const cleared = await (await fetch(`${base}/clear`, { method: "POST" })).json();
        expect(cleared.rules).toEqual([]);
    });
});
