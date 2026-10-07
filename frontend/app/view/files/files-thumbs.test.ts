// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The grid view's thumbnail maker: never more than four at once, abandoned
 * when every tile that asked has gone, and a failure not retried.
 * muxreview on #4225.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const net = vi.hoisted(() => ({
    active: 0,
    peak: 0,
    started: [] as string[],
    aborted: [] as string[],
    release: [] as (() => void)[],
    fail: new Set<string>(),
}));

vi.mock("@/app/element/local-media", async (orig) => ({
    ...(await orig<typeof import("@/app/element/local-media")>()),
    fetchMediaBlob: (path: string, opts: { signal?: AbortSignal } = {}) =>
        new Promise<Blob>((resolve, reject) => {
            net.started.push(path);
            net.active++;
            net.peak = Math.max(net.peak, net.active);
            const done = () => {
                net.active--;
            };
            opts.signal?.addEventListener("abort", () => {
                net.aborted.push(path);
                done();
                reject(new DOMException("aborted", "AbortError"));
            });
            net.release.push(() => {
                done();
                if (net.fail.has(path)) reject(new Error("404"));
                else resolve(new Blob(["png"], { type: "image/png" }));
            });
        }),
}));

import { resetThumbnailsForTests, thumbnail } from "./files-thumbs";

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    resetThumbnailsForTests();
    Object.assign(net, { active: 0, peak: 0, started: [], aborted: [], release: [], fail: new Set() });
    (URL as unknown as { createObjectURL: unknown }).createObjectURL = () => "blob:t";
    (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = () => {};
});
afterEach(resetThumbnailsForTests);

describe("thumbnails", () => {
    it("never decodes more than four at once, even as slots change hands", async () => {
        const jobs = Array.from({ length: 10 }, (_, i) => thumbnail(`/p/${i}.png`, 1));
        await flush();
        expect(net.active).toBe(4);
        // Finish one: the next waiter takes its slot; a new caller can't jump in.
        net.release.shift()!();
        void thumbnail("/p/late.png", 1);
        await flush();
        expect(net.peak).toBe(4);
        while (net.release.length) {
            net.release.shift()!();
            await flush();
        }
        await Promise.all(jobs);
        expect(net.peak).toBe(4);
    });

    it("abandons a fetch once every tile that asked for it has gone", async () => {
        const a = new AbortController();
        const b = new AbortController();
        const p1 = thumbnail("/p/x.png", 1, a.signal);
        const p2 = thumbnail("/p/x.png", 1, b.signal);
        await flush();
        expect(net.started).toEqual(["/p/x.png"]);
        a.abort();
        expect(net.aborted).toEqual([]);
        b.abort();
        expect(net.aborted).toEqual(["/p/x.png"]);
        expect(await p1).toBeNull();
        expect(await p2).toBeNull();
    });

    it("never starts a queued fetch whose tiles have gone", async () => {
        for (let i = 0; i < 4; i++) void thumbnail(`/p/${i}.png`, 1);
        const gone = new AbortController();
        const queued = thumbnail("/p/queued.png", 1, gone.signal);
        await flush();
        gone.abort();
        net.release.shift()!();
        await flush();
        expect(await queued).toBeNull();
        expect(net.started).not.toContain("/p/queued.png");
    });

    it("doesn't retry an image that failed", async () => {
        net.fail.add("/p/bad.png");
        const first = thumbnail("/p/bad.png", 1);
        await flush();
        net.release.shift()!();
        expect(await first).toBeNull();
        expect(await thumbnail("/p/bad.png", 1)).toBeNull();
        expect(net.started.filter((p) => p === "/p/bad.png")).toHaveLength(1);
    });
});
