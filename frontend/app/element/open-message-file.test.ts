// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ PaneOpenCommand: vi.fn(async (..._args: unknown[]) => ({ block_id: "b1" })) }));
const notices = vi.hoisted(() => ({ push: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ atoms: { activeTabId: () => "tab-1" }, pushNotification: notices.push }));

import { openMessageFile } from "./open-message-file";

beforeEach(() => vi.clearAllMocks());

describe("openMessageFile", () => {
    it("asks srv to open the file as the message of the agent's pane names it, in the tab on screen, and sends nothing else", async () => {
        openMessageFile("/workspace/pic.png", "pane-1");
        await vi.waitFor(() => expect(rpc.PaneOpenCommand).toHaveBeenCalledTimes(1));
        expect(rpc.PaneOpenCommand).toHaveBeenCalledWith({}, { view: "editor", file: "/workspace/pic.png", message_of: "pane-1", tab_id: "tab-1" }, {});
        expect(notices.push).not.toHaveBeenCalled();
    });

    it("with no pane to name opens nothing, asks srv nothing and says so", async () => {
        openMessageFile("/workspace/pic.png", undefined);
        expect(rpc.PaneOpenCommand).not.toHaveBeenCalled();
        expect(notices.push).toHaveBeenCalledTimes(1);
        expect((notices.push.mock.calls[0][0] as { title: string }).title).toBe("Couldn't open the file");
    });

    it("tells the person why when srv refuses it", async () => {
        rpc.PaneOpenCommand.mockRejectedValueOnce(new Error("This file isn't in a folder the agent shares with its container"));
        openMessageFile("C:/Users/me/photo.png", "pane-1");
        await vi.waitFor(() => expect(notices.push).toHaveBeenCalledTimes(1));
        const notice = notices.push.mock.calls[0][0] as { title: string; message: string };
        expect(notice.title).toBe("Couldn't open the file");
        expect(notice.message).toContain("isn't in a folder the agent shares");
    });
});
