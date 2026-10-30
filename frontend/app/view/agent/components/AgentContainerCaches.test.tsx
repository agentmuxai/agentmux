// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentContainerCaches: the Build cache section of the Container limits panel. Shows what a container agent's cache
 * volumes hold, says so when it is a lot, clears them after a confirmation, and shows nothing for an agent that keeps
 * no cache.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { GetContainerCachesCommand: vi.fn(), ClearContainerCachesCommand: vi.fn() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { AgentContainerCaches } from "./AgentContainerCaches";

const GB = 1024 ** 3;
const caches = (total: number, volumes = 2) => ({
    volumes: Array.from({ length: volumes }, (_, i) => ({ name: `agentmux-cache-x-${i}`, bytes: total / volumes })),
    total_bytes: total,
    warn_bytes: 20 * GB,
});

beforeEach(() => vi.clearAllMocks());
afterEach(() => cleanup());

const renderIt = () => render(() => <AgentContainerCaches agentId="uid-1" agentName="Builder" />);

describe("AgentContainerCaches", () => {
    it("shows nothing for an agent that keeps no cache", async () => {
        vi.mocked(RpcApi.GetContainerCachesCommand).mockResolvedValue(caches(0, 0) as any);
        renderIt();
        await waitFor(() => expect(RpcApi.GetContainerCachesCommand).toHaveBeenCalledWith(expect.anything(), { agent_id: "uid-1" }));
        expect(screen.queryByTestId("container-caches")).toBeNull();
    });

    it("stays hidden, with no error, when the caches can't be read (Docker not answering)", async () => {
        vi.mocked(RpcApi.GetContainerCachesCommand).mockRejectedValue(new Error("Docker is not available on this machine"));
        renderIt();
        await waitFor(() => expect(RpcApi.GetContainerCachesCommand).toHaveBeenCalled());
        await Promise.resolve();
        expect(screen.queryByTestId("container-caches")).toBeNull();
        expect(screen.queryByTestId("container-caches-error")).toBeNull();
    });

    it("says how much the volumes hold, and that is a lot only above the warning level", async () => {
        vi.mocked(RpcApi.GetContainerCachesCommand).mockResolvedValue(caches(3.2 * GB) as any);
        renderIt();
        expect((await screen.findByTestId("container-caches-size")).textContent).toMatch(/3\.2 GB in 2 volumes/);
        expect(screen.queryByTestId("container-caches-large")).toBeNull();
        cleanup();
        vi.mocked(RpcApi.GetContainerCachesCommand).mockResolvedValue(caches(21 * GB) as any);
        renderIt();
        expect(await screen.findByTestId("container-caches-large")).toBeTruthy();
    });

    it("clears only after a confirmation, then says what was freed", async () => {
        vi.mocked(RpcApi.GetContainerCachesCommand).mockResolvedValue(caches(2 * GB) as any);
        vi.mocked(RpcApi.ClearContainerCachesCommand).mockResolvedValue({ ...caches(0, 0), freed_bytes: 2 * GB } as any);
        renderIt();
        fireEvent.click(await screen.findByTestId("container-caches-clear"));
        expect(RpcApi.ClearContainerCachesCommand).not.toHaveBeenCalled();
        fireEvent.click(await screen.findByTestId("container-caches-confirm"));
        await waitFor(() => expect(RpcApi.ClearContainerCachesCommand).toHaveBeenCalledWith(expect.anything(), { agent_id: "uid-1" }));
        expect((await screen.findByTestId("container-caches-cleared")).textContent).toMatch(/2\.0 GB freed/);
        expect(screen.queryByTestId("container-caches-size")).toBeNull();
    });

    it("shows a refusal in words and keeps the volumes listed", async () => {
        vi.mocked(RpcApi.GetContainerCachesCommand).mockResolvedValue(caches(2 * GB) as any);
        vi.mocked(RpcApi.ClearContainerCachesCommand).mockRejectedValue(new Error("a turn is running; clear the caches when it has finished"));
        renderIt();
        fireEvent.click(await screen.findByTestId("container-caches-clear"));
        fireEvent.click(await screen.findByTestId("container-caches-confirm"));
        expect((await screen.findByTestId("container-caches-error")).textContent).toMatch(/a turn is running/);
        expect(screen.getByTestId("container-caches-size")).toBeTruthy();
    });
});
