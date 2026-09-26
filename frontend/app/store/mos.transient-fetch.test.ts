// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * #3868: one failed GetObject used to poison the object cache. The catch left
 * the entry at `{ value: null }` with no pending fetch, so the next
 * `loadAndPinMuxObject` returned that null WITHOUT fetching, and `initMux`
 * aborted with `startup: client "<id>" did not load` for an object srv had
 * all along. On the reporting machine the failure was a Windows loopback
 * connect refusal (WSAENOBUFS), surfacing as `TypeError: Failed to fetch`.
 *
 * Pinned here: a transient failure is retried; when it still fails the entry
 * is re-fetched on the next explicit load (into the SAME entry, so existing
 * subscribers see the value); a definitive "not found" stays null; and no
 * request goes out before the backend endpoint is known.
 */

import { createEffect, createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    fetch: vi.fn(),
    endpointSet: true,
}));

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { MuxObjUpdate: "waveobj:update" } }));
vi.mock("@/util/endpoints", () => ({
    getWebServerEndpoint: () => (h.endpointSet ? "http://127.0.0.1:1" : "http://null"),
    isWebServerEndpointSet: () => h.endpointSet,
}));
vi.mock("@/util/fetchutil", () => ({ fetch: (...a: any[]) => h.fetch(...a) }));
vi.mock("./services", () => ({ ObjectService: { UpdateObject: async () => {} } }));
vi.mock("./app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));

const client = { otype: "client", oid: "c1", version: 1, windowids: ["w1"] } as any;

function respond(data: unknown, error?: string) {
    return Promise.resolve({ ok: true, json: () => Promise.resolve({ data, error }) });
}
const refused = () => Promise.reject(new TypeError("Failed to fetch"));

describe("mos — a transient GetObject failure does not poison the cache", () => {
    beforeEach(() => {
        vi.resetModules();
        vi.useRealTimers();
        h.fetch.mockReset();
        h.endpointSet = true;
    });

    it("retries a refused GetObject and resolves the object", async () => {
        vi.useFakeTimers();
        h.fetch.mockImplementationOnce(refused).mockImplementationOnce(() => respond(client));
        const MOS = await import("./mos");

        const p = MOS.loadAndPinMuxObject("client:c1");
        await vi.advanceTimersByTimeAsync(2000);

        await expect(p).resolves.toEqual(client);
        expect(h.fetch).toHaveBeenCalledTimes(2);
    });

    it("re-fetches on the next load after retries are exhausted, instead of returning the cached null", async () => {
        vi.useFakeTimers();
        h.fetch.mockImplementation(refused);
        const MOS = await import("./mos");

        // A memo reads the object first (window-identity's `client` memo), and
        // its fetch fails every attempt.
        MOS.getObjectValue("client:c1");
        await vi.advanceTimersByTimeAsync(2000);
        const failedAttempts = h.fetch.mock.calls.length;
        expect(failedAttempts).toBeGreaterThan(1);

        // The network recovers; initMux's explicit load must fetch again.
        h.fetch.mockImplementation(() => respond(client));
        const p = MOS.loadAndPinMuxObject("client:c1");
        await vi.advanceTimersByTimeAsync(2000);

        await expect(p).resolves.toEqual(client);
        expect(h.fetch.mock.calls.length).toBe(failedAttempts + 1);
    });

    it("delivers the re-fetched value to a subscriber of the original entry", async () => {
        vi.useFakeTimers();
        h.fetch.mockImplementation(refused);
        const MOS = await import("./mos");

        const seen: any[] = [];
        const dispose = createRoot((d) => {
            const atom = MOS.getMuxObjectAtom("client:c1");
            createEffect(() => seen.push(atom()));
            return d;
        });
        await vi.advanceTimersByTimeAsync(2000);

        h.fetch.mockImplementation(() => respond(client));
        await (async () => {
            const p = MOS.loadAndPinMuxObject("client:c1");
            await vi.advanceTimersByTimeAsync(2000);
            await p;
        })();

        expect(seen.at(-1)).toEqual(client);
        dispose();
    });

    it("rejects a load that still cannot reach srv, rather than resolving null", async () => {
        vi.useFakeTimers();
        h.fetch.mockImplementation(refused);
        const MOS = await import("./mos");

        const p = MOS.loadAndPinMuxObject("client:c1");
        const settled = p.then(
            (v) => ({ ok: true, v }),
            (e) => ({ ok: false, e })
        );
        await vi.advanceTimersByTimeAsync(2000);

        const r: any = await settled;
        expect(r.ok).toBe(false);
        expect(String(r.e)).toContain("Failed to fetch");
    });

    it("keeps a definitive 'not found' as null, without re-fetching", async () => {
        h.fetch.mockImplementation(() => respond(null, "not found: tab:gone"));
        const MOS = await import("./mos");

        await expect(MOS.loadAndPinMuxObject("tab:gone")).rejects.toThrow("not found");
        await expect(MOS.loadAndPinMuxObject("tab:gone")).resolves.toBeNull();
        expect(h.fetch).toHaveBeenCalledTimes(1);
    });

    it("sends nothing while the backend endpoint is unknown, and fetches once it is", async () => {
        h.endpointSet = false;
        h.fetch.mockImplementation(() => respond(client));
        const MOS = await import("./mos");

        MOS.getObjectValue("client:c1");
        await Promise.resolve();
        await new Promise((r) => setTimeout(r, 0));
        expect(h.fetch).not.toHaveBeenCalled();

        h.endpointSet = true;
        await expect(MOS.loadAndPinMuxObject("client:c1")).resolves.toEqual(client);
        expect(h.fetch).toHaveBeenCalledTimes(1);
        expect(String(h.fetch.mock.calls[0][0])).not.toContain("http://null");
    });
});
