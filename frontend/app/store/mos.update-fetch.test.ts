// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Task #47590, the pane open path. A block the backend has just created reaches the page inside
 * the reply to `CreateBlock`, as an update. The object store used to fetch it again on the spot
 * (a backend call per new pane, tab and layout), and whatever that fetch read replaced what was
 * there when it landed, newer or not.
 *
 * Pinned here: an update for an object the store has not heard of starts no fetch, and a fetch
 * that lands after a newer update does not replace it.
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
vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));

const block = (version: number, view = "browser") => ({ otype: "block", oid: "b1", version, meta: { view } }) as any;
const update = (obj: any) => ({ updatetype: "update", otype: obj.otype, oid: obj.oid, obj }) as any;

describe("mos: an object that arrives as an update", () => {
    beforeEach(() => {
        vi.resetModules();
        h.fetch.mockReset();
    });

    it("is not fetched again", async () => {
        const MOS = await import("./mos");

        MOS.updateMuxObject(update(block(1)));

        expect(h.fetch).not.toHaveBeenCalled();
        expect(MOS.getObjectValue("block:b1")).toEqual(block(1));
        // Reading it afterwards finds it: still no fetch.
        expect(h.fetch).not.toHaveBeenCalled();
    });

    it("keeps a newer update over a fetch that lands after it", async () => {
        let land!: (v: unknown) => void;
        h.fetch.mockImplementationOnce(
            () => new Promise((resolve) => (land = (data) => resolve({ ok: true, json: () => Promise.resolve({ data }) })))
        );
        const MOS = await import("./mos");

        // Something reads the block first, which fetches it; the fetch reads version 1.
        expect(MOS.getObjectValue("block:b1")).toBeNull();
        expect(h.fetch).toHaveBeenCalledTimes(1);
        // Two updates arrive while the fetch is on its way.
        MOS.updateMuxObject(update(block(2, "browser")));
        MOS.updateMuxObject(update(block(3, "browser")));
        land(block(1, "term"));
        await new Promise((r) => setTimeout(r, 0));

        expect(MOS.getObjectValue("block:b1")).toEqual(block(3, "browser"));
    });

    it("takes a fetch that is newer than what is there", async () => {
        let land!: (v: unknown) => void;
        h.fetch.mockImplementationOnce(
            () => new Promise((resolve) => (land = (data) => resolve({ ok: true, json: () => Promise.resolve({ data }) })))
        );
        const MOS = await import("./mos");

        expect(MOS.getObjectValue("block:b1")).toBeNull();
        MOS.updateMuxObject(update(block(2)));
        land(block(5));
        await new Promise((r) => setTimeout(r, 0));

        expect(MOS.getObjectValue("block:b1")).toEqual(block(5));
    });

    it("is still fetched by a later read when the update was not a usable object", async () => {
        h.fetch.mockImplementation(() => new Promise(() => {}));
        const MOS = await import("./mos");

        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "b1", obj: { otype: "block" } } as any);
        expect(h.fetch).not.toHaveBeenCalled();

        // The bad update left nothing behind, so the first read fetches as it always did.
        expect(MOS.getObjectValue("block:b1")).toBeNull();
        expect(h.fetch).toHaveBeenCalledTimes(1);
    });
});
