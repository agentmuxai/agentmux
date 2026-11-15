// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const { trash } = vi.hoisted(() => ({ trash: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { FsTrashCommand: trash } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { pendingPastedCopies, rememberPastedCopies, settlePastedCopies } from "./pasted-copies";

describe("pasted copies", () => {
    beforeEach(() => {
        trash.mockReset();
        trash.mockResolvedValue({ results: [] });
    });

    it("trashes a copy whose @mention was removed before sending", async () => {
        rememberPastedCopies("b1", ["C:\\work\\Pasted image.png"]);
        await settlePastedCopies("b1", "look at this");
        expect(trash).toHaveBeenCalledTimes(1);
        expect(trash.mock.calls[0][1]).toEqual({ paths: ["C:\\work\\Pasted image.png"] });
        expect(pendingPastedCopies("b1")).toEqual([]);
    });

    it("keeps a copy whose @mention is still in the message, and forgets it", async () => {
        rememberPastedCopies("b2", ["/work/Pasted image.png"]);
        await settlePastedCopies("b2", "see @Pasted image.png please");
        expect(trash).not.toHaveBeenCalled();
        expect(pendingPastedCopies("b2")).toEqual([]);
        // A later send never touches a copy an earlier message referred to.
        await settlePastedCopies("b2", "next");
        expect(trash).not.toHaveBeenCalled();
    });

    it("trashes only the copies that lost their mention", async () => {
        rememberPastedCopies("b3", ["/work/a.png", "/work/b.png", "/work/c (1).png"]);
        await settlePastedCopies("b3", "@a.png and @c (1).png");
        expect(trash.mock.calls[0][1]).toEqual({ paths: ["/work/b.png"] });
    });

    it("keeps each pane's copies apart", async () => {
        rememberPastedCopies("b4", ["/w4/x.png"]);
        rememberPastedCopies("b5", ["/w5/y.png"]);
        await settlePastedCopies("b4", "");
        expect(trash.mock.calls[0][1]).toEqual({ paths: ["/w4/x.png"] });
        expect(pendingPastedCopies("b5")).toEqual(["/w5/y.png"]);
        await settlePastedCopies("b5", "@y.png");
    });

    it("does nothing for a pane that pasted nothing, and survives a failed trash", async () => {
        await settlePastedCopies("none", "hello");
        expect(trash).not.toHaveBeenCalled();
        trash.mockRejectedValue(new Error("no trash"));
        rememberPastedCopies("b6", ["/w6/z.png"]);
        await expect(settlePastedCopies("b6", "")).resolves.toBeUndefined();
    });
});
