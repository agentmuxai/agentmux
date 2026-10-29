// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * After srv dropped events for this window, `resyncMuxObjects` re-fetches the objects in use.
 * Before it, a dropped `waveobj:update` left the cached object at its old version until
 * something else happened to touch it: the "parts of the UI broken" on a saturated machine.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn() }));

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { MuxObjUpdate: "waveobj:update" } }));
vi.mock("@/util/endpoints", () => ({
    getWebServerEndpoint: () => "http://127.0.0.1:1",
    isWebServerEndpointSet: () => true,
}));
vi.mock("@/util/fetchutil", () => ({ fetch: (...a: any[]) => h.fetch(...a) }));
vi.mock("./services", () => ({ ObjectService: { UpdateObject: async () => {} } }));
vi.mock("./app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));

const block = (oid: string, version: number, title = "") => ({ otype: "block", oid, version, meta: { title } }) as any;

/** srv's answers by oref: an object, or an error string. */
function serve(db: Record<string, any>) {
    h.fetch.mockImplementation((_url: string, init: { body: string }) => {
        const oref = JSON.parse(init.body).args[0];
        const v = db[oref];
        const body = typeof v === "string" ? { error: v } : { data: v };
        return Promise.resolve({ ok: true, json: () => Promise.resolve(body) });
    });
}

describe("resyncMuxObjects", () => {
    let MOS: typeof import("./mos");

    beforeEach(async () => {
        vi.resetModules();
        h.fetch.mockReset();
        MOS = await import("./mos");
        // The window holds three blocks, all fetched at version 1.
        serve({ "block:b1": block("b1", 1), "block:b2": block("b2", 1), "block:b3": block("b3", 1) });
        await Promise.all(["b1", "b2", "b3"].map((id) => MOS.loadAndPinMuxObject(`block:${id}`)));
    });

    it("brings back an object whose update was dropped (before: it stays at the old version)", async () => {
        serve({ "block:b1": block("b1", 2, "new"), "block:b2": block("b2", 1), "block:b3": block("b3", 1) });
        // Before the fix nothing re-fetches: the window shows version 1 for as long as nothing touches it.
        expect(MOS.getObjectValue<any>("block:b1").version).toBe(1);

        const changed = await MOS.resyncMuxObjects();

        expect(changed).toBe(1);
        expect(MOS.getObjectValue<any>("block:b1")).toMatchObject({ version: 2, meta: { title: "new" } });
    });

    it("does not overwrite an update that arrived while the fetch was in flight", async () => {
        serve({ "block:b1": block("b1", 2), "block:b2": block("b2", 1), "block:b3": block("b3", 1) });
        const p = MOS.resyncMuxObjects();
        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "b1", obj: block("b1", 3, "pushed") } as any);
        await p;
        expect(MOS.getObjectValue<any>("block:b1")).toMatchObject({ version: 3, meta: { title: "pushed" } });
    });

    it("clears an object srv no longer has (a dropped delete)", async () => {
        serve({ "block:b1": block("b1", 1), "block:b2": "call object.GetObject error: not found: block:b2", "block:b3": block("b3", 1) });
        await MOS.resyncMuxObjects();
        expect(MOS.getObjectValue<any>("block:b2")).toBeNull();
        expect(MOS.getObjectValue<any>("block:b1")).not.toBeNull();
    });

    it("keeps the cached object when a fetch fails for another reason", async () => {
        h.fetch.mockImplementation(() => Promise.resolve({ ok: false, status: 500, statusText: "boom" }));
        const changed = await MOS.resyncMuxObjects();
        expect(changed).toBe(0);
        expect(MOS.getObjectValue<any>("block:b1")).toMatchObject({ version: 1 });
    });

    it("fetches a few at a time, not all at once", async () => {
        let inFlight = 0;
        let peak = 0;
        h.fetch.mockImplementation(() => {
            inFlight++;
            peak = Math.max(peak, inFlight);
            return new Promise((res) =>
                setTimeout(() => {
                    inFlight--;
                    res({ ok: true, json: () => Promise.resolve({ data: block("b1", 1) }) });
                }, 5),
            );
        });
        for (let i = 4; i < 14; i++) await MOS.loadAndPinMuxObject(`block:x${i}`).catch(() => {});
        peak = 0;
        await MOS.resyncMuxObjects(2);
        expect(peak).toBeLessThanOrEqual(2);
    });
});
