// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NativeMemoryHistoryCommand: vi.fn(),
        NativeMemoryDiffCommand: vi.fn(),
        NativeMemoryRevertCommand: vi.fn(),
        NativeMemoryReadFileCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { NativeMemoryHistoryModel, orderVersionsOldestFirst } from "./native-memory-history-model";

function meta(id: string): NativeMemoryVersionMeta {
    return {
        id,
        content_hash: "",
        parent_version_id: null,
        source: "human",
        source_detail: "{}",
        session_id: "",
        created_at: 0,
    };
}

describe("orderVersionsOldestFirst", () => {
    // versionsAtom() is newest-first: index 0 = newest, higher index = older.
    const versions = [meta("v3-newest"), meta("v2"), meta("v1-oldest")];

    it("returns [oldest, newest] regardless of click order — first click newer", () => {
        const [from, to] = orderVersionsOldestFirst("v3-newest", "v1-oldest", versions);
        expect(from).toBe("v1-oldest");
        expect(to).toBe("v3-newest");
    });

    it("returns [oldest, newest] regardless of click order — first click older", () => {
        // Regression for reagent P1: an earlier revision had this branch's
        // output backwards, putting the newer id in `from`.
        const [from, to] = orderVersionsOldestFirst("v1-oldest", "v3-newest", versions);
        expect(from).toBe("v1-oldest");
        expect(to).toBe("v3-newest");
    });

    it("handles two adjacent versions in either click order — v2 is older than v3-newest", () => {
        expect(orderVersionsOldestFirst("v2", "v3-newest", versions)).toEqual(["v2", "v3-newest"]);
        expect(orderVersionsOldestFirst("v3-newest", "v2", versions)).toEqual(["v2", "v3-newest"]);
    });

    it("falls back to input order when an id is not found", () => {
        expect(orderVersionsOldestFirst("unknown-a", "unknown-b", versions)).toEqual(["unknown-a", "unknown-b"]);
    });
});

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
    let resolve!: (v: T) => void;
    const promise = new Promise<T>((res) => {
        resolve = res;
    });
    return { promise, resolve };
}

// Regression for reagent P2 on PR #2678: computeDiff() had no request-id
// guard — selecting one version pair, then a different pair before the
// first NativeMemoryDiffCommand resolved, could let the stale response
// land after the newer one and overwrite diffTextAtom.
describe("NativeMemoryHistoryModel diff request staleness", () => {
    const versions = [meta("v3-newest"), meta("v2"), meta("v1-oldest")];
    let dispose: (() => void) | undefined;

    afterEach(() => {
        dispose?.();
        dispose = undefined;
        vi.clearAllMocks();
    });

    async function makeModel(): Promise<NativeMemoryHistoryModel> {
        vi.mocked(RpcApi.NativeMemoryHistoryCommand).mockResolvedValue({ versions });
        let model!: NativeMemoryHistoryModel;
        createRoot((d) => {
            dispose = d;
            model = new NativeMemoryHistoryModel("agent-1", "MEMORY.md");
        });
        // Let the constructor's fire-and-forget loadHistory() settle.
        await Promise.resolve();
        await Promise.resolve();
        return model;
    }

    it("discards a stale diff response that resolves after a newer selection's diff", async () => {
        const model = await makeModel();

        const first = deferred<NativeMemoryDiffResult>();
        const second = deferred<NativeMemoryDiffResult>();
        vi.mocked(RpcApi.NativeMemoryDiffCommand)
            .mockReturnValueOnce(first.promise)
            .mockReturnValueOnce(second.promise);

        // First pair selected — fires the first (slow) diff request.
        model.toggleDiffSelection("v1-oldest");
        model.toggleDiffSelection("v2");

        // Deselect one and pick a different pair before the first request
        // resolves — fires the second (fast) diff request.
        model.toggleDiffSelection("v2");
        model.toggleDiffSelection("v3-newest");

        // The second, newer request resolves first.
        second.resolve({ diff: "second diff" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.diffTextAtom()).toBe("second diff");

        // The first, now-stale request resolves after — must be discarded,
        // not overwrite the newer diff already shown.
        first.resolve({ diff: "first diff (stale)" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.diffTextAtom()).toBe("second diff");
    });

    it("discards a stale diff response after the selection is cleared entirely", async () => {
        const model = await makeModel();

        const pending = deferred<NativeMemoryDiffResult>();
        vi.mocked(RpcApi.NativeMemoryDiffCommand).mockReturnValueOnce(pending.promise);

        model.toggleDiffSelection("v1-oldest");
        model.toggleDiffSelection("v2");
        model.clearDiffSelection();

        pending.resolve({ diff: "stale diff" });
        await Promise.resolve();
        await Promise.resolve();
        expect(model.diffTextAtom()).toBeNull();
    });
});
