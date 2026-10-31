// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// The drawer's shell dies with the pane, even when the pane's block is already gone at cleanup.

import { createRoot, createSignal } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    deleteSubBlock: vi.fn(),
    collapse: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        DeleteSubBlockCommand: (...args: unknown[]) => hub.deleteSubBlock(...args),
        SetMetaCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ MOS: { makeORef: (t: string, id: string) => `${t}:${id}` } }));
vi.mock("../shell-exit-collapse", () => ({
    collapseDrawerOnShellExit: (...args: unknown[]) => hub.collapse(...args),
}));

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { useShellDrawerLifecycle } from "./useShellDrawerLifecycle";

const paneModel = { dispatchPane: vi.fn() } as unknown as AgentPaneModel;

beforeEach(() => {
    hub.deleteSubBlock.mockReset().mockResolvedValue(undefined);
    hub.collapse.mockReset().mockResolvedValue(undefined);
});

describe("useShellDrawerLifecycle", () => {
    it("deletes the shell's sub-block when the pane is disposed", async () => {
        const [shellSubBlockId] = createSignal<string | undefined>("sub-1");
        const dispose = createRoot((d) => {
            useShellDrawerLifecycle({ blockId: "b", shellSubBlockId, paneModel, clearTermWrite: () => {} });
            return d;
        });
        await Promise.resolve();
        dispose();
        expect(hub.deleteSubBlock).toHaveBeenCalledWith({}, { blockid: "sub-1" });
    });

    it("still deletes it when the block is already null at cleanup", async () => {
        const [shellSubBlockId, setBlock] = createSignal<string | undefined>("sub-1");
        const dispose = createRoot((d) => {
            useShellDrawerLifecycle({ blockId: "b", shellSubBlockId, paneModel, clearTermWrite: () => {} });
            return d;
        });
        await Promise.resolve();
        setBlock(undefined);
        await Promise.resolve();
        dispose();
        expect(hub.deleteSubBlock).toHaveBeenCalledWith({}, { blockid: "sub-1" });
    });

    it("deletes nothing when the pane never had a shell", () => {
        const [shellSubBlockId] = createSignal<string | undefined>(undefined);
        const dispose = createRoot((d) => {
            useShellDrawerLifecycle({ blockId: "b", shellSubBlockId, paneModel, clearTermWrite: () => {} });
            return d;
        });
        dispose();
        expect(hub.deleteSubBlock).not.toHaveBeenCalled();
    });

    it("hands the exited sub-block to the collapse once, and does not delete it again at cleanup", async () => {
        const [shellSubBlockId] = createSignal<string | undefined>("sub-1");
        let exited: () => void = () => {};
        const dispose = createRoot((d) => {
            exited = useShellDrawerLifecycle({ blockId: "b", shellSubBlockId, paneModel, clearTermWrite: () => {} }).handleShellExited;
            return d;
        });
        await Promise.resolve();
        exited();
        expect(hub.collapse).toHaveBeenCalledTimes(1);
        expect(hub.collapse.mock.calls[0][0]).toMatchObject({ parentBlockId: "b", exitedSubBlockId: "sub-1" });
        dispose();
        expect(hub.deleteSubBlock).not.toHaveBeenCalled();
    });

    it("collapses the drawer through the pane model as a system dispatch", async () => {
        const [shellSubBlockId] = createSignal<string | undefined>("sub-1");
        let exited: () => void = () => {};
        createRoot(() => {
            exited = useShellDrawerLifecycle({ blockId: "b", shellSubBlockId, paneModel, clearTermWrite: () => {} }).handleShellExited;
        });
        await Promise.resolve();
        exited();
        hub.collapse.mock.calls[0][0].collapseDrawer();
        expect(paneModel.dispatchPane).toHaveBeenCalledWith({ type: "DetailsCollapse" }, "system");
    });
});
