// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Hangar's system icons: one request per file type and size, never a path;
 * the offered size covers the display scale times the pane's zoom; at most six
 * at once; abandoned when every row that asked has gone; a failure not retried;
 * a 501 switches the whole feature off for the session.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const net = vi.hoisted(() => ({
    urls: [] as string[],
    status: new Map<string, number>(),
    active: 0,
    peak: 0,
    release: [] as (() => void)[],
    hold: false,
    aborted: [] as string[],
}));

vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));
vi.mock("@/app/store/auth-headers", () => ({ authHeaders: () => ({ "X-AuthKey": "k" }) }));
vi.mock("@/util/fetchutil", () => ({
    fetch: (url: string, init: { headers: Record<string, string>; signal: AbortSignal }) => {
        net.urls.push(url);
        expect(init.headers["X-AuthKey"]).toBe("k");
        const respond = () => {
            const status = net.status.get(url) ?? 200;
            return { ok: status === 200, status, body: null, blob: async () => new Blob(["png"]) };
        };
        if (!net.hold) return Promise.resolve(respond());
        return new Promise((resolve, reject) => {
            net.active++;
            net.peak = Math.max(net.peak, net.active);
            init.signal.addEventListener("abort", () => {
                net.active--;
                net.aborted.push(url);
                reject(new DOMException("aborted", "AbortError"));
            });
            net.release.push(() => {
                net.active--;
                resolve(respond());
            });
        });
    },
}));

import {
    cachedIcon,
    iconTarget,
    resetIconsForTest,
    snapPx,
    systemIcon,
    systemIconsAvailable,
    type IconTarget,
} from "./files-icons";

let n = 0;
beforeEach(() => {
    resetIconsForTest();
    net.urls.length = 0;
    net.status.clear();
    net.release.length = 0;
    net.aborted.length = 0;
    net.active = net.peak = 0;
    net.hold = false;
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => `blob:icon-${++n}`, revokeObjectURL: () => {} }));
});
afterEach(() => vi.unstubAllGlobals());

const file = (ext: string): IconTarget => ({ kind: "file", ext });
const ICON = (q: string) => `http://srv/agentmux/file-icon?${q}`;

describe("what is asked for", () => {
    it("is a type, never a path: the extension lower-cased, or nothing for a name srv would refuse", () => {
        expect(iconTarget({ name: "Main.RS", is_dir: false })).toEqual(file("rs"));
        expect(iconTarget({ name: "src", is_dir: true })).toEqual({ kind: "dir", ext: "" });
        expect(iconTarget({ name: "Makefile", is_dir: false })).toEqual(file(""));
        expect(iconTarget({ name: ".gitignore", is_dir: false })).toEqual(file(""));
        expect(iconTarget({ name: "a.tar.gz", is_dir: false })).toEqual(file("gz"));
        expect(iconTarget({ name: "x.é", is_dir: false })).toEqual(file(""));
        expect(iconTarget({ name: "x." + "a".repeat(25), is_dir: false })).toEqual(file(""));
        expect(iconTarget({ name: "main.c++", is_dir: false })).toEqual(file("c++"));
    });

    it("is the offered size that covers the CSS size at the display scale times the pane's zoom, never scaled up", () => {
        expect(snapPx(16, 1)).toBe(16);
        expect(snapPx(16, 1.25)).toBe(20);
        expect(snapPx(16, 1.5)).toBe(24);
        expect(snapPx(16, 2)).toBe(32);
        expect(snapPx(16, 1.1 * 1.25)).toBe(24);
        expect(snapPx(48, 1)).toBe(48);
        expect(snapPx(48, 1.25)).toBe(64);
        expect(snapPx(48, 4)).toBe(256);
        expect(snapPx(16, 0)).toBe(16);
    });
});

describe("fetching", () => {
    it("asks once per type and size however many rows want it, and keeps the answer", async () => {
        const [a, b, c] = await Promise.all([systemIcon(file("ts"), 16), systemIcon(file("ts"), 16), systemIcon(file("ts"), 16)]);
        expect(net.urls).toEqual([ICON("kind=file&ext=ts&px=16")]);
        expect(a).toMatch(/^blob:/);
        expect([b, c]).toEqual([a, a]);
        expect(cachedIcon(file("ts"), 16)).toBe(a);
        expect(await systemIcon(file("ts"), 16)).toBe(a);
        expect(net.urls).toHaveLength(1);
        await systemIcon(file("ts"), 20);
        await systemIcon({ kind: "dir", ext: "" }, 16);
        expect(net.urls).toHaveLength(3);
    });

    it("never has more than six in flight", async () => {
        net.hold = true;
        const all = Array.from({ length: 20 }, (_, i) => systemIcon(file(`t${i}`), 16));
        for (let i = 0; i < 60; i++) {
            await Promise.resolve();
            net.release.shift()?.();
        }
        await Promise.all(all);
        expect(net.peak).toBeLessThanOrEqual(6);
        expect(net.urls).toHaveLength(20);
    });

    it("stops a request when every row that asked has scrolled away, but not while one still wants it", async () => {
        net.hold = true;
        const one = new AbortController();
        const two = new AbortController();
        const p1 = systemIcon(file("zip"), 16, one.signal);
        const p2 = systemIcon(file("zip"), 16, two.signal);
        await Promise.resolve();
        await Promise.resolve();
        one.abort();
        expect(net.aborted).toEqual([]);
        two.abort();
        expect(net.aborted).toHaveLength(1);
        expect(await p1).toBeNull();
        expect(await p2).toBeNull();
    });

    it("does not ask again for a type that failed, and a busy or slow answer is tried again later", async () => {
        net.status.set(ICON("kind=file&ext=bad&px=16"), 500);
        expect(await systemIcon(file("bad"), 16)).toBeNull();
        expect(await systemIcon(file("bad"), 16)).toBeNull();
        expect(net.urls).toHaveLength(1);
        net.status.set(ICON("kind=file&ext=slow&px=16"), 504);
        await systemIcon(file("slow"), 16);
        await systemIcon(file("slow"), 16);
        expect(net.urls.filter((u) => u.includes("ext=slow"))).toHaveLength(2);
    });

    it("turns itself off for the session when srv has no system source (501), asking for nothing more", async () => {
        net.status.set(ICON("kind=file&ext=rs&px=16"), 501);
        expect(systemIconsAvailable()).toBe(true);
        expect(await systemIcon(file("rs"), 16)).toBeNull();
        expect(systemIconsAvailable()).toBe(false);
        expect(await systemIcon(file("md"), 16)).toBeNull();
        expect(net.urls).toHaveLength(1);
    });
});
