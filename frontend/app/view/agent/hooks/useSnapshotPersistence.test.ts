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

import { MAX_SAVED_HELD } from "./useHistoryPagination";
import { saveForReload } from "@/app/store/recycle-savers";
import { flushPaneSnapshots, unsavedPaneSnapshots, useSnapshotPersistence } from "./useSnapshotPersistence";
import type { AgentAtoms } from "../../../agent-model/state";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";

const FILTER = { showThinking: false, showSuccessfulTools: true, showFailedTools: true, showIncoming: true, showOutgoing: true };

function mountPane(blockId = "b1") {
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
        const [hidden, setHidden] = createSignal(false);
        const [foreign, setForeign] = createSignal(false);
        useSnapshotPersistence({
            blockId,
            definitionId: "def1",
            getAtoms: () => ({ documentStateAtom: docStateAtom }) as unknown as AgentAtoms,
            getDocument: doc,
            getDetailsOpen: detailsOpen,
            snapshotIsForeignBlock: foreign,
            hidden,
            log: () => {},
        });
        const setDocState = docStateAtom[1];
        return { dispose, setDocState, setDetailsOpen, setHidden, setForeign };
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

    it("rows held open are written when the pane goes hidden (ids only), not as they change", async () => {
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, heldOpenNodes: new Set(["h1", "h2"]) }));
        await vi.advanceTimersByTimeAsync(30_000);
        expect(rpc.writeState).not.toHaveBeenCalled(); // visible: a scroll-driven set isn't written
        pane.setHidden(true);
        await vi.advanceTimersByTimeAsync(30_000);
        expect(written()).toHaveLength(1);
        expect(written()[0].documentState.heldOpenNodeIds).toEqual(["h1", "h2"]);
        pane.dispose();
    });

    it("hiding a pane with nothing held open writes nothing", async () => {
        const pane = mountPane();
        pane.setHidden(true);
        await vi.advanceTimersByTimeAsync(30_000);
        expect(rpc.writeState).not.toHaveBeenCalled();
        pane.dispose();
    });

    it("keeps only the newest MAX_SAVED_HELD held ids", async () => {
        const pane = mountPane();
        const ids = Array.from({ length: MAX_SAVED_HELD + 15 }, (_, i) => `h${i}`);
        pane.setDocState((prev) => ({ ...prev, heldOpenNodes: new Set(ids) }));
        pane.setHidden(true);
        await vi.advanceTimersByTimeAsync(30_000);
        const saved = written()[0].documentState.heldOpenNodeIds as string[];
        expect(saved).toHaveLength(MAX_SAVED_HELD);
        expect(saved.at(-1)).toBe(ids.at(-1));
        expect(saved[0]).toBe(ids[15]);
        pane.dispose();
    });

    it("the page going away writes the held rows at once", async () => {
        const win = new EventTarget();
        vi.stubGlobal("window", win);
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, heldOpenNodes: new Set(["h1"]) }));
        win.dispatchEvent(new Event("pagehide"));
        await vi.advanceTimersByTimeAsync(0);
        expect(written()).toHaveLength(1);
        expect(written()[0].documentState.heldOpenNodeIds).toEqual(["h1"]);
        pane.dispose();
        vi.unstubAllGlobals();
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

    it("a failed write makes the flush say false, and the change is written again", async () => {
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        rpc.writeState.mockRejectedValueOnce(new Error("timeout"));
        await expect(flushPaneSnapshots()).resolves.toBe(false);
        // Still dirty: the next flush writes it, and then it's saved.
        await expect(flushPaneSnapshots()).resolves.toBe(true);
        expect(rpc.writeState).toHaveBeenCalledTimes(2);
        pane.dispose();
    });

    it("an overlay change made while the flush's write is in flight is written before the flush resolves", async () => {
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        let release: () => void = () => {};
        rpc.writeState.mockImplementationOnce(() => new Promise<void>((r) => (release = r)));
        const flushed = flushPaneSnapshots();
        await vi.advanceTimersByTimeAsync(0);
        // The user pins another row while the first write waits.
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2", "n3"]) }));
        release();
        await expect(flushed).resolves.toBe(true);
        expect(written().at(-1)!.documentState.pinnedNodeIds).toEqual(["n2", "n3"]);
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

describe("useSnapshotPersistence: what blocks a recycle is named, and a pane with nothing of its own to save does not", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        rpc.lineCount.mockReset().mockResolvedValue({ count: 42 });
        rpc.writeState.mockReset().mockResolvedValue(undefined);
    });
    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
    });

    it("a pane that failed a write and then became a continuation pane no longer blocks the recycle", async () => {
        // A continuation pane shows another block's history and never writes, so a failure
        // from before it became one must not stay as "not saved" for good.
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        rpc.writeState.mockRejectedValueOnce(new Error("timeout"));
        await expect(flushPaneSnapshots()).resolves.toBe(false);
        pane.setForeign(true);
        await expect(flushPaneSnapshots()).resolves.toBe(true);
        pane.dispose();
    });

    it("names the pane and the error when a write fails", async () => {
        const pane = mountPane("blockabcdef1234");
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        rpc.writeState.mockRejectedValue(new Error("FORBIDDEN: not your transcript"));
        const problems = await unsavedPaneSnapshots();
        expect(problems).toHaveLength(1);
        expect(problems[0]).toContain("blockabc");
        expect(problems[0]).toContain("write failed: FORBIDDEN: not your transcript");
        pane.dispose();
    });

    it("names a pane whose overlay keeps changing as such, not as a failed write", async () => {
        const pane = mountPane("blockabcdef1234");
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n0"]) }));
        let n = 0;
        // Each write is followed by another change, as if the user kept pinning rows.
        rpc.writeState.mockImplementation(async () => {
            n++;
            pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set([...prev.pinnedNodes, `x${n}`]) }));
        });
        const problems = await unsavedPaneSnapshots();
        expect(problems).toHaveLength(1);
        expect(problems[0]).toContain("blockabc");
        expect(problems[0]).toContain("kept changing");
        expect(problems[0]).not.toContain("write failed");
        pane.dispose();
    });

    it("lists only the panes that are not saved", async () => {
        const good = mountPane("goodgoodgood");
        const bad = mountPane("badbadbadbad");
        good.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["a"]) }));
        bad.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["b"]) }));
        rpc.writeState.mockImplementation(async (_c: unknown, args: { content: string }) => {
            if (JSON.parse(args.content).documentState.pinnedNodeIds[0] === "b") throw new Error("disk");
        });
        const problems = await unsavedPaneSnapshots();
        expect(problems).toHaveLength(1);
        expect(problems[0]).toContain("badbadba");
        good.dispose();
        bad.dispose();
    });

    it("the recycle's save keeps the reason short for the user and writes the cause to the console", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const pane = mountPane("blockabcdef1234");
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        rpc.writeState.mockRejectedValue(new Error("timeout"));
        const reasons = await saveForReload();
        expect(reasons).toContain("a pane's collapsed rows, pins or filter couldn't be saved");
        expect(reasons.join(" ")).not.toContain("timeout");
        expect(warn).toHaveBeenCalledTimes(1);
        expect(String(warn.mock.calls[0][0])).toContain("[recycle]");
        expect(String(warn.mock.calls[0][0])).toContain("write failed: timeout");
        pane.dispose();
    });

    it("says nothing when every pane is saved", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const pane = mountPane();
        pane.setDocState((prev) => ({ ...prev, pinnedNodes: new Set(["n2"]) }));
        expect((await saveForReload()).filter((r) => r.includes("collapsed rows"))).toEqual([]);
        expect(warn).not.toHaveBeenCalled();
        pane.dispose();
    });
});
