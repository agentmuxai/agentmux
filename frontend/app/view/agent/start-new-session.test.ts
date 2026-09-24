// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { archiveThenReturnToPicker, newSessionArchives } from "./start-new-session";

describe("newSessionArchives", () => {
    it("archives the pane's own transcript and then the agent's global zone", async () => {
        const calls: string[] = [];
        const archives = newSessionArchives(
            "block-1",
            "def-1",
            async (id) => void calls.push(`block:${id}`),
            async (id) => void calls.push(`agent:${id}`)
        );
        for (const a of archives) await a();
        expect(calls).toEqual(["block:block-1", "agent:def-1"]);
    });

    it.each([undefined, null, "", 42])("still archives the pane when there is no agent id (%s)", async (id) => {
        const archiveBlock = vi.fn(async () => {});
        const archiveAgent = vi.fn(async () => {});
        const archives = newSessionArchives("block-1", id, archiveBlock, archiveAgent);
        for (const a of archives) await a();
        expect(archiveBlock).toHaveBeenCalledWith("block-1");
        expect(archiveAgent).not.toHaveBeenCalled();
    });
});

describe("archiveThenReturnToPicker", () => {
    it("runs every archive before returning to the picker", async () => {
        const calls: string[] = [];
        await archiveThenReturnToPicker(
            [async () => void calls.push("a"), async () => void calls.push("b")],
            async () => void calls.push("picker"),
            () => calls.push("error")
        );
        expect(calls).toEqual(["a", "b", "picker"]);
    });

    it("keeps going after a failed archive and still returns to the picker", async () => {
        const calls: string[] = [];
        const onError = vi.fn();
        await archiveThenReturnToPicker(
            [
                async () => {
                    throw new Error("rpc down");
                },
                async () => void calls.push("b"),
            ],
            async () => void calls.push("picker"),
            onError
        );
        expect(onError).toHaveBeenCalledOnce();
        expect(calls).toEqual(["b", "picker"]);
    });
});
