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
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}` },
    pushNotification: (n: unknown) => hub.toast(n),
}));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));

import { AttachmentDraft, pastedFileName } from "./attachment-draft";

const info = (id: string, bytes = 10): AttachmentInfo => ({
    id,
    name: "",
    mime: "image/png",
    bytes,
    width: 10,
    height: 10,
    send_mime: "image/png",
    send_bytes: bytes,
    send_width: 10,
    send_height: 10,
    first_frame_only: false,
    kind: "image",
    text_bytes: 0,
    macros: false,
});

const accepted = (batchId: string, names: string[]) => ({
    batch_id: batchId,
    accepted: names.map((name, index) => ({ index, path: `/p/${name}`, name, bytes: 10 })),
    rejected: [],
    non_images: ["/p/notes.txt"],
    total_bytes: 10 * names.length,
    max_files: 128,
    max_total_bytes: 1024,
});

describe("AttachmentDraft", () => {
    beforeEach(() => {
        hub.ingest.mockReset();
        hub.cancel.mockReset().mockResolvedValue(undefined);
        hub.toast.mockReset();
    });

    it("keeps events that arrive before the ingest reply", async () => {
        const d = new AttachmentDraft("b1");
        hub.ingest.mockImplementation(async (_c: unknown, req: { batch_id: string }) => {
            // The backend finished before its reply reached us.
            d.onReady({ batch_id: req.batch_id, index: 0, info: info("a".repeat(64)) });
            d.onBatchDone({ batch_id: req.batch_id });
            return accepted(req.batch_id, ["a.png"]);
        });
        const nonImages = await d.ingestPaths(["/p/a.png", "/p/notes.txt"]);
        expect(nonImages).toEqual(["/p/notes.txt"]);
        expect(d.items().map((i) => i.status)).toEqual(["ready"]);
        expect(d.refs()).toEqual([{ id: "a".repeat(64), name: "a.png" }]);
    });

    it("tracks progress and failures by batch and index", async () => {
        const d = new AttachmentDraft("b2");
        let batch = "";
        hub.ingest.mockImplementation(async (_c: unknown, req: { batch_id: string }) => {
            batch = req.batch_id;
            return accepted(batch, ["a.png", "b.png"]);
        });
        await d.ingestPaths(["/p/a.png", "/p/b.png"]);
        d.onProgress({ batch_id: batch, index: 0, stage: "copying", done_bytes: 5, total_bytes: 10 });
        expect(d.progress()).toEqual({ done: 5, total: 20, count: 2 });
        d.onFailed({ batch_id: batch, index: 1, code: "decode", error: "Couldn't decode" });
        d.onReady({ batch_id: batch, index: 0, info: info("c".repeat(64)) });
        expect(d.items().map((i) => [i.name, i.status, i.error])).toEqual([
            ["a.png", "ready", undefined],
            ["b.png", "error", "Couldn't decode"],
        ]);
        // A failed image is never sent.
        expect(d.refs().map((r) => r.name)).toEqual(["a.png"]);
    });

    it("drops a duplicate and flashes the tile already there", async () => {
        const d = new AttachmentDraft("b3");
        let batch = "";
        hub.ingest.mockImplementation(async (_c: unknown, req: { batch_id: string }) => {
            batch = req.batch_id;
            return accepted(batch, ["a.png", "a-copy.png"]);
        });
        await d.ingestPaths(["/p/a.png", "/p/a-copy.png"]);
        const same = info("d".repeat(64));
        d.onReady({ batch_id: batch, index: 0, info: same });
        d.onReady({ batch_id: batch, index: 1, info: same });
        expect(d.items().map((i) => i.name)).toEqual(["a.png"]);
        expect(d.flashKey()).toBe(d.items()[0].key);
    });

    it("reorders, removes with undo, and cancels what's still processing", async () => {
        const d = new AttachmentDraft("b4");
        let batch = "";
        hub.ingest.mockImplementation(async (_c: unknown, req: { batch_id: string }) => {
            batch = req.batch_id;
            return accepted(batch, ["a.png", "b.png", "c.png"]);
        });
        await d.ingestPaths(["/p/a.png", "/p/b.png", "/p/c.png"]);
        d.onReady({ batch_id: batch, index: 0, info: info("1".repeat(64)) });
        d.onReady({ batch_id: batch, index: 1, info: info("2".repeat(64)) });
        const [a, b] = d.items();
        d.move(b.key, -1);
        expect(d.items().map((i) => i.name)).toEqual(["b.png", "a.png", "c.png"]);
        const removed = d.remove(a.key)!;
        expect(d.items().map((i) => i.name)).toEqual(["b.png", "c.png"]);
        d.restore([removed]);
        expect(d.items().map((i) => i.name)).toEqual(["b.png", "a.png", "c.png"]);
        d.cancel();
        expect(hub.cancel).toHaveBeenCalledWith({}, { batch_id: batch });
        expect(d.items().map((i) => i.name)).toEqual(["b.png", "a.png"]);
    });

    it("reports images refused by the limits", async () => {
        const d = new AttachmentDraft("b5");
        hub.ingest.mockImplementation(async (_c: unknown, req: { batch_id: string }) => ({
            ...accepted(req.batch_id, []),
            rejected: [{ path: "/p/z.png", name: "z.png", code: "too_many", reason: "A prompt can carry at most 128 images." }],
        }));
        await d.ingestPaths(["/p/z.png"]);
        expect(hub.toast).toHaveBeenCalledTimes(1);
        expect(hub.toast.mock.calls[0][0]).toMatchObject({ title: "z.png wasn't attached", type: "warning" });
    });
});

describe("pastedFileName", () => {
    it("names clipboard bitmaps by time and keeps real file names", () => {
        const at = new Date(2026, 8, 26, 14, 3, 12);
        expect(pastedFileName(new File([], "image.png", { type: "image/png" }), at)).toBe(
            "Pasted image 2026-09-26 14.03.12.png",
        );
        expect(pastedFileName(new File([], "diagram.jpg", { type: "image/jpeg" }), at)).toBe("diagram.jpg");
        expect(pastedFileName(new File([], "report.pdf", { type: "application/pdf" }), at)).toBe("report.pdf");
        expect(pastedFileName(new File([], "", { type: "" }), at)).toBe("Pasted file 2026-09-26 14.03.12");
    });
});
