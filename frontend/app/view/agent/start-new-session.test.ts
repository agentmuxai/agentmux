// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { archiveThenReturnToPicker } from "./start-new-session";

describe("archiveThenReturnToPicker", () => {
    it("archives the agent's conversation before returning to the picker", async () => {
        const calls: string[] = [];
        await archiveThenReturnToPicker(
            "def-1",
            async (id) => void calls.push(`archive:${id}`),
            async () => void calls.push("picker"),
            () => calls.push("error")
        );
        expect(calls).toEqual(["archive:def-1", "picker"]);
    });

    it("still returns to the picker when the archive fails", async () => {
        const onError = vi.fn();
        const backToPicker = vi.fn(async () => {});
        await archiveThenReturnToPicker(
            "def-1",
            async () => {
                throw new Error("rpc down");
            },
            backToPicker,
            onError
        );
        expect(onError).toHaveBeenCalledOnce();
        expect(backToPicker).toHaveBeenCalledOnce();
    });

    it.each([undefined, null, "", 42])("skips the archive when the pane has no agent id (%s)", async (id) => {
        const archive = vi.fn(async () => {});
        const backToPicker = vi.fn(async () => {});
        await archiveThenReturnToPicker(id, archive, backToPicker, () => {});
        expect(archive).not.toHaveBeenCalled();
        expect(backToPicker).toHaveBeenCalledOnce();
    });
});
