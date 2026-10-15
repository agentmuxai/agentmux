// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    lineCount: vi.fn(),
    writeState: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockfileLineCountCommand: (...a: unknown[]) => rpc.lineCount(...a),
        AgentSessionWriteStateCommand: (...a: unknown[]) => rpc.writeState(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { flushPaneSnapshots, useSnapshotPersistence } from "./useSnapshotPersistence";
import type { AgentAtoms } from "../state";
import type { DocumentNode, DocumentState } from "../types";

const FILTER = { showThinking: false, showSuccessfulTools: true, showFailedTools: true, showIncoming: true, showOutgoing: true };

function mountPane() {
    return createRoot((dispose) => {
        const [doc] = createSignal<DocumentNode[]>([]);
        const docStateAtom = createSignal<DocumentState>({
            collapsedNodes: new Set(),
            pinnedNodes: new Set(),
            heldOpenNodes: new Set(),
            scrollPosition: 0,
            filter: FILTER,
        } as DocumentState);
        const [detailsOpen, setDetailsOpen] = createSignal(false);
        useSnapshotPersistence({
            blockId: "b1",
            definitionId: "def1",
            getAtoms: () => ({ documentStateAtom: docStateAtom }) as unknown as AgentAtoms,
            getDocument: doc,
            getDetailsOpen: detailsOpen,
            snapshotIsForeignBlock: () => false,
            log: () => {},
        });
        const setDocState = docStateAtom[1];
        return { dispose, setDocState, setDetailsOpen };
    });
}

const written = () => rpc.writeState.mock.calls.map((c) => JSON.parse((c[1] as { content: string }).content));

describe("useSnapshotPersistence: the overlay is saved, and can be flushed", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        rpc.lineCount.mockReset().mockResolvedValue({ count: 42 });
        rpc.writeState.mockReset().mockResolvedValue(undefined);
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it("collapsing a row alone is written by the 30 s save", async () => {
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, collapsedNodes: new Set(["n1"]) }));
        await vi.advanceTimersByTimeAsync(30_000);
        expect(written()).toHaveLength(1);
        expect(written()[0].documentState.collapsedNodeIds).toEqual(["n1"]);
        pane.dispose();
    });

    it("a held-open change alone (not saved) writes nothing", async () => {
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, heldOpenNodes: new Set(["n1"]) }));
        await vi.advanceTimersByTimeAsync(30_000);
        expect(rpc.writeState).not.toHaveBeenCalled();
        pane.dispose();
    });

    it("opening details is written too", async () => {
        const pane = mountPane();
        pane.setDetailsOpen(true);
        await vi.advanceTimersByTimeAsync(30_000);
        expect(written()[0].paneState.detailsOpen).toBe(true);
        pane.dispose();
    });

    it("flushPaneSnapshots writes a pending change at once and resolves after the write", async () => {
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        await flushPaneSnapshots();
        expect(written()).toHaveLength(1);
        expect(written()[0].documentState.pinnedNodeIds).toEqual(["n2"]);
        expect(written()[0].highWaterMark).toBe(42);
        // Nothing left for the interval.
        await vi.advanceTimersByTimeAsync(30_000);
        expect(rpc.writeState).toHaveBeenCalledTimes(1);
        pane.dispose();
    });

    it("with nothing changed a flush writes nothing; a closed pane is no longer flushed, and its close-time write still lands", async () => {
        const pane = mountPane();
        await flushPaneSnapshots();
        expect(rpc.writeState).not.toHaveBeenCalled();
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        pane.dispose(); // the close-time write (a)
        expect(rpc.writeState).not.toHaveBeenCalled(); // still waiting on the line count
        await flushPaneSnapshots();
        await vi.advanceTimersByTimeAsync(0);
        expect(rpc.writeState).toHaveBeenCalledTimes(1);
    });
});
