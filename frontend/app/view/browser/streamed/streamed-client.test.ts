// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamedEvents } from "./streamed-client";

type FakeSocket = { url: string; readyState: number; sent: string[]; closed: number; onopen?: () => void; onclose?: unknown; onerror?: unknown; onmessage?: unknown; binaryType?: string; send(s: string): void; close(): void };
const sockets: FakeSocket[] = [];
vi.mock("@/util/wsutil", () => ({
    newWebSocket: (url: string) => {
        const s: FakeSocket = {
            url,
            readyState: 0,
            sent: [],
            closed: 0,
            send(m) {
                this.sent.push(m);
            },
            close() {
                this.closed++;
            },
        };
        sockets.push(s);
        return s;
    },
}));

let outcome: Promise<{ ok: true } | { ok: false; message: string; aborted?: boolean }> = Promise.resolve({ ok: true });
let startingCalls: (() => void) | undefined;
const asked: AbortSignal[] = [];
vi.mock("./lease-step", () => ({
    ensureLease: (o: { signal: AbortSignal; onStarting: () => void }) => {
        asked.push(o.signal);
        startingCalls = o.onStarting;
        return outcome;
    },
    releaseOnPageHide: () => {},
}));

import { openStreamedPane, screenModeAsked, streamedUrl } from "./streamed-client";

function events(): StreamedEvents & { log: string[] } {
    const log: string[] = [];
    return {
        log,
        onFrame: () => log.push("frame"),
        onMessage: (m) => log.push(`message:${m.t}`),
        onOpen: () => log.push("open"),
        onEnd: (how, reason) => log.push(`end:${how}${reason ? `:${reason}` : ""}`),
        onStarting: () => log.push("starting"),
    };
}

const start = { url: "https://a.test/", w: 800, h: 600 };
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    vi.stubGlobal("window", { location: { origin: "https://host.test" } });
    vi.stubGlobal("WebSocket", { OPEN: 1 });
});
afterEach(() => {
    sockets.length = 0;
    asked.length = 0;
    outcome = Promise.resolve({ ok: true });
    vi.unstubAllGlobals();
});

describe("screen mode", () => {
    it("is asked for with mode=screen, and an ordinary pane's address is unchanged", () => {
        expect(streamedUrl("wss://host.test", start, true)).toContain("mode=screen");
        expect(streamedUrl("wss://host.test", start)).not.toContain("mode=");
        expect(streamedUrl("wss://host.test", start, false)).toBe(streamedUrl("wss://host.test", start));
    });

    it("is asked for by the page's ?screen=1 or the local setting, and by nothing else", () => {
        vi.stubGlobal("window", { location: { search: "" }, localStorage: { getItem: () => null } });
        expect(screenModeAsked()).toBe(false);
        vi.stubGlobal("window", { location: { search: "?screen=1" }, localStorage: { getItem: () => null } });
        expect(screenModeAsked()).toBe(true);
        vi.stubGlobal("window", { location: { search: "" }, localStorage: { getItem: (k: string) => (k === "streamedBrowserScreen" ? "1" : null) } });
        expect(screenModeAsked()).toBe(true);
        vi.stubGlobal("window", { location: { search: "?screen=0" }, localStorage: { getItem: () => "0" } });
        expect(screenModeAsked()).toBe(false);
    });
});

describe("a streamed pane's socket", () => {
    it("asks the server for the browser first, and opens the socket only once it says it is ready", async () => {
        let ready!: () => void;
        outcome = new Promise((r) => (ready = () => r({ ok: true })));
        const ev = events();
        openStreamedPane(start, ev);
        await settle();
        expect(sockets).toHaveLength(0);
        startingCalls?.();
        expect(ev.log).toEqual(["starting"]);
        ready();
        await settle();
        expect(sockets).toHaveLength(1);
        expect(sockets[0].url).toBe(streamedUrl("wss://host.test", start));
    });

    it("asks for the full browser the person drives when the pane says so, whatever the page and the setting say", async () => {
        vi.stubGlobal("window", { location: { origin: "https://host.test", search: "" }, localStorage: { getItem: () => null } });
        openStreamedPane({ ...start, screen: true }, events());
        await settle();
        expect(sockets[0].url).toContain("mode=screen");
    });

    it("honours an explicit choice of the ordinary browser even when the page asks for the full one (the pane's fallback)", async () => {
        vi.stubGlobal("window", { location: { origin: "https://host.test", search: "?screen=1" }, localStorage: { getItem: () => null } });
        openStreamedPane({ ...start, screen: false }, events());
        await settle();
        expect(sockets[0].url).not.toContain("mode=screen");
    });

    it("carries frames and messages as before once the socket is open", async () => {
        const ev = events();
        const h = openStreamedPane(start, ev);
        await settle();
        const s = sockets[0];
        s.readyState = 1;
        s.onopen?.();
        expect(ev.log).toEqual(["open"]);
        h.send({ t: "nav", go: "reload" });
        expect(s.sent).toEqual([JSON.stringify({ t: "nav", go: "reload" })]);
    });

    it("ends as failed, with the server's words, and never opens a socket, when the server will not hand out a browser", async () => {
        outcome = Promise.resolve({ ok: false, message: "The server's browser is full right now." });
        const ev = events();
        openStreamedPane(start, ev);
        await settle();
        expect(ev.log).toEqual(["end:failed:The server's browser is full right now."]);
        expect(sockets).toHaveLength(0);
    });

    it("sends nothing while it is waiting, and opens nothing when the pane goes before the server answers", async () => {
        let ready!: () => void;
        outcome = new Promise((r) => (ready = () => r({ ok: true })));
        const ev = events();
        const h = openStreamedPane(start, ev);
        h.send({ t: "nav", go: "reload" });
        h.close();
        expect(asked[0].aborted).toBe(true);
        ready();
        await settle();
        expect(sockets).toHaveLength(0);
        expect(ev.log).toEqual([]);
    });

    it("closes the socket it opened, without reporting it, when the pane goes", async () => {
        const ev = events();
        const h = openStreamedPane(start, ev);
        await settle();
        h.close();
        expect(sockets[0].closed).toBe(1);
        expect(sockets[0].onclose).toBeNull();
        expect(ev.log).toEqual([]);
    });
});
