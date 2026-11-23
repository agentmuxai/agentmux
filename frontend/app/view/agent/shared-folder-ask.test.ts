// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the launch asks srv about a shared folder, and what the person's Go ahead sends. */

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { FolderOverlapCheckCommand: vi.fn(), FolderOverlapAcceptCommand: vi.fn(), AgentOpenCommand: vi.fn(), AgentStopCommand: vi.fn() },
}));
let runtimeRow: { block_id?: string; state: string } | undefined;
const openAgentInActiveTab = vi.fn();
vi.mock("@/app/store/agent-runtime-live", () => ({
    sharedAgentRuntime: () => ({ rowOf: () => runtimeRow }),
    openAgentInActiveTab: (...a: unknown[]) => openAgentInActiveTab(...a),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: vi.fn(async () => {}) }));

import { setBlockMeta } from "@/app/store/block-meta";
import { RpcApi } from "@/app/store/rpc-api";
import { declineAskedOpen, dropAskedOpen, goAheadWithSharedFolder, isSharedFolderRefusal, openIntoPane, sharedFolderOverlaps } from "./shared-folder-ask";

const pair = (container: string) => ({ host_id: "h-1", host_name: "Korp", container_id: container, container_name: container });

beforeEach(() => vi.clearAllMocks());

describe("sharedFolderOverlaps", () => {
    it("asks srv about the host agent and the folder it opens in", async () => {
        vi.mocked(RpcApi.FolderOverlapCheckCommand).mockResolvedValue({ overlaps: [pair("c-1")] } as any);
        expect(await sharedFolderOverlaps("h-1", "host", "/w/korp")).toEqual([pair("c-1")]);
        expect(RpcApi.FolderOverlapCheckCommand).toHaveBeenCalledWith(expect.anything(), { agent_id: "h-1", folder: "/w/korp" });
    });

    it("is null when it shares no folder", async () => {
        vi.mocked(RpcApi.FolderOverlapCheckCommand).mockResolvedValue({ overlaps: [] } as any);
        expect(await sharedFolderOverlaps("h-1", "host", "/w/korp")).toBeNull();
    });

    it("does not ask for an agent that runs in a container", async () => {
        expect(await sharedFolderOverlaps("c-1", "container", "/w/aria")).toBeNull();
        expect(RpcApi.FolderOverlapCheckCommand).not.toHaveBeenCalled();
    });

    it("fails the launch when srv can't be asked, rather than opening past the ask", async () => {
        vi.mocked(RpcApi.FolderOverlapCheckCommand).mockRejectedValue(new Error("down"));
        await expect(sharedFolderOverlaps("h-1", "host", "/w/korp")).rejects.toThrow("down");
    });
});

describe("goAheadWithSharedFolder", () => {
    it("keeps the choice for each pair, in order", async () => {
        vi.mocked(RpcApi.FolderOverlapAcceptCommand).mockResolvedValue({ chosen: true } as any);
        await goAheadWithSharedFolder([pair("c-1"), pair("c-2")]);
        expect(vi.mocked(RpcApi.FolderOverlapAcceptCommand).mock.calls.map((c) => c[1])).toEqual([
            { host_id: "h-1", container_id: "c-1" },
            { host_id: "h-1", container_id: "c-2" },
        ]);
    });

    it("stops at the first refusal", async () => {
        vi.mocked(RpcApi.FolderOverlapAcceptCommand).mockRejectedValue(new Error("only the person"));
        await expect(goAheadWithSharedFolder([pair("c-1"), pair("c-2")])).rejects.toThrow("only the person");
        expect(RpcApi.FolderOverlapAcceptCommand).toHaveBeenCalledTimes(1);
    });
});

describe("a pane that srv held an open for", () => {
    it("knows the refusal by its word, wherever the message starts", () => {
        expect(isSharedFolderRefusal("SHARED_FOLDER: Korp (host) and Aria (container) share a folder")).toBe(true);
        expect(isSharedFolderRefusal("agent.open: SHARED_FOLDER: Korp (host) and Aria (container) share a folder")).toBe(true);
        expect(isSharedFolderRefusal("AGENT_LIVE_ELSEWHERE: it runs in another AgentMux")).toBe(false);
    });

    it("opens the agent into itself once the person has gone ahead", async () => {
        vi.mocked(RpcApi.AgentOpenCommand).mockResolvedValue({ block_id: "pane-1" } as any);
        await openIntoPane("h-1", "pane-1", "Korp");
        expect(RpcApi.AgentOpenCommand).toHaveBeenCalledWith(expect.anything(), { agent_id: "h-1", into_block_id: "pane-1", focus: true });
        expect(openAgentInActiveTab).not.toHaveBeenCalled();
        expect(setBlockMeta).not.toHaveBeenCalled();
    });

    it("shows an agent that has a block of its own the usual way, and leaves this pane an ordinary picker", async () => {
        vi.mocked(RpcApi.AgentOpenCommand).mockResolvedValue({ block_id: "parked-1" } as any);
        openAgentInActiveTab.mockResolvedValue(undefined);
        await openIntoPane("h-1", "pane-1", "Korp");
        expect(setBlockMeta).toHaveBeenCalledWith("pane-1", { "layout:agent": null });
        expect(openAgentInActiveTab).toHaveBeenCalledWith({ agent_id: "h-1", name: "Korp" });
    });

    it("says why when the usual way refuses", async () => {
        vi.mocked(RpcApi.AgentOpenCommand).mockResolvedValue({ block_id: "parked-1" } as any);
        openAgentInActiveTab.mockResolvedValue("open in another AgentMux");
        await expect(openIntoPane("h-1", "pane-1", "Korp")).rejects.toThrow("open in another AgentMux");
    });

    it("stops being a pane that waits when the person does not open it", async () => {
        await dropAskedOpen("pane-1");
        expect(setBlockMeta).toHaveBeenCalledWith("pane-1", { "layout:agent": null });
    });

    it("stops an agent that already runs (restart resume started it) when the person says no", async () => {
        runtimeRow = { block_id: "parked-1", state: "idle" };
        await declineAskedOpen("pane-1", "h-1");
        expect(setBlockMeta).toHaveBeenCalledWith("pane-1", { "layout:agent": null });
        expect(RpcApi.AgentStopCommand).toHaveBeenCalledWith(expect.anything(), { blockid: "parked-1", force: true });
    });

    it("stops nothing when the agent has no process", async () => {
        runtimeRow = { block_id: "old-block", state: "stopped" };
        await declineAskedOpen("pane-1", "h-1");
        runtimeRow = undefined;
        await declineAskedOpen("pane-1", "h-1");
        expect(RpcApi.AgentStopCommand).not.toHaveBeenCalled();
    });
});
