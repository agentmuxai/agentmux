// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const moveHere = vi.hoisted(() => vi.fn());
const closeHolderPane = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AgentMoveHereCommand: moveHere } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: vi.fn() }));
vi.mock("./refused-send", () => ({ refusedSendOf: () => undefined }));
vi.mock("./open-here", async (orig) => ({ ...(await orig<typeof import("./open-here")>()), closeHolderPane }));

import { requestOpenHere, takeOpenHere } from "./open-here";
import { openInOtherPaneActions } from "./open-in-other-pane";

const actions = () => {
    const log = vi.fn();
    return { log, ...openInOtherPaneActions({ blockId: "blk-me", log, send: vi.fn() }) };
};

beforeEach(() => {
    vi.clearAllMocks();
    closeHolderPane.mockResolvedValue(true);
});

describe("Move here", () => {
    it("leaves the pane that held the agent open when it was not Open here", async () => {
        moveHere.mockResolvedValue({ from_block: "blk-old" });
        await actions().onMoveHere();
        expect(closeHolderPane).not.toHaveBeenCalled();
    });

    it("closes the pane that held the agent after Open here, and says which", async () => {
        moveHere.mockResolvedValue({ from_block: "blk-old" });
        requestOpenHere("blk-me");
        expect(takeOpenHere("blk-me")).toBe(true);
        const a = actions();
        await a.onMoveHere();
        await vi.waitFor(() => expect(closeHolderPane).toHaveBeenCalledWith("blk-old"));
        await vi.waitFor(() => expect(a.log).toHaveBeenCalledWith("agent", "Open here — closed blk-old"));
    });

    it("says the pane stays when it is in another window", async () => {
        moveHere.mockResolvedValue({ from_block: "blk-far" });
        closeHolderPane.mockResolvedValue(false);
        requestOpenHere("blk-me");
        takeOpenHere("blk-me");
        const a = actions();
        await a.onMoveHere();
        await vi.waitFor(() => expect(a.log).toHaveBeenCalledWith("agent", "Open here — blk-far is in another window and stays open"));
    });

    it("closes nothing when nobody else held the agent", async () => {
        moveHere.mockResolvedValue({});
        requestOpenHere("blk-me");
        takeOpenHere("blk-me");
        await actions().onMoveHere();
        expect(closeHolderPane).not.toHaveBeenCalled();
    });

    it("closes nothing when the move fails", async () => {
        moveHere.mockRejectedValue(new Error("stuck"));
        requestOpenHere("blk-me");
        takeOpenHere("blk-me");
        await expect(actions().onMoveHere()).rejects.toThrow("stuck");
        expect(closeHolderPane).not.toHaveBeenCalled();
    });
});
