// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { StreamedEvents } from "./streamed-client";

let events: StreamedEvents | null = null;
const sent: object[] = [];
let closed = 0;
vi.mock("./streamed-client", () => ({
    openStreamedPane: (_start: unknown, ev: StreamedEvents) => {
        events = ev;
        return { send: (m: object) => sent.push(m), close: () => void closed++ };
    },
}));

import { useStreamedPane } from "./use-streamed-pane";

function mount() {
    let dispose = () => {};
    const remembered: string[] = [];
    const pane = createRoot((d) => {
        dispose = d;
        return useStreamedPane(() => {}, () => "https://start.test/", (u) => remembered.push(u));
    });
    return { pane, remembered, dispose };
}

afterEach(() => {
    events = null;
    sent.length = 0;
    closed = 0;
    vi.useRealTimers();
});

const nav = (url: string, loading = false) => ({ t: "nav" as const, url, title: "", canGoBack: false, canGoForward: false, loading });

describe("a streamed pane's state", () => {
    it("opens only once it is on screen and has a size", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        vi.advanceTimersByTime(150);
        expect(events).toBeNull();
        pane.resized(800, 600);
        expect(events).toBeNull();
        vi.advanceTimersByTime(150);
        expect(events).not.toBeNull();
        expect(pane.status()).toBe("connecting");
        dispose();
    });

    it("keeps the page showing when the server refuses something, and drops the complaint at the next navigation", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        events!.onOpen();
        events!.onMessage({ t: "error", message: "only http and https addresses can be opened here" });
        expect(pane.status()).toBe("live");
        expect(pane.error()).toContain("only http");
        events!.onMessage(nav("https://a.test/"));
        expect(pane.error()).toBe("");
        events!.onMessage({ t: "error", message: "again" });
        pane.dismissError();
        expect(pane.error()).toBe("");
        dispose();
    });

    it("remembers a page's address once it has loaded, and only a web address", () => {
        vi.useFakeTimers();
        const { pane, remembered, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        events!.onMessage(nav("https://a.test/", true));
        events!.onMessage(nav("https://a.test/", false));
        events!.onMessage(nav("about:blank", false));
        expect(remembered).toEqual(["https://a.test/"]);
        dispose();
    });

    it("lets go of the socket when hidden, and comes back at the page it was on", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        events!.onMessage(nav("https://a.test/page"));
        pane.shown(false);
        expect(closed).toBe(1);
        expect(pane.status()).toBe("paused");
        pane.shown(true);
        expect(pane.status()).toBe("connecting");
        dispose();
    });

    it("sends the newest size once the socket opens when the pane changed size while it was opening", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        pane.resized(1000, 700);
        vi.advanceTimersByTime(150);
        events!.onOpen();
        expect(sent).toContainEqual({ t: "resize", w: 1000, h: 700 });
        dispose();
    });

    it("keeps an address typed while it is still connecting and sends it once the socket opens", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        pane.send({ t: "nav", go: "url", url: "https://a.test/" });
        pane.send({ t: "mouse", type: "down", x: 1, y: 1 });
        expect(sent).toHaveLength(0);
        events!.onOpen();
        expect(sent).toEqual([{ t: "nav", go: "url", url: "https://a.test/" }]);
        pane.send({ t: "nav", go: "reload" });
        expect(sent).toHaveLength(2);
        dispose();
    });

    it("follows a drag: the newest size goes out every interval while the size keeps changing, not once it stops", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        events!.onOpen();
        sent.length = 0;
        for (let w = 810; w <= 850; w += 10) {
            pane.resized(w, 600);
            vi.advanceTimersByTime(40);
        }
        expect(sent.length).toBeGreaterThanOrEqual(1);
        vi.advanceTimersByTime(150);
        expect(sent[sent.length - 1]).toEqual({ t: "resize", w: 850, h: 600 });
        dispose();
    });

    it("does not reopen a failed or ended socket when the pane is resized; Reconnect does", () => {
        vi.useFakeTimers();
        const { pane, dispose } = mount();
        pane.shown(true);
        pane.resized(800, 600);
        vi.advanceTimersByTime(150);
        events!.onOpen();
        events!.onEnd("failed");
        events = null;
        pane.resized(900, 600);
        vi.advanceTimersByTime(150);
        expect(events).toBeNull();
        expect(pane.status()).toBe("failed");
        pane.reconnect();
        expect(events).not.toBeNull();
        dispose();
    });
});
