// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";

const hub = vi.hoisted(() => ({
    ingest: vi.fn(),
    cancel: vi.fn(),
    info: vi.fn(),
    toast: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AttachmentsIngestCommand: (...a: unknown[]) => hub.ingest(...a),
        AttachmentsCancelCommand: (...a: unknown[]) => hub.cancel(...a),
        AttachmentsInfoCommand: (...a: unknown[]) => hub.info(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", async () => {
    const { isNoPayloadMuxEvent } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>("@/app/store/mps-events.generated");
    const mock = { muxEventSubscribe: () => () => {} };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); else if (isNoPayloadMuxEvent(eventType)) handler(null, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}` },
    pushNotification: (n: unknown) => hub.toast(n),
}));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));

import type { AttachmentRef } from "@/types/rpc/AttachmentRef";

const info = (id: string): AttachmentInfo => ({
    id,
    name: "",
    mime: "image/png",
    bytes: 10,
    width: 10,
    height: 10,
    send_mime: "image/png",
    send_bytes: 10,
    send_width: 10,
    send_height: 10,
    first_frame_only: false,
    kind: "image",
    text_bytes: 0,
    macros: false,
});

// A reload starts the module over: `vi.resetModules` plus a fresh import.
async function freshModule() {
    vi.resetModules();
    return import("./attachment-draft");
}

describe("attachment draft across a reload", () => {
    beforeEach(() => {
        sessionStorage.clear();
        hub.info.mockReset().mockImplementation(async (_c: unknown, req: { ids: string[] }) => ({
            items: req.ids.filter((id) => id !== "gone").map(info),
        }));
    });

    it("stores the ready attachments and brings them back, in order", async () => {
        const before = await freshModule();
        const refs: AttachmentRef[] = [
            { id: "a1", name: "one.png" },
            { id: "a2", name: "two.png" },
        ];
        await before.getAttachmentDraft("b1").restoreRefs(refs);
        expect(JSON.parse(sessionStorage.getItem("agentmux:attachment-draft:b1")!)).toEqual(refs);

        const after = await freshModule();
        const draft = after.getAttachmentDraft("b1");
        await vi.waitFor(() => expect(draft.readyItems().map((i) => i.name)).toEqual(["one.png", "two.png"]));
        expect(hub.info).toHaveBeenLastCalledWith(expect.anything(), { ids: ["a1", "a2"] });
    });

    it("a file the store no longer has comes back as an error tile, and isn't stored again", async () => {
        sessionStorage.setItem("agentmux:attachment-draft:b1", JSON.stringify([{ id: "gone", name: "old.png" }, { id: "a1", name: "one.png" }]));
        const { getAttachmentDraft } = await freshModule();
        const draft = getAttachmentDraft("b1");
        await vi.waitFor(() => expect(draft.items()).toHaveLength(2));
        expect(draft.items()[0]).toMatchObject({ name: "old.png", status: "error" });
        expect(JSON.parse(sessionStorage.getItem("agentmux:attachment-draft:b1")!)).toEqual([{ id: "a1", name: "one.png" }]);
    });

    it("a send clears what's stored", async () => {
        const { getAttachmentDraft } = await freshModule();
        const draft = getAttachmentDraft("b1");
        await draft.restoreRefs([{ id: "a1", name: "one.png" }]);
        draft.clearAfterSend();
        expect(sessionStorage.getItem("agentmux:attachment-draft:b1")).toBeNull();
    });

    it("nothing stored, nothing asked", async () => {
        const { getAttachmentDraft } = await freshModule();
        getAttachmentDraft("b1");
        expect(hub.info).not.toHaveBeenCalled();
    });
});
